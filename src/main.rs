use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{fs, io, panic, path::Path, time::Duration};

mod app;
mod ast_diff;
mod config;
mod export;
mod input;
mod logger;
mod project;
mod search;
mod text_diff;
mod ui;

use app::App;
use config::OpenIn;

#[derive(Parser)]
#[command(name = "astdiff", about = "C++ diff visualizer with AST + project search")]
struct Cli {
    /// Path to a C++ file, a second file to diff against, or a project directory
    left: String,
    /// Second file to diff (optional)
    right: Option<String>,
}

fn main() -> Result<()> {

    // Load config first so we know the desired log level before logging anything.
    let cfg = config::Config::load();
    let _ = logger::init("astdiff.log", cfg.log_level.to_level_filter());
    log::info!(
        "astdiff starting — editor={} log_level={}",
        cfg.open_in.label(),
        cfg.log_level.label()
    );

    let cli = Cli::parse();
    let left_path = Path::new(&cli.left);
    log::debug!("cli args: left={:?} right={:?}", cli.left, cli.right);

    let app = if left_path.is_dir() {
        // Project browser mode
        let files = project::load(left_path)?;
        if files.is_empty() {
            eprintln!("No C++ source files found in {:?}", left_path);
            std::process::exit(1);
        }
        log::info!("project mode: {} translation units in {:?}", files.len(), left_path);
        App::new_project(files, cli.left.clone(), cfg.clone())
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

    log::debug!("initialising terminal (raw mode + alternate screen)");
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {}", info);
        log::logger().flush();
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = app;

    loop {
        let height = terminal.size()?.height as usize;
        terminal.draw(|f| ui::render(f, &mut app, height))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
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
            }
        }

        if app.should_quit {
            break;
        }
    }

    log::debug!("restoring terminal");
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    log::info!("astdiff exiting cleanly");

    Ok(())
}
