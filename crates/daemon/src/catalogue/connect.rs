//! Connect, on its own (D79): the ConnectRemote, the cluster the dealer pushes, the device list, moving playback,
//! the Jam, our transport buttons for another device, an Engine's own commands over Connect, queue edits and the
//! resume after an update. It runs beside browsing, so a wait here (an Engine's device can take 20 s to show up, a
//! resume retries for 6.5 s) never holds up a search (#8).
//!
//! Login stays with the Catalogue: Connect asks through the session it keeps in the shared slot, and drops its remote
//! when that session ends or changes.

use super::*;

pub(super) struct Connect {
    engine: Identity,
    slot: SessionSlot,
    ev: mpsc::Sender<CatEvent>,
    /// browsing, which owns the login: told when the session ended so it logs in again
    browse: mpsc::WeakUnboundedSender<CatCmd>,
    remote: Option<ConnectRemote>,
    /// the session `remote` was started on: a new login needs a new remote (its dealer belongs to the old one)
    on: Option<Session>,
    /// the latest Connect cluster, kept fresh by the dealer
    cluster: std::sync::Arc<std::sync::Mutex<Option<librespot_protocol::connect::Cluster>>>,
    /// told whenever a new cluster arrives (the Spotify app as Engine waits on it for its device)
    cluster_changed: std::sync::Arc<tokio::sync::Notify>,
}

impl Connect {
    pub(super) fn new(engine: Identity, slot: SessionSlot, ev: mpsc::Sender<CatEvent>, browse: mpsc::WeakUnboundedSender<CatCmd>) -> Connect {
        Connect { engine, slot, ev, browse, remote: None, on: None, cluster: Default::default(), cluster_changed: Default::default() }
    }

