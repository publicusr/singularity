//! `singularity`: the terminal Client.

mod conn;

use conn::ConnEvent;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use singularity_proto::{DaemonMsg, Event, EngineStatus};
use std::time::Duration;

enum Link {
    Connecting,
    Connected { version: String, started_unix_ms: u64 },
    Incompatible(String),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("singularity {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (_cmd_tx, mut conn_rx) = conn::spawn();
    let mut term = ratatui::init();
    let mut link = Link::Connecting;
    let mut engine: Option<EngineStatus> = None;
    let res = loop {
        while let Ok(ev) = conn_rx.try_recv() {
            match ev {
                ConnEvent::Connecting | ConnEvent::Disconnected => link = Link::Connecting,
                ConnEvent::Connected { daemon_version, started_unix_ms } => link = Link::Connected { version: daemon_version, started_unix_ms },
                ConnEvent::Incompatible(m) => link = Link::Incompatible(m),
                ConnEvent::Msg(DaemonMsg::Event(Event::Engine(s))) => engine = Some(s),
                ConnEvent::Msg(_) => {}
            }
        }
        let draw = term.draw(|f| {
            let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1)]).areas(f.area());
            let (text, color) = match &link {
                Link::Connecting => ("connecting to singularityd…".to_string(), Color::Rgb(120, 120, 120)),
                Link::Connected { version, started_unix_ms } => {
                    let up = singularity_proto::now_ms().saturating_sub(*started_unix_ms) / 1000;
                    (format!("connected to singularityd v{version} · up {}m {}s", up / 60, up % 60), Color::Rgb(242, 211, 160))
                }
                Link::Incompatible(m) => (m.clone(), Color::Rgb(220, 76, 76)),
            };
            let engine_line = match &engine {
                Some(EngineStatus::NotConfigured { reason }) => format!("engine: not configured ({reason})"),
                Some(s) => format!("engine: {s:?}"),
                None => String::new(),
            };
            let p = Paragraph::new(vec![
                Line::styled("◉ SINGULARITY", Style::default().fg(Color::Rgb(242, 211, 160))),
                Line::styled(text, Style::default().fg(color)),
                Line::styled(engine_line, Style::default().fg(Color::Rgb(90, 90, 90))),
            ])
            .centered();
            f.render_widget(p, mid);
        });
        if let Err(e) = draw {
            break Err(e.into());
        }
        if event::poll(Duration::from_millis(100))? {
            if let TermEvent::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Release && matches!(k.code, KeyCode::Char('q') | KeyCode::Esc) {
                    break Ok(());
                }
            }
        }
    };
    ratatui::restore();
    res
}
