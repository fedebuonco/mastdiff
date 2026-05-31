use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{fs, io, panic, path::Path, sync::mpsc, time::Duration};

mod app;
mod ast_diff;
mod config;
mod export;
mod input;
mod logger;
mod project;
mod search;
mod syntax;
mod text_diff;
mod ui;

use app::App;
use config::OpenIn;

#[derive(Parser)]
#[command(name = "mastdiff", about = "C++ diff visualizer with AST + project search")]
struct Cli {
    /// Path to a C++ file, a second file to diff against, or a project directory
    left: String,
    /// Second file to diff (optional)
    right: Option<String>,
}

fn main() -> Result<()> {

    // Load config first so we know the desired log level before logging anything.
    let cfg = config::Config::load();
    let _ = logger::init("mastdiff.log", cfg.log_level.to_level_filter());
    log::info!(
        "mastdiff starting — editor={} log_level={}",
        cfg.open_in.label(),
        cfg.log_level.label()
    );

    let cli = Cli::parse();
    let left_path = Path::new(&cli.left);
    log::debug!("cli args: left={:?} right={:?}", cli.left, cli.right);

    // For project mode we spawn a background loader and stream results to the
    // UI.  For the other modes we still build the App synchronously.
    let mut project_rx: Option<mpsc::Receiver<project::LoadMsg>> = None;

    let app = if left_path.is_dir() {
        // Project browser mode — start TUI immediately, load in background
        log::info!("project mode (streaming): {:?}", left_path);
        let (tx, rx) = mpsc::channel::<project::LoadMsg>();
        let dir = left_path.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) = project::load_streaming(&dir, &tx) {
                log::error!("project loader error: {}", e);
            }
        });
        project_rx = Some(rx);
        App::new_project_loading(cli.left.clone(), cfg.clone())
    } else if let Some(ref right_path) = cli.right {
        // Two-file diff mode
        let left = fs::read_to_string(&cli.left)?;
        let right = fs::read_to_string(right_path)?;
        log::info!("diff mode: {:?} vs {:?}", cli.left, right_path);
        App::new_diff(left, right, cli.left.clone(), right_path.clone(), cfg.clone())
    } else {
        // Single-file AST browse mode
        let content = fs::read_to_string(&cli.left)?;
        log::info!("single-file mode: {:?}", cli.left);
        App::new_single(content, cli.left.clone(), cfg.clone())
    };

    log::debug!("initialising terminal (raw mode + alternate screen + mouse)");
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {}", info);
        log::logger().flush();
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen
        );
        hook(info);
    }));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = app;

    // Drain any terminal events that leaked from the shell (e.g. the Enter
    // keypress used to launch mastdiff — which would otherwise immediately
    // open the first file in the project browser).
    while event::poll(Duration::from_millis(0))? {
        let _ = event::read()?;
    }

    loop {
        // Advance the spinner animation counter (≈20 ticks/s at 50 ms poll).
        app.spinner_tick = app.spinner_tick.wrapping_add(1);

        // Drain pending search result batches from the background search thread.
        app.tick_search();

        // Drain messages from the background project loader.
        if let Some(ref rx) = project_rx {
            let mut finished = false;
            while let Ok(msg) = rx.try_recv() {
                if matches!(msg, project::LoadMsg::Finished(_)) {
                    finished = true;
                }
                app.handle_load_msg(msg);
                if finished { break; }
            }
            if finished { project_rx = None; }
        }

        let height = terminal.size()?.height as usize;
        terminal.draw(|f| ui::render(f, &mut app, height))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                }
                Event::Key(key) => {
                if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE {
                    break;
                }
                app.handle_key(key);

                // Handle external editor open (set by Ctrl+o in any mode)
                if let Some((file, line, col)) = app.pending_open.take() {
                    match app.config.open_in {
                        OpenIn::Vim => {
                            // Hand off terminal to vim, restore TUI on exit
                            disable_raw_mode()?;
                            execute!(io::stdout(), LeaveAlternateScreen)?;
                            log::info!("opening vim: {} +{}", file, line + 1);
                            let _ = std::process::Command::new("vim")
                                .arg(format!("+{}", line + 1))
                                .arg(&file)
                                .status();
                            enable_raw_mode()?;
                            execute!(io::stdout(), EnterAlternateScreen)?;
                            terminal.clear()?;
                        }
                        OpenIn::VSCode => {
                            // Spawn in background — vscode opens its own window
                            let target = format!("{}:{}:{}", file, line + 1, col + 1);
                            log::info!("opening vscode: {}", target);
                            let _ = std::process::Command::new("code")
                                .arg("--goto")
                                .arg(&target)
                                .spawn();
                        }
                    }
                }
                } // end Event::Key arm
                _ => {}
            } // end match event::read()
        }

        if app.should_quit {
            break;
        }
    }

    log::debug!("restoring terminal");
    disable_raw_mode()?;
    execute!(
        io::stdout(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    log::info!("mastdiff exiting cleanly");

    Ok(())
}