    /// Run Connect's commands one after another until browsing hangs up.
    pub(super) async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<CatCmd>) {
        while let Some(cmd) = cmds.recv().await {
            match cmd {
                CatCmd::PlayFrom { context_uri, index, track_uri } => self.play_from(context_uri, index, track_uri).await,
                CatCmd::Queue(edit) => self.queue(edit).await,
                CatCmd::Devices => self.devices().await,
                CatCmd::Transfer(id) => self.transfer(id).await,
                CatCmd::StartJam => self.start_jam().await,
                CatCmd::EndJam => self.end_jam().await,
                CatCmd::Resume(r) => self.resume(r).await,
                CatCmd::Remote(cmd) => self.remote_command(cmd).await,
                CatCmd::Engine(cmd) => self.engine_command(cmd).await,
                other => tracing::warn!("Connect was handed a browsing command: {other:?}"),
            }
        }
    }

    async fn fail(&self, message: String) {
        if self.ev.send(CatEvent::Error(message.clone())).await.is_err() {
            tracing::warn!("no one heard: {message}");
        }
    }

    /// The Catalogue's current session. Logged out, or with a session Spotify ended (browsing is told, and logs in
    /// again), there's none, and the remote of an old one goes.
    fn session(&mut self) -> anyhow::Result<Session> {
        let current = self.slot.lock().unwrap().clone();
        match current {
            Some(s) if !s.is_invalid() => {
                if self.on.as_ref().is_some_and(|on| on.session_id() != s.session_id()) {
                    self.drop_remote();
                }
                Ok(s)
            }
            Some(_) => {
                self.drop_remote();
                if let Some(b) = self.browse.upgrade() {
                    let _ = b.send(CatCmd::Reconnect);
                }
                anyhow::bail!("the Spotify session ended; logging in again")
            }
            None => {
                self.drop_remote();
                anyhow::bail!("not logged in to Spotify")
            }
        }
    }

    fn drop_remote(&mut self) {
        self.remote = None;
        self.on = None;
    }

    /// ConnectRemote (started on first use) and Soloist's Connect id.
    async fn remote(&mut self) -> anyhow::Result<(Session, &mut ConnectRemote)> {
        let session = self.session()?;
        if self.remote.is_none() {
            let (remote, cluster, mut updates, mut ids) = ConnectRemote::start(session.clone()).await?;
            self.remote = Some(remote);
            *self.cluster.lock().expect("cluster lock") = Some(cluster.clone());
            emit_cluster(&self.ev, &cluster, &self.engine).await;
            // the dealer pushes every change; keep the latest and tell the Daemon. After a reconnect (a network
            // change) it has a new connection id, which must be registered again, or the pushes stop.
            let (latest, ev, engine, changed) = (self.cluster.clone(), self.ev.clone(), self.engine.clone(), self.cluster_changed.clone());
            let session = session.clone();
            // every change to the account's Jam (D64): read it again, the update itself is loose JSON
            let mut jam_updates = session.dealer().listen_for(crate::jam::UPDATE_TOPIC, |_| -> Result<(), librespot_core::Error> { Ok(()) })?;
            tokio::spawn(async move {
                loop {
                    let c = tokio::select! {
                        u = jam_updates.next() => match u {
                            Some(_) => {
                                let (names, engine_id) = jam_devices(&latest, &engine);
                                match crate::jam::current(&session, &names, engine_id.as_deref()).await {
                                    Ok(jam) => {
                                        let _ = ev.send(CatEvent::Jam(jam)).await;
                                    }
                                    Err(e) => tracing::debug!("Jam: couldn't read it after an update: {e}"),
                                }
                                continue;
                            }
                            None => return,
                        },
                        u = updates.next() => match u {
                            Some(Ok(u)) => match u.cluster.into_option() {
                                Some(c) => c,
                                None => continue,
                            },
                            Some(Err(_)) => continue,
                            None => return,
                        },
                        id = ids.next() => match id {
                            Some(Ok(id)) => match crate::connect::reconnected(&session, &id).await {
                                Ok(c) => {
                                    tracing::info!("Connect: registered again after a reconnect");
                                    c
                                }
                                Err(e) => {
                                    tracing::warn!("Connect: couldn't register again after a reconnect: {e}");
                                    continue;
                                }
                            },
                            Some(Err(_)) => continue,
                            None => return,
                        },
                    };
                    *latest.lock().expect("cluster lock") = Some(c.clone());
                    emit_cluster(&ev, &c, &engine).await;
                    changed.notify_waiters();
                }
            });
        }
        self.on = Some(session.clone());
        Ok((session, self.remote.as_mut().expect("just started")))
    }

    /// The remote and this computer's own device on Connect. Only what plays here needs it: controlling another
    /// device, moving playback and the device list don't (C6).
    async fn ours(&mut self) -> anyhow::Result<(Session, &mut ConnectRemote, String)> {
        let cluster = self.cluster.clone();
        let name = self.engine.device_name.clone();
        let (session, remote) = self.remote().await?;
        // the latest cluster shows the device under another id, or not at all: it re-registered (the app restarted,
        // or came back after a network change), so look it up again
        let name = &name;
        if let Some(cached) = remote.cached_id(name) {
            let latest = cluster.lock().expect("cluster lock").as_ref().map(|c| c.device.iter().find(|(_, d)| d.name == *name).map(|(id, _)| id.clone()));
            if latest.is_some_and(|now| now.as_deref() != Some(cached)) {
                remote.forget_ids();
            }
        }
        let to = remote.device_id(name).await?;
        Ok((session, remote, to))
    }

    async fn devices(&mut self) {
        if let Err(e) = self.remote().await {
            tracing::debug!("Connect devices: {e}");
            return;
        }
        let c = self.cluster.lock().expect("cluster lock").clone();
        if let Some(c) = c {
            emit_cluster(&self.ev, &c, &self.engine).await;
        }
        self.refresh_jam().await;
    }

    async fn transfer(&mut self, to: String) {
        let cluster = self.cluster.clone();
        let result = async {
            let (_, remote) = self.remote().await?;
            remote.transfer(&remote_cluster(&cluster)?, &to).await
        }
        .await;
        if let Err(e) = result {
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            self.fail(format!("Couldn't move playback: {e}")).await;
        }
    }

    /// Start a Jam hosted on the Engine's own device (D64; verified to work for Soloist's, 2026-10-30).
    async fn start_jam(&mut self) {
        let result = async {
            let (session, _, to) = self.ours().await?;
            let (names, _) = jam_devices(&self.cluster, &self.engine);
            crate::jam::start(&session, &to, &names).await
        }
        .await;
        match result {
            Ok(Some(jam)) => {
                tracing::info!("Jam started on {}", jam.host.as_deref().unwrap_or("an unlisted device"));
                let _ = self.ev.send(CatEvent::Jam(Some(jam))).await;
            }
            Ok(None) => self.fail("Spotify didn't start a Jam (Premium is needed to host one)".into()).await,
            Err(e) => self.fail(format!("Couldn't start a Jam: {e}")).await,
        }
    }

    /// End the Jam this account runs, for everyone; the music keeps playing.
    async fn end_jam(&mut self) {
        let result = async {
            let (session, _) = self.remote().await?;
            let (names, engine_id) = jam_devices(&self.cluster, &self.engine);
            match crate::jam::current(&session, &names, engine_id.as_deref()).await? {
                None => Ok(None),
                Some(jam) if !jam.owner => anyhow::bail!("only the one who started it can end it"),
                Some(jam) => crate::jam::end(&session, &jam.session_id).await.map(|()| Some(jam)),
            }
        }
        .await;
        match result {
            Ok(ended) => {
                if ended.is_some() {
                    tracing::info!("Jam ended");
                }
                let _ = self.ev.send(CatEvent::Jam(None)).await;
            }
            Err(e) => self.fail(format!("Couldn't end the Jam: {e}")).await,
        }
    }

    /// Read the account's Jam (on login and when the Devices list is asked for).
    async fn refresh_jam(&mut self) {
        let Ok(session) = self.session() else { return };
        let (names, engine_id) = jam_devices(&self.cluster, &self.engine);
        match crate::jam::current(&session, &names, engine_id.as_deref()).await {
            Ok(jam) => {
                let _ = self.ev.send(CatEvent::Jam(jam)).await;
            }
            Err(e) => tracing::debug!("Jam: couldn't read it: {e}"),
        }
    }

    /// After a Soloist update: the restarted device takes a moment to show up on Connect, so this tries a few times,
    /// then falls back to the Engine playing the track alone.
    async fn resume(&mut self, r: crate::engine::Resume) {
        for attempt in 0..4 {
            tokio::time::sleep(Duration::from_millis(if attempt == 0 { 500 } else { 2000 })).await;
            let result = async {
                let (_, remote, to) = self.ours().await?;
                remote.resume(&to, &r, "").await
            }
            .await;
            match result {
                Ok(()) => return,
                Err(e) => {
                    tracing::debug!("resume after the update (attempt {}): {e}", attempt + 1);
                    if let Some(rm) = self.remote.as_mut() {
                        rm.forget_ids();
                    }
                }
            }
        }
        let _ = self.ev.send(CatEvent::ToEngine(Command::Play { uri: Some(r.track_uri) })).await;
        let _ = self.ev.send(CatEvent::ToEngine(Command::Seek { position_ms: r.position_ms })).await;
        if r.paused {
            let _ = self.ev.send(CatEvent::ToEngine(Command::Pause)).await;
        }
    }

    /// Our transport buttons, for the device that's playing.
    async fn remote_command(&mut self, cmd: Command) {
        let active = self.cluster.lock().expect("cluster lock").as_ref().map(|c| c.active_device_id.clone()).filter(|a| !a.is_empty());
        let result = async {
            let to = active.ok_or_else(|| anyhow::anyhow!("no device is playing"))?;
            let (_, remote) = self.remote().await?;
            remote.control(&to, &cmd).await
        }
        .await;
        if let Err(e) = result {
            self.fail(format!("Couldn't control the other device: {e}")).await;
        }
    }

    /// An Engine played over Connect (the Spotify app, D51; librespot, D52): its commands go to its Connect device. Failures are reported, never handed
    /// back to the Engine (that's where they came from).
    async fn engine_command(&mut self, cmd: Command) {
        let cluster = self.cluster.clone();
        let result = async {
            let (session, remote, to) = self.engine_device().await?;
            let state = remote_cluster(&cluster).ok();
            let ours = state.as_ref().is_some_and(|c| c.active_device_id == to);
            let ps = state.as_ref().and_then(|c| c.player_state.as_ref());
            let playing = ours && ps.is_some_and(|p| p.is_playing && !p.is_paused);
            match cmd {
                Command::SetSoloistKey { .. } => anyhow::bail!("the Spotify app plays without a Soloist key"),
                Command::TogglePlay if playing => remote.control(&to, &Command::Pause).await,
                // resume what the account played last, on this computer
                Command::TogglePlay | Command::Play { uri: None } if !ours => match state.as_ref().filter(|_| ps.is_some_and(|p| p.track.is_some())) {
                    Some(c) => remote.transfer(c, &to).await,
                    None => anyhow::bail!("nothing to resume yet: play something from a list"),
                },
                Command::TogglePlay => remote.control(&to, &Command::Play { uri: None }).await,
                Command::AddToQueue { uri } => remote.add_to_queue(&to, &uri).await,
                Command::PlayFrom { context_uri, index, track_uri } => {
                    let context = if context_uri == library::LIKED { format!("spotify:user:{}:collection", session.username()) } else { context_uri };
                    remote.play_from(&to, &context, index, track_uri.as_deref()).await
                }
                other => remote.control(&to, &other).await,
            }
        }
        .await;
        if let Err(e) = result {
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            let player = self.engine.player;
            let mut player = player.strip_prefix("the ").unwrap_or(player).to_string();
            player[..1].make_ascii_uppercase();
            self.fail(format!("{player}: {e}")).await;
        }
    }

    /// ConnectRemote and the Spotify app's device. A device that isn't on Connect (the app just started, or came back
    /// after a network change) is waited for: each pushed cluster is looked at, for up to 20 s. No polling.
    async fn engine_device(&mut self) -> anyhow::Result<(Session, &mut ConnectRemote, String)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let name = self.engine.device_name.clone();
        loop {
            let changed = self.cluster_changed.clone();
            let next = changed.notified();
            match self.ours().await {
                Ok(_) => break,
                Err(e) if Instant::now() >= deadline || !e.to_string().contains("not found") => return Err(e),
                Err(_) => {
                    tracing::debug!("waiting for {name:?} on Connect");
                    if let Some(r) = self.remote.as_mut() {
                        r.forget_ids();
                    }
                    let _ = tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), next).await;
                }
            }
        }
        self.ours().await
    }

    /// Queue edits Soloist can't make (play next, remove, clear, move) go through Connect.
    async fn queue(&mut self, edit: QueueEdit) {
        let result = async {
            let (_, remote, to) = self.ours().await?;
            remote.edit_queue(&to, &edit).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("queue edit via Connect failed: {e}");
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            match edit {
                // Soloist can at least append it
                QueueEdit::PlayNext(uri) => {
                    let _ = self.ev.send(CatEvent::ToEngine(Command::AddToQueue { uri })).await;
                    self.fail("Couldn't put it first; added it to the end of the queue".into()).await;
                }
                _ => {
                    self.fail(format!("Couldn't change the queue: {e}")).await;
                    let _ = self.ev.send(CatEvent::Resync).await;
                }
            }
        }
    }

    /// Start a context at track N through ConnectRemote; if that fails, the Engine plays the track alone.
    async fn play_from(&mut self, context_uri: String, index: u32, track_uri: Option<String>) {
        let result = async {
            // a failure below drops the remote, so a stale device id is looked up again next time
            let (session, remote, to) = self.ours().await?;
            let context = if context_uri == library::LIKED { format!("spotify:user:{}:collection", session.username()) } else { context_uri.clone() };
            remote.play_from(&to, &context, index, track_uri.as_deref()).await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!("play from track {index} of {context_uri} via Connect failed: {e}");
            if let Some(r) = self.remote.as_mut() {
                r.forget_ids();
            }
            let _ = self.ev.send(CatEvent::ToEngine(Command::PlayFrom { context_uri, index, track_uri })).await;
        }
    }
}
