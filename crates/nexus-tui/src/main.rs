//! `nexus-tui` binary entry point.
//!
//! Owns the terminal (raw mode, alternate screen, mouse capture) and the
//! event loop. All routing from crossterm events to the application goes
//! through [`InputTranslator`]; rendering is a single thin call to
//! [`render_frame`].

use std::io::{self, stdout};
use std::panic;
use std::time::Duration;

use crossterm::event::{self, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use nexus_client::SessionService;
use nexus_core::PerformanceMetrics;
use nexus_tui::app::render_frame;
use nexus_tui::app::App;
use nexus_tui::input::{InputTranslator, UiEvent};

/// Default server address when `NEXUS_ADDRESS` is not set.
const DEFAULT_ADDRESS: &str = "ws://localhost:9000";

fn main() -> io::Result<()> {
    // Restore the terminal even on panic so users are not stranded.
    let _guard = TerminalGuard::enter()?;

    let address = std::env::var("NEXUS_ADDRESS").unwrap_or_else(|_| DEFAULT_ADDRESS.into());
    let mut app = App::new(SessionService::new(address));
    if let Ok((cols, rows)) = crossterm::terminal::size() {
        app.resize(cols, rows);
    }
    app.session.connect(env_username());

    // The tick event drives periodic work (network drains, cursor blink).
    let result = run_loop(&mut app);

    let _ = crossterm::execute!(stdout(), crossterm::cursor::Show);
    result?;
    Ok(())
}

fn env_username() -> String {
    std::env::var("NEXUS_USERNAME").unwrap_or_else(|_| "anon".to_string())
}

fn run_loop(app: &mut App) -> io::Result<()> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;
    terminal.hide_cursor()?;

    let perf = std::sync::Arc::new(PerformanceMetrics::new());

    // Redraw every ~33ms even when idle so blink/scrollback stay live.
    let mut last_tick = std::time::Instant::now();
    loop {
        if event::poll(Duration::from_millis(33))? {
            let raw = event::read()?;
            if let Some(ui) = InputTranslator::translate(raw) {
                app.handle_event(ui);
            }
        } else if last_tick.elapsed() >= Duration::from_millis(33) {
            app.handle_event(UiEvent::Tick);
            last_tick = std::time::Instant::now();
        }

        if app.quit {
            break;
        }

        // Re-read the terminal size each frame; resize when it changed.
        if let Ok((cols, rows)) = crossterm::terminal::size() {
            if cols != app.cols || rows != app.rows {
                app.resize(cols, rows);
            }
        }

        terminal.draw(|frame| render_frame(app, frame))?;
        perf.record_latency_ms(std::time::Instant::now().elapsed().as_millis() as u64);
    }

    let _ = terminal.show_cursor();
    Ok(())
}

/// Restores terminal state on teardown, including panics.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        out.execute(EnterAlternateScreen)?;
        out.execute(EnableMouseCapture)?;

        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let _ = restore();
            previous(info);
        }));
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore();
        let _ = panic::take_hook();
    }
}

#[allow(clippy::result_unit_err)]
fn restore() -> Result<(), ()> {
    let mut out = stdout();
    let _ = disable_raw_mode();
    let _ = out.execute(LeaveAlternateScreen);
    let _ = out.execute(crossterm::cursor::Show);
    let _ = out.execute(crossterm::event::DisableMouseCapture);
    Ok(())
}
