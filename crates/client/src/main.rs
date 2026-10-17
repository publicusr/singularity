//! `singularity`: the terminal Client.

mod app;
mod conn;
mod layout;
mod theme;
mod ui;

use app::{App, Link};
use conn::ConnEvent;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyEventKind};
use ratatui::crossterm::execute;
use singularity_proto::{DaemonMsg, Event};
use std::io::stdout;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularity {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (cmd_tx, mut conn_rx) = conn::spawn();
    let mut app = App::new(Some(cmd_tx));
    let mut term = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let res = (|| -> anyhow::Result<()> {
        while !app.quit {
            while let Ok(ev) = conn_rx.try_recv() {
                apply(&mut app, ev);
            }
            term.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(33))? {
                loop {
                    match event::read()? {
                        TermEvent::Key(k) if k.kind != KeyEventKind::Release => app.key(k),
                        TermEvent::Mouse(m) => app.mouse(m),
                        _ => {}
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

fn apply(app: &mut App, ev: ConnEvent) {
    match ev {
        ConnEvent::Connecting => {}
        ConnEvent::Disconnected => app.link = Link::Connecting,
        ConnEvent::Connected { daemon_version } => app.link = Link::Connected { version: daemon_version },
        ConnEvent::Incompatible(m) => {
            app.say(m.clone());
            app.link = Link::Incompatible(m);
        }
        ConnEvent::Msg(DaemonMsg::Event(Event::Engine(s))) => app.engine = Some(s),
        ConnEvent::Msg(DaemonMsg::Event(Event::Playback(p))) => app.playback = p,
        ConnEvent::Msg(DaemonMsg::Event(Event::Error { message })) => app.say(message),
        ConnEvent::Msg(_) => {}
    }
}
