mod cli;
mod event;
mod model;
mod render;
mod source;
mod stats;

use std::error::Error;
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{poll, read, Event as CEvent, KeyCode};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use cli::{Cli, Mode};
use model::AppState;
use source::{build_source, print_interfaces};

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, Box<dyn Error>> {
        terminal::enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn maybe_quit() -> Result<bool, Box<dyn Error>> {
    while poll(Duration::from_millis(0))? {
        if let CEvent::Key(key) = read()? {
            if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.mode == Mode::ListInterfaces {
        print_interfaces()?;
        return Ok(());
    }

    let mut source = build_source(&cli)?;
    let _guard = TerminalGuard::enter()?;

    let (term_w, _) = terminal::size()?;
    let width = cli
        .width
        .unwrap_or_else(|| term_w.saturating_sub(24).max(32) as usize);

    let mut state = AppState::new(width, cli.tick_ms, cli.mode, cli.max_alerts);
    let tick = Duration::from_millis(cli.tick_ms);

    let mut last_tick = Instant::now();
    let mut stdout = stdout();

    loop {
        if maybe_quit()? {
            break;
        }

        let now = Instant::now();
        let elapsed = now.saturating_duration_since(last_tick);
        let budget = elapsed.min(tick.saturating_mul(4));
        let events = source.poll(budget)?;
        for ev in events {
            state.ingest(ev);
        }

        if elapsed >= tick {
            state.finalize_tick();
            render::draw(&mut stdout, &state)?;
            stdout.flush()?;
            last_tick = now;
        } else {
            std::thread::sleep(tick - elapsed);
        }
    }

    Ok(())
}
