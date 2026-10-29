//! Dev tool: inspect and exercise Spotify's Jam (social-connect) API for the owner's live checks.
//!
//!     cargo run -p singularityd --example jam_probe                     # current Jam, read-only
//!     cargo run -p singularityd --example jam_probe -- devices
//!     cargo run -p singularityd --example jam_probe -- start --device "<device name>"
//!
//! The probe uses the saved Catalogue login, registers a *hidden* controller for the run and removes it
//! afterwards. It never prints access tokens and never writes anything to disk. `start`, `end`, `kick` and
//! `perm` ask for `y` first because they change other people's phones.
//!
//! Nothing here runs against Spotify by itself: it is built and unit-tested offline.

use std::fmt;
use std::io::{self, BufRead, Write};
use std::time::Duration;

use futures_util::StreamExt;
use librespot_core::dealer::protocol::{Message as DealerMessage, PayloadValue};
use librespot_core::http_client::HttpClientError;
use librespot_core::{cache::Cache, config::SessionConfig, session::Session};
use librespot_protocol::connect::{
    Capabilities, Cluster, Device, DeviceInfo, MemberType, PutStateReason, PutStateRequest,
};
use protobuf::Message as ProtobufMessage;
use qrcodegen::{QrCode, QrCodeEcc};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use singularityd::catalogue::Dirs;

const HIDDEN_NAME: &str = "Singularity jam probe";
const SESSION_CURRENT: &str = "/social-connect/v2/sessions/current";
const SESSION_UPDATE_TOPIC: &str = "social-connect/v2/session_update";
const INVITE_BASE: &str = "https://open.spotify.com/socialsession";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Post,
    Put,
    Delete,
}

