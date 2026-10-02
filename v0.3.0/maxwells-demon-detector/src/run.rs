//! Run loops: the interactive TUI and headless text frames.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, style, terminal};

use crate::app::App;
use crate::cli::{Cli, Mode};
use crate::render::{render, View};
use crate::screen::Screen;
use crate::source::{self, Recorder, Sink, Source};
use crate::Result;

const MIN_TICK_MS: u64 = 20;
const MAX_TICK_MS: u64 = 2_000;

/// Entry point used by the binary.
pub fn run(cli: &Cli) -> Result<()> {
    if cli.mode == Mode::ListInterfaces {
        return source::print_interfaces();
    }
    if cli.headless {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        return run_headless(cli, &mut out);
    }
    run_tui(cli)
}

fn make_parts(cli: &Cli) -> Result<(Box<dyn Source>, Sink, App)> {
    if cli.record.is_some() && cli.mode == Mode::Bytes {
        return Err("--record captures packet events; it doesn't apply to --mode bytes".into());
    }
    // Open the input first so a bad --input never leaves an empty recording behind.
    let src = source::build(cli)?;
    let recorder = match &cli.record {
        Some(p) => {
            Some(Recorder::create(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?)
        }
        None => None,
    };
    let app = App::new(
        source::lane_set(cli.mode),
        cli.window as usize,
        cli.max_alerts,
    );
    Ok((src, Sink::new(recorder), app))
}

/// Headless mode: advance the clock tick by tick and print frames as plain text.
///
/// With `--fast` the virtual clock jumps a whole tick per step (no sleeping), so a
/// minute of demo renders in well under a second: ideal for tests and screenshots.
pub fn run_headless<W: Write>(cli: &Cli, out: &mut W) -> Result<()> {
    let (mut src, mut sink, mut app) = make_parts(cli)?;
    if cli.fast && src.realtime() {
        return Err(
            "--fast needs demo or file input (stdin and live capture run in real time)".into(),
        );
    }
    let endless = cli.mode == Mode::Demo || cli.loop_input;
    if endless && cli.frames.is_none() && cli.print_every == 0 {
        return Err(
            "this input never ends: add --frames N (e.g. --frames 480 = 60 s at 125 ms)".into(),
        );
    }
    let tick = Duration::from_millis(cli.tick_ms);
    let max_ticks = cli.frames.unwrap_or(u64::MAX);
    let start = Instant::now();
    let mut next_deadline = start;
    let mut vclock = Duration::ZERO;
    let mut ticks = 0u64;
    let mut ticks_after_end = 0u32;
    loop {
        if cli.fast {
            vclock += tick;
        } else {
            next_deadline += tick;
            let now = Instant::now();
            if next_deadline > now {
                std::thread::sleep(next_deadline - now);
            }
            vclock = start.elapsed();
        }
        src.poll(vclock, &mut sink)?;
        for o in sink.obs.drain(..) {
            app.ingest(&o);
        }
        app.finish_tick(vclock.as_secs_f64());
        ticks += 1;

        let status = src.status();
        if status.ended {
            ticks_after_end += 1;
        }
        // After the input ends, run a couple more ticks so the last data is drawn.
        let done = ticks >= max_ticks || ticks_after_end > 2;
        let periodic = cli.print_every > 0 && ticks.is_multiple_of(cli.print_every);
        if periodic || done {
            let view = View {
                status: &status,
                tick_ms: cli.tick_ms,
                paused: false,
                show_legend: true,
                recording: sink.recording(),
                record_error: sink.record_error.as_deref(),
                hints: false,
            };
            let frame = render(&app, &view, cli.width, cli.height);
            if cli.print_every > 0 {
                writeln!(out, "--- tick {ticks}  t={:.2}s ---", vclock.as_secs_f64())?;
            }
            out.write_all(frame.to_text().as_bytes())?;
            out.flush()?;
        }
        if done {
            break;
        }
    }
    if !app.alerts.is_empty() {
        writeln!(out, "alerts (newest first):")?;
        for a in &app.alerts {
            let secs = a.t.max(0.0) as u64;
            writeln!(
                out,
                "  {:02}:{:02} {:<6} {}",
                secs / 60,
                secs % 60,
                a.lane,
                a.msg
            )?;
        }
    }
    sink.flush()?;
    Ok(())
}

/// Puts the terminal into full-screen raw mode; restores it on drop (also on
/// error paths and panics, via the hook installed in `run_tui`).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        if let Err(e) = execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::DisableLineWrap,
            terminal::Clear(terminal::ClearType::All)
        ) {
            restore_terminal();
            return Err(e);
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let mut out = io::stdout();
    let _ = execute!(
        out,
        style::ResetColor,
        terminal::EnableLineWrap,
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

fn run_tui(cli: &Cli) -> Result<()> {
    if !io::stdout().is_terminal() {
        return Err("stdout is not a terminal. Run mdd in a terminal window, or add --headless to print frames as text".into());
    }
    let (mut src, mut sink, mut app) = make_parts(cli)?;
    let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let mut screen = Screen::new(!cli.no_color && !no_color_env);

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
    let guard = TerminalGuard::enter()?;
    let result = tui_loop(cli, src.as_mut(), &mut sink, &mut app, &mut screen);
    drop(guard);
    let _ = std::panic::take_hook(); // back to the default hook
    sink.flush()?;
    result
}

fn tui_loop(
    cli: &Cli,
    src: &mut dyn Source,
    sink: &mut Sink,
    app: &mut App,
    screen: &mut Screen,
) -> Result<()> {
    let mut out = io::stdout();
    let mut tick_ms = cli.tick_ms.clamp(MIN_TICK_MS, MAX_TICK_MS);
    let mut paused = false;
    let mut show_legend = true;
    let mut redraw = true;
    let mut vclock = Duration::ZERO;
    let mut last = Instant::now();
    let mut next_tick = last + Duration::from_millis(tick_ms);

    loop {
        // 1. Keyboard and resize events (short timeout keeps the flow smooth).
        let wait = next_tick
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(25));
        if event::poll(wait)? {
            loop {
                match event::read()? {
                    // Windows reports key releases too; act on presses only.
                    Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(())
                        }
                        KeyCode::Char(' ') | KeyCode::Char('p') => {
                            paused = !paused;
                            redraw = true;
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            tick_ms = (tick_ms * 4 / 5).max(MIN_TICK_MS);
                            redraw = true;
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            tick_ms = (tick_ms * 5 / 4 + 1).min(MAX_TICK_MS);
                            redraw = true;
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            screen.color = !screen.color;
                            screen.invalidate();
                            redraw = true;
                        }
                        KeyCode::Char('l') | KeyCode::Char('L') => {
                            show_legend = !show_legend;
                            redraw = true;
                        }
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        screen.invalidate();
                        redraw = true;
                    }
                    _ => {}
                }
                if !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        // 2. Clocks. File/demo sources pause for real; live sources keep being
        //    analysed while the display is frozen (so nothing is dropped).
        let now = Instant::now();
        let running = !paused || src.realtime();
        if running {
            vclock += now - last;
        }
        last = now;

        // 3. Pull data.
        if running {
            src.poll(vclock, sink)?;
            for o in sink.obs.drain(..) {
                app.ingest(&o);
            }
        }

        // 4. Close a column every tick.
        if now >= next_tick {
            if running {
                app.finish_tick(vclock.as_secs_f64());
                if !paused {
                    redraw = true;
                }
            }
            next_tick += Duration::from_millis(tick_ms);
            if next_tick < now {
                next_tick = now + Duration::from_millis(tick_ms);
            }
        }

        // 5. Draw (diffed, synchronized).
        if redraw {
            let (w, h) = terminal::size()?;
            let status = src.status();
            let view = View {
                status: &status,
                tick_ms,
                paused,
                show_legend,
                recording: sink.recording(),
                record_error: sink.record_error.as_deref(),
                hints: true,
            };
            let frame = render(app, &view, w, h);
            screen.present(&mut out, &frame)?;
            redraw = false;
        }
    }
}