impl Verb {
    fn method(self) -> reqwest::Method {
        match self {
            Verb::Post => reqwest::Method::POST,
            Verb::Put => reqwest::Method::PUT,
            Verb::Delete => reqwest::Method::DELETE,
        }
    }
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verb::Post => "POST",
            Verb::Put => "PUT",
            Verb::Delete => "DELETE",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    verb: Verb,
    path: String,
    body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProbeError {
    Http(u16),
    Other(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProbeError::Http(code) => write!(f, "HTTP {code}"),
            ProbeError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ProbeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Permission {
    QueueOnly,
    Volume,
}

#[derive(Debug)]
enum Cmd {
    Current,
    Devices,
    Start { device: String },
    End,
    Kick { member_id: String },
    Perm { permission: Permission, on: bool },
    Watch,
    Help(Option<String>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cmd = match parse_args(std::env::args().skip(1).collect()) {
        Ok(Cmd::Help(topic)) => {
            print!("{}", help_for(topic.as_deref()));
            return Ok(());
        }
        Ok(cmd) => cmd,
        Err(message) => {
            eprintln!("{message}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let session = match connect().await {
        Ok(session) => session,
        Err(error) => {
            eprintln!("cannot connect to Spotify: {error}");
            std::process::exit(1);
        }
    };

    let result = run(&session, cmd).await;
    let _ = session.spclient().delete_connect_state_request().await;
    session.dealer().close().await;
    result
}

async fn connect() -> anyhow::Result<Session> {
    let dirs = Dirs::from_env();
    let cache = Cache::new(Some(&dirs.credentials), None, None, None)?;
    let credentials = singularityd::catalogue::saved_credentials(&dirs)
        .ok_or_else(|| anyhow::anyhow!("no saved login: log in to Singularity first"))?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(credentials, true).await?;

    let mut connection = session
        .dealer()
        .listen_for("hm://pusher/v1/connections/", |message| {
            message
                .headers
                .get("Spotify-Connection-Id")
                .cloned()
                .ok_or_else(|| librespot_core::Error::internal("no connection id"))
        })?;
    session.dealer().start().await?;
    let id = tokio::time::timeout(Duration::from_secs(10), connection.next())
        .await?
        .ok_or_else(|| anyhow::anyhow!("dealer closed while connecting"))??;
    session.set_connection_id(&id);
    Ok(session)
}

async fn run(session: &Session, cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Current => current(session).await?,
        Cmd::Devices => devices(session).await?,
        Cmd::Start { device } => start(session, &device).await?,
        Cmd::End => end(session).await?,
        Cmd::Kick { member_id } => kick(session, &member_id).await?,
        Cmd::Perm { permission, on } => perm(session, permission, on).await?,
        Cmd::Watch => watch(session).await?,
        Cmd::Help(topic) => print!("{}", help_for(topic.as_deref())),
    }
    Ok(())
}

async fn current(session: &Session) -> anyhow::Result<()> {
    match get_current(session).await {
        Ok(answer) => {
            println!("HTTP 200");
            print!("{}", summarize_session(&answer));
        }
        Err(ProbeError::Http(404)) => println!("HTTP 404 (no current Jam)"),
        Err(error) => println!("{error}"),
    }
    Ok(())
}

async fn devices(session: &Session) -> anyhow::Result<()> {
    let cluster = match read_cluster(session).await {
        Ok(cluster) => cluster,
        Err(error) => {
            println!("cannot read the Connect cluster: {error}");
            return Ok(());
        }
    };
    println!("active device: {}", cluster.active_device_id);
    for (id, device) in &cluster.device {
        let playing = if id == &cluster.active_device_id {
            "  <- playing"
        } else {
            ""
        };
        let hidden = if device.capabilities.hidden {
            "  (hidden)"
        } else {
            ""
        };
        println!(
            "{id}  {}  {:?}{playing}{hidden}",
            device.name, device.device_type
        );
    }
    Ok(())
}

async fn start(session: &Session, name: &str) -> anyhow::Result<()> {
    let cluster = match read_cluster(session).await {
        Ok(cluster) => cluster,
        Err(error) => {
            println!("cannot read the Connect cluster: {error}");
            return Ok(());
        }
    };
    let Some((device_id, _)) = cluster
        .device
        .iter()
        .find(|(_, device)| device.name == name)
    else {
        println!("no Connect device named {name:?}; run `jam_probe -- devices`");
        return Ok(());
    };
    if !confirm(&format!("Start a Jam on {name:?}?")) {
        println!("not sent");
        return Ok(());
    }

    let path = start_path(device_id);
    println!("GET {path}");
    let answer = match request(session, reqwest::Method::GET, &path, None).await {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map_err(|error| ProbeError::Other(format!("start answer is not JSON: {error}"))),
        Err(error) => Err(error),
    };
    println!(
        "{}",
        start_verdict(answer.as_ref(), device_id, name, session.device_id())
    );

    if let Ok(answer) = &answer {
        print!("{}", summarize_session(answer));
        match invite_link(answer) {
            Some(link) => {
                println!("\ninvite: {link}\n");
                print_qr(&link);
            }
            None => println!("no join token in the answer: no invite link to show"),
        }
        match read_cluster(session).await {
            Ok(cluster) => {
                let jam_id = field_str(session_object(answer), "session_id", "sessionId")
                    .unwrap_or("(missing)");
                let cluster_id = if cluster.player_state.session_id.is_empty() {
                    "(empty)"
                } else {
                    &cluster.player_state.session_id
                };
                println!(
                    "cluster player_state.session_id: {cluster_id}  (Jam session_id: {jam_id})"
                );
            }
            Err(error) => println!("cluster read failed: {error}"),
        }
    }
    Ok(())
}

async fn end(session: &Session) -> anyhow::Result<()> {
    let answer = match get_current(session).await {
        Ok(answer) => answer,
        Err(ProbeError::Http(404)) => {
            println!("no current Jam");
            return Ok(());
        }
        Err(error) => {
            println!("cannot read the current Jam: {error}");
            return Ok(());
        }
    };
    let current = session_object(&answer);
    if !field_bool(current, "active", "active").unwrap_or(false) {
        println!("no active Jam (a dormant record, or `active` is missing)");
        return Ok(());
    }
    if !field_bool(current, "is_session_owner", "isSessionOwner").unwrap_or(false) {
        println!("refusing: this account is not the session owner");
        return Ok(());
    }
    let Some(session_id) = field_str(current, "session_id", "sessionId") else {
        println!("the current Jam has no session_id");
        return Ok(());
    };
    if !confirm("End the Jam for everyone?") {
        println!("not sent");
        return Ok(());
    }
    let candidates = end_candidates(session_id, session.device_id());
    try_candidates(session, &candidates).await?;
    Ok(())
}

async fn kick(session: &Session, member_id: &str) -> anyhow::Result<()> {
    let Some(session_id) = owned_session(session).await? else {
        return Ok(());
    };
    if !confirm(&format!("Remove member {member_id} from the Jam?")) {
        println!("not sent");
        return Ok(());
    }
    let candidate = kick_candidate(&session_id, member_id, session.device_id());
    try_candidates(session, &[candidate]).await?;
    Ok(())
}

async fn perm(session: &Session, permission: Permission, on: bool) -> anyhow::Result<()> {
    let Some(session_id) = owned_session(session).await? else {
        return Ok(());
    };
    let setting = match permission {
        Permission::QueueOnly => "queue-only",
        Permission::Volume => "guest volume",
    };
    let state = if on { "on" } else { "off" };
    if !confirm(&format!("Set {setting} {state} for guests?")) {
        println!("not sent");
        return Ok(());
    }
    let candidates = permission_candidates(permission, on, &session_id, session.device_id());
    try_candidates(session, &candidates).await?;
    Ok(())
}

async fn watch(session: &Session) -> anyhow::Result<()> {
    let mut updates = session.dealer().listen_for(
        SESSION_UPDATE_TOPIC,
        |message| -> Result<DealerMessage, librespot_core::Error> { Ok(message) },
    )?;
    println!("watching {SESSION_UPDATE_TOPIC} and the Connect cluster; Ctrl-C to stop");
    let mut seen = String::new();
    if let Ok(cluster) = read_cluster(session).await {
        seen = cluster_snapshot(&cluster);
        print_cluster(&cluster);
    }
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("stopped");
                break;
            }
            message = updates.next() => {
                match message {
                    Some(Ok(message)) => print_update(message),
                    Some(Err(error)) => println!("session_update error: {error}"),
                    None => {
                        println!("session_update stream ended");
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                match read_cluster(session).await {
                    Ok(cluster) => {
                        let snapshot = cluster_snapshot(&cluster);
                        if snapshot != seen {
                            seen = snapshot;
                            print_cluster(&cluster);
                        }
                    }
                    Err(error) => println!("cluster read failed: {error}"),
                }
            }
        }
    }
    Ok(())
}

async fn owned_session(session: &Session) -> anyhow::Result<Option<String>> {
    let answer = match get_current(session).await {
        Ok(answer) => answer,
        Err(ProbeError::Http(404)) => {
            println!("no current Jam");
            return Ok(None);
        }
        Err(error) => {
            println!("cannot read the current Jam: {error}");
            return Ok(None);
        }
    };
    let current = session_object(&answer);
    if !field_bool(current, "active", "active").unwrap_or(false) {
        println!("no active Jam (a dormant record, or `active` is missing)");
        return Ok(None);
    }
    if !field_bool(current, "is_session_owner", "isSessionOwner").unwrap_or(false) {
        println!("refusing: this account is not the session owner");
        return Ok(None);
    }
    match field_str(current, "session_id", "sessionId") {
        Some(id) => Ok(Some(id.to_string())),
        None => {
            println!("the current Jam has no session_id");
            Ok(None)
        }
    }
}

async fn get_current(session: &Session) -> Result<Value, ProbeError> {
    let bytes = request(session, reqwest::Method::GET, SESSION_CURRENT, None).await?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ProbeError::Other(format!("current answer is not JSON: {error}")))
}

async fn request(
    session: &Session,
    method: reqwest::Method,
    path: &str,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, ProbeError> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    if body.is_some() {
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    let mut result = session
        .spclient()
        .request(&method, path, Some(headers.clone()), body)
        .await;
    if let Err(error) = &result
        && let Some(code) = status_code(error)
        && is_backoff_status(code)
    {
        println!("HTTP {code}: backing off once");
        tokio::time::sleep(Duration::from_secs(2)).await;
        result = session
            .spclient()
            .request(&method, path, Some(headers), body)
            .await;
    }
    result.map(|bytes| bytes.to_vec()).map_err(probe_error)
}

async fn put_state(session: &Session) -> Result<Vec<u8>, ProbeError> {
    let mut state = hidden_state(session);
    let mut result = session.spclient().put_connect_state_request(&state).await;
    if let Err(error) = &result
        && let Some(code) = status_code(error)
        && is_backoff_status(code)
    {
        println!("HTTP {code}: backing off once");
        tokio::time::sleep(Duration::from_secs(2)).await;
        state = hidden_state(session);
        result = session.spclient().put_connect_state_request(&state).await;
    }
    result.map(|bytes| bytes.to_vec()).map_err(probe_error)
}

async fn read_cluster(session: &Session) -> Result<Cluster, ProbeError> {
    let bytes = put_state(session).await?;
    Cluster::parse_from_bytes(&bytes)
        .map_err(|error| ProbeError::Other(format!("cluster parse failed: {error}")))
}

fn hidden_state(session: &Session) -> PutStateRequest {
    let mut request = PutStateRequest::new();
    request.member_type = MemberType::CONNECT_STATE.into();
    request.put_state_reason = PutStateReason::NEW_DEVICE.into();
    let mut info = DeviceInfo::new();
    info.name = HIDDEN_NAME.into();
    info.device_id = session.device_id().to_string();
    info.can_play = false;
    let mut capabilities = Capabilities::new();
    capabilities.hidden = true;
    capabilities.is_observable = true;
    capabilities.is_controllable = false;
    capabilities.can_be_player = false;
    capabilities.gaia_eq_connect_id = true;
    info.capabilities = Some(capabilities).into();
    let mut device = Device::new();
    device.device_info = Some(info).into();
    request.device = Some(device).into();
    request
}

fn status_code(error: &librespot_core::Error) -> Option<u16> {
    error
        .error
        .downcast_ref::<HttpClientError>()
        .map(|HttpClientError::StatusCode(code)| code.as_u16())
}

fn probe_error(error: librespot_core::Error) -> ProbeError {
    match status_code(&error) {
        Some(code) => ProbeError::Http(code),
        None => ProbeError::Other(error.to_string()),
    }
}

fn is_backoff_status(code: u16) -> bool {
    code == 429 || (500..=599).contains(&code)
}

fn should_fallback(code: u16) -> bool {
    matches!(code, 400 | 404 | 405 | 415 | 422)
}

async fn send_candidate(session: &Session, candidate: &Candidate) -> Result<(), ProbeError> {
    let body = candidate.body.as_deref().map(str::as_bytes);
    request(session, candidate.verb.method(), &candidate.path, body)
        .await
        .map(|_| ())
}

async fn try_candidates(session: &Session, candidates: &[Candidate]) -> anyhow::Result<bool> {
    for candidate in candidates {
        match send_candidate(session, candidate).await {
            Ok(()) => {
                println!(
                    "{} {} -> HTTP 2xx (answered)",
                    candidate.verb, candidate.path
                );
                return Ok(true);
            }
            Err(ProbeError::Http(code)) => {
                println!("{} {} -> HTTP {code}", candidate.verb, candidate.path);
                if should_fallback(code) {
                    continue;
                }
                return Ok(true);
            }
            Err(error) => {
                println!("{} {} -> {error}", candidate.verb, candidate.path);
                return Err(error.into());
            }
        }
    }
    println!("no candidate answered (all asked for the next one)");
    Ok(false)
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
        return false;
    }
    matches!(line.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
}

fn start_path(device_id: &str) -> String {
    format!(
        "/social-connect/v2/sessions/current_or_new?local_device_id={device_id}&activate=true&type=REMOTE"
    )
}

fn end_candidates(session_id: &str, local_device_id: &str) -> Vec<Candidate> {
    vec![
        Candidate {
            verb: Verb::Delete,
            path: format!(
                "/social-connect/v2/sessions/{session_id}?local_device_id={local_device_id}"
            ),
            body: None,
        },
        Candidate {
            verb: Verb::Delete,
            path: format!(
                "/social-connect/v3/sessions/{session_id}?local_device_id={local_device_id}"
            ),
            body: None,
        },
    ]
}

fn kick_candidate(session_id: &str, member_id: &str, local_device_id: &str) -> Candidate {
    Candidate {
        verb: Verb::Post,
        path: format!(
            "/social-connect/v3/sessions/{session_id}/member/{member_id}/kick?local_device_id={local_device_id}"
        ),
        body: None,
    }
}

fn permission_candidates(
    permission: Permission,
    on: bool,
    session_id: &str,
    local_device_id: &str,
) -> Vec<Candidate> {
    let (slug, field, value) = match permission {
        Permission::QueueOnly => ("queue-only-mode", "queue_only_mode", Value::Bool(on)),
        Permission::Volume => (
            "participant-volume-control",
            "participant_volume_control",
            Value::String(if on { "ENABLED" } else { "DISABLED" }.into()),
        ),
    };
    let body = json!({ (field): value }).to_string();
    let underscore = slug.replace('-', "_");
    let paths = [
        (
            Verb::Post,
            format!(
                "/social-connect/v2/sessions/{session_id}/{slug}?local_device_id={local_device_id}"
            ),
        ),
        (
            Verb::Post,
            format!(
                "/social-connect/v2/sessions/{session_id}/{underscore}?local_device_id={local_device_id}"
            ),
        ),
        (
            Verb::Put,
            format!(
                "/social-connect/v2/sessions/{session_id}/{slug}?local_device_id={local_device_id}"
            ),
        ),
        (
            Verb::Post,
            format!(
                "/social-connect/v3/sessions/{session_id}/{slug}?local_device_id={local_device_id}"
            ),
        ),
    ];
    paths
        .into_iter()
        .map(|(verb, path)| Candidate {
            verb,
            path,
            body: Some(body.clone()),
        })
        .collect()
}

fn session_object(value: &Value) -> &Value {
    value
        .get("session")
        .filter(|session| session.is_object())
        .unwrap_or(value)
}

fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    value.get(snake).or_else(|| value.get(camel))
}

fn field_str<'a>(value: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    field(value, snake, camel).and_then(Value::as_str)
}

fn field_bool(value: &Value, snake: &str, camel: &str) -> Option<bool> {
    field(value, snake, camel).and_then(Value::as_bool)
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn summarize_session(value: &Value) -> String {
    let session = session_object(value);
    let mut out = String::new();
    let mut line = |name: &str, snake: &str, camel: &str| {
        let text = field(session, snake, camel)
            .map(scalar)
            .unwrap_or_else(|| "(missing)".into());
        out.push_str(&format!("{name}: {text}\n"));
    };
    line("session_id", "session_id", "sessionId");
    line("active", "active", "active");
    line("is_session_owner", "is_session_owner", "isSessionOwner");
    line(
        "host_active_device_id",
        "host_active_device_id",
        "hostActiveDeviceId",
    );
    line("queue_only_mode", "queue_only_mode", "queueOnlyMode");
    line(
        "participantVolumeControl",
        "participant_volume_control",
        "participantVolumeControl",
    );
    line("user_capabilities", "user_capabilities", "userCapabilities");
    match field(session, "session_members", "sessionMembers").and_then(Value::as_array) {
        Some(members) => {
            out.push_str(&format!("members: {}\n", members.len()));
            for member in members {
                let name = field_str(member, "display_name", "displayName").unwrap_or("(no name)");
                let id = field_str(member, "id", "id").unwrap_or("(no id)");
                let listening = field_bool(member, "is_listening", "isListening")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".into());
                let controlling = field_bool(member, "is_controlling", "isControlling")
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".into());
                out.push_str(&format!(
                    "  - {name}  id={id}  listening={listening}  controlling={controlling}\n"
                ));
            }
        }
        None => out.push_str("members: 0\n"),
    }
    out
}

fn invite_link(value: &Value) -> Option<String> {
    let session = session_object(value);
    let token = field_str(session, "join_session_token", "joinSessionToken")
        .map(str::to_string)
        .or_else(|| {
            field_str(session, "join_session_uri", "joinSessionUri")
                .and_then(|uri| uri.rsplit(':').next())
                .map(str::to_string)
        })
        .or_else(|| {
            field_str(session, "join_session_url", "joinSessionUrl")
                .and_then(|url| url.rsplit('/').next())
                .and_then(|last| last.split('?').next())
                .map(str::to_string)
        })
        .filter(|token| {
            !token.is_empty()
                && token
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        });
    token.map(|token| format!("{INVITE_BASE}/{token}"))
}

fn start_verdict(
    answer: Result<&Value, &ProbeError>,
    named_id: &str,
    named_name: &str,
    hidden_id: &str,
) -> String {
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => return format!("start failed: {error}"),
    };
    match field_str(
        session_object(answer),
        "host_active_device_id",
        "hostActiveDeviceId",
    ) {
        Some(id) if id == named_id => format!("Jam hosted on {named_name}"),
        Some(id) if id == hidden_id => "Jam hosted on the hidden controller".into(),
        Some(id) => format!("Jam hosted on an unknown device ({id})"),
        None => "Jam hosted on an unknown device (host_active_device_id is missing)".into(),
    }
}

fn render_qr(text: &str) -> String {
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).expect("invite link fits in a QR code");
    let border = 4;
    let size = code.size();
    let mut out = String::new();
    let mut y = -border;
    while y < size + border {
        out.push_str("\x1b[30;47m");
        let mut x = -border;
        while x < size + border {
            let top = qr_module(&code, x, y);
            let bottom = qr_module(&code, x, y + 1);
            out.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
            x += 1;
        }
        out.push_str("\x1b[0m\n");
        y += 2;
    }
    out
}

fn qr_module(code: &QrCode, x: i32, y: i32) -> bool {
    let size = code.size();
    x >= 0 && y >= 0 && x < size && y < size && code.get_module(x, y)
}

fn print_qr(link: &str) {
    print!("{}", render_qr(link));
    println!("(scan this, or open the link above, on the second phone)");
}

fn print_update(message: DealerMessage) {
    let text = match message.payload {
        PayloadValue::Json(text) => text,
        PayloadValue::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        PayloadValue::Empty => {
            println!("session_update: empty payload");
            return;
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(value) => {
            let reason = field_str(&value, "reason", "reason").unwrap_or("(no reason)");
            println!("\n--- session_update: {reason}");
            print!("{}", summarize_session(&value));
            if let Some(members) = field(&value, "updated_session_members", "updatedSessionMembers")
                .and_then(Value::as_array)
            {
                let names: Vec<&str> = members
                    .iter()
                    .map(|member| {
                        field_str(member, "display_name", "displayName").unwrap_or("(no name)")
                    })
                    .collect();
                println!(
                    "updated_session_members: {} ({})",
                    names.len(),
                    names.join(", ")
                );
            }
        }
        Err(error) => println!("session_update: payload is not JSON ({error})"),
    }
}

fn cluster_snapshot(cluster: &Cluster) -> String {
    format!(
        "{}\u{1f}{}",
        cluster.player_state.session_id, cluster.player_state.play_origin.feature_identifier
    )
}

fn print_cluster(cluster: &Cluster) {
    let state = &cluster.player_state;
    let session_id = if state.session_id.is_empty() {
        "(empty)"
    } else {
        &state.session_id
    };
    let feature = if state.play_origin.feature_identifier.is_empty() {
        "(none)"
    } else {
        &state.play_origin.feature_identifier
    };
    println!(
        "cluster: player_state.session_id={session_id}  play_origin.feature_identifier={feature}"
    );
}

fn usage() -> &'static str {
    "\
usage:
  jam_probe [current]                                  read the account's current Jam (default)
  jam_probe devices                                    list Connect devices and their ids
  jam_probe start --device \"<name>\"                    start a Jam on that device, show link and QR
  jam_probe end                                        end the Jam this account owns
  jam_probe kick <member_id>                           remove a guest (asks first)
  jam_probe perm queue-only on|off                     guests may add only, or also control (asks first)
  jam_probe perm volume on|off                         guests may change volume (asks first)
  jam_probe watch                                      print session_update and cluster changes
  jam_probe <command> --help                           help for one command

Run `jam_probe --help` for the whole help. Commands that change the Jam ask for `y` on stdin."
}

fn help_for(topic: Option<&str>) -> String {
    match topic {
        None => format!(
            "\
jam_probe: inspect and exercise Spotify's Jam (social-connect) API.

It uses the saved Catalogue login (the same one `connect_probe` uses), registers a hidden
controller for the run and removes it afterwards. It never prints access tokens or writes to disk.

{}

Reading output:
  current       HTTP 200 plus the session fields; HTTP 404 means no Jam. `active` missing/false is a
                dormant record left by an ended Jam.
  devices       the Connect device id, name, type and the one marked \"<- playing\".
  start         one plain line first: \"Jam hosted on <name>\", \"Jam hosted on the hidden controller\",
                or \"start failed: HTTP <code>\". Then the invite link and a QR, and the cluster's
                player_state.session_id next to the Jam's session_id (they should match on the host).
  end / kick /  each candidate line ends in the HTTP status that answered. 2xx is success;
  perm          \"no candidate answered\" means every assumed path was 404/405/400-ish.
  watch         one block per session_update reason, then cluster lines when player_state.session_id
                or play_origin.feature_identifier changes. Ctrl-C stops it.",
            usage()
        ),
        Some("current") => "jam_probe current\n\nRead GET /social-connect/v2/sessions/current and print HTTP status and the session\nstructure. Read-only; this is also the default command. Tokens are not printed.\n".into(),
        Some("devices") => "jam_probe devices\n\nRegister the hidden controller and list the Connect cluster's devices with their ids,\nmarking the active (playing) one. Read-only.\n".into(),
        Some("start") => "jam_probe start --device \"<name>\"\n\nStart a Jam on the named Connect device with GET /social-connect/v2/sessions/current_or_new\n(activate=true, type=REMOTE), then print the verdict, the invite link and a QR. Asks for y first\nbecause it changes account state. Use `devices` to see the names.\n".into(),
        Some("end") => "jam_probe end\n\nDELETE the current Jam if this account owns it. Tries the v2 path, then v3, and prints\nwhich one answered. Refuses when `is_session_owner` is false. Asks for y first.\n".into(),
        Some("kick") => "jam_probe kick <member_id>\n\nRemove a guest with POST /social-connect/v3/sessions/{id}/member/{member_id}/kick.\nUse the `id=` shown by `current`. Asks for y first; it changes another person's phone.\n".into(),
        Some("perm") => "jam_probe perm queue-only on|off\njam_probe perm volume on|off\n\nSet `queue_only_mode` or `participantVolumeControl`. The permission paths are assumed, so the\nprobe tries several in order and prints the HTTP status of each; the first that answers wins.\nAsks for y first; it changes other people's phones.\n".into(),
        Some("watch") => "jam_probe watch\n\nSubscribe to the dealer topic social-connect/v2/session_update and print each update's reason and\nsession fields, plus cluster changes to player_state.session_id and a guest command's\nPlayOrigin.feature_identifier. Ctrl-C stops it.\n".into(),
        Some(other) => format!("unknown command {other:?}\n\n{}", usage()),
    }
}

fn parse_args(args: Vec<String>) -> Result<Cmd, String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Cmd::Current);
    };
    let rest = &args[1..];
    match command {
        "-h" | "--help" | "help" => Ok(Cmd::Help(
            rest.first()
                .map(String::as_str)
                .filter(|topic| *topic != "--help")
                .map(str::to_string),
        )),
        "current" => {
            if wants_help(rest) {
                Ok(Cmd::Help(Some("current".into())))
            } else if rest.is_empty() {
                Ok(Cmd::Current)
            } else {
                Err("current takes no arguments".into())
            }
        }
        "devices" => {
            if wants_help(rest) {
                Ok(Cmd::Help(Some("devices".into())))
            } else if rest.is_empty() {
                Ok(Cmd::Devices)
            } else {
                Err("devices takes no arguments".into())
            }
        }
        "start" => {
            if wants_help(rest) {
                return Ok(Cmd::Help(Some("start".into())));
            }
            match rest {
                [flag, name] if flag == "--device" && !name.is_empty() => Ok(Cmd::Start {
                    device: name.clone(),
                }),
                _ => Err("start needs --device \"<name>\"".into()),
            }
        }
        "end" => {
            if wants_help(rest) {
                Ok(Cmd::Help(Some("end".into())))
            } else if rest.is_empty() {
                Ok(Cmd::End)
            } else {
                Err("end takes no arguments".into())
            }
        }
        "kick" => {
            if wants_help(rest) {
                return Ok(Cmd::Help(Some("kick".into())));
            }
            match rest {
                [member_id] if !member_id.is_empty() => Ok(Cmd::Kick {
                    member_id: member_id.clone(),
                }),
                _ => Err("kick needs one member id (the `id=` from `current`)".into()),
            }
        }
        "perm" => {
            if wants_help(rest) {
                return Ok(Cmd::Help(Some("perm".into())));
            }
            let (permission, state) = match rest {
                [which, state] => (which.as_str(), state.as_str()),
                _ => return Err("perm needs `queue-only|volume` and `on|off`".into()),
            };
            let permission = match permission {
                "queue-only" => Permission::QueueOnly,
                "volume" => Permission::Volume,
                other => {
                    return Err(format!(
                        "unknown permission {other:?}; use queue-only or volume"
                    ));
                }
            };
            let on = match state {
                "on" => true,
                "off" => false,
                other => return Err(format!("unknown state {other:?}; use on or off")),
            };
            Ok(Cmd::Perm { permission, on })
        }
        "watch" => {
            if wants_help(rest) {
                Ok(Cmd::Help(Some("watch".into())))
            } else if rest.is_empty() {
                Ok(Cmd::Watch)
            } else {
                Err("watch takes no arguments".into())
            }
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn wants_help(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--help" || arg == "-h")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("fixture is JSON")
    }

    fn session_with_host(host: &str) -> Value {
        json(&format!(
            r#"{{
                "session_id": "session-1",
                "join_session_token": "AbC123",
                "is_session_owner": true,
                "active": true,
                "host_active_device_id": "{host}",
                "session_members": [{{"id": "user-1", "display_name": "Guest", "is_listening": true, "is_controlling": false}}],
                "queue_only_mode": false,
                "participantVolumeControl": "DISABLED",
                "user_capabilities": {{"invite": true, "remove_participants": true, "manage_participant_settings": true}}
            }}"#
        ))
    }

    #[test]
    fn start_path_names_the_device_and_activates() {
        assert_eq!(
            start_path("device-1"),
            "/social-connect/v2/sessions/current_or_new?local_device_id=device-1&activate=true&type=REMOTE"
        );
    }

    #[test]
    fn verdict_is_hosted_on_the_named_device() {
        let answer = session_with_host("device-1");
        assert_eq!(
            start_verdict(Ok(&answer), "device-1", "Soloist", "hidden-1"),
            "Jam hosted on Soloist"
        );
    }

    #[test]
    fn verdict_is_hosted_on_the_hidden_controller() {
        let answer = session_with_host("hidden-1");
        assert_eq!(
            start_verdict(Ok(&answer), "device-1", "Soloist", "hidden-1"),
            "Jam hosted on the hidden controller"
        );
    }

    #[test]
    fn verdict_reports_an_http_error() {
        let error = ProbeError::Http(403);
        assert_eq!(
            start_verdict(Err(&error), "device-1", "Soloist", "hidden-1"),
            "start failed: HTTP 403"
        );
    }

    #[test]
    fn verdict_accepts_camel_case_host_field() {
        let answer = json(r#"{"hostActiveDeviceId": "device-1"}"#);
        assert_eq!(
            start_verdict(Ok(&answer), "device-1", "Soloist", "hidden-1"),
            "Jam hosted on Soloist"
        );
    }

    #[test]
    fn invite_link_comes_from_the_token() {
        let answer = json(r#"{"join_session_token": "AbC123"}"#);
        assert_eq!(
            invite_link(&answer).as_deref(),
            Some("https://open.spotify.com/socialsession/AbC123")
        );
    }

    #[test]
    fn invite_link_falls_back_to_the_uri_and_url() {
        let from_uri = json(r#"{"join_session_uri": "spotify:socialsession:AbC123"}"#);
        assert_eq!(
            invite_link(&from_uri).as_deref(),
            Some("https://open.spotify.com/socialsession/AbC123")
        );
        let from_url =
            json(r#"{"join_session_url": "hm://social-connect/v2/sessions/join/AbC123?x=1"}"#);
        assert_eq!(
            invite_link(&from_url).as_deref(),
            Some("https://open.spotify.com/socialsession/AbC123")
        );
    }

    #[test]
    fn summary_never_prints_join_tokens_or_the_internal_url() {
        let answer = json(
            r#"{
                "session_id": "session-1",
                "join_session_token": "SecretToken1",
                "join_session_url": "hm://secret/path/SecretToken1",
                "join_session_uri": "spotify:socialsession:SecretToken1",
                "active": true,
                "is_session_owner": true,
                "host_active_device_id": "device-1",
                "session_members": [],
                "user_capabilities": {"invite": true}
            }"#,
        );
        let summary = summarize_session(&answer);
        assert!(!summary.contains("SecretToken1"));
        assert!(!summary.contains("hm://"));
        assert!(summary.contains("host_active_device_id: device-1"));
        assert!(summary.contains("members: 0"));
    }

    #[test]
    fn end_tries_v2_then_v3() {
        let candidates = end_candidates("session-1", "device-1");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].verb, Verb::Delete);
        assert_eq!(candidates[1].verb, Verb::Delete);
        assert!(
            candidates[0]
                .path
                .starts_with("/social-connect/v2/sessions/session-1?")
        );
        assert!(
            candidates[1]
                .path
                .starts_with("/social-connect/v3/sessions/session-1?")
        );
    }

    #[test]
    fn permission_candidates_fall_back_in_order() {
        let queue = permission_candidates(Permission::QueueOnly, true, "session-1", "device-1");
        assert_eq!(queue.len(), 4);
        assert_eq!(queue[0].verb, Verb::Post);
        assert!(
            queue[0]
                .path
                .contains("/v2/sessions/session-1/queue-only-mode?")
        );
        assert!(
            queue[1]
                .path
                .contains("/v2/sessions/session-1/queue_only_mode?")
        );
        assert_eq!(queue[2].verb, Verb::Put);
        assert!(
            queue[3]
                .path
                .contains("/v3/sessions/session-1/queue-only-mode?")
        );
        assert_eq!(
            queue[0].body.as_deref(),
            Some(r#"{"queue_only_mode":true}"#)
        );

        let volume = permission_candidates(Permission::Volume, false, "session-1", "device-1");
        assert_eq!(
            volume[0].body.as_deref(),
            Some(r#"{"participant_volume_control":"DISABLED"}"#)
        );
        assert!(volume[0].path.contains("/participant-volume-control?"));
    }

    #[test]
    fn kick_uses_the_v3_member_path() {
        let candidate = kick_candidate("session-1", "user-1", "device-1");
        assert_eq!(candidate.verb, Verb::Post);
        assert_eq!(
            candidate.path,
            "/social-connect/v3/sessions/session-1/member/user-1/kick?local_device_id=device-1"
        );
    }

    #[test]
    fn backoff_is_only_for_429_and_5xx() {
        assert!(is_backoff_status(429));
        assert!(is_backoff_status(500));
        assert!(is_backoff_status(503));
        assert!(!is_backoff_status(400));
        assert!(!is_backoff_status(403));
        assert!(!is_backoff_status(404));
    }

    #[test]
    fn qr_is_rendered_with_unicode_half_blocks() {
        let rendered = render_qr("https://open.spotify.com/socialsession/AbC123");
        assert!(rendered.contains('█'));
        assert!(rendered.contains('▀') || rendered.contains('▄'));
        assert!(rendered.starts_with("\x1b[30;47m"));
        assert!(rendered.lines().count() > 20);
    }

    #[test]
    fn parse_rejects_missing_device() {
        assert!(parse_args(vec!["start".into()]).is_err());
        assert!(matches!(
            parse_args(vec!["start".into(), "--device".into(), "Soloist".into()]),
            Ok(Cmd::Start { .. })
        ));
    }

    #[test]
    fn help_exists_for_every_command() {
        for command in [
            "current", "devices", "start", "end", "kick", "perm", "watch",
        ] {
            assert!(help_for(Some(command)).contains(command));
        }
    }
}
