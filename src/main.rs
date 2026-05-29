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
mod export;
mod input;
mod project;
mod search;
mod text_diff;
mod ui;

use app::App;

#[derive(Parser)]
#[command(name = "astdiff", about = "C++ diff visualizer with AST + project search")]
struct Cli {
    /// Path to a C++ file, a second file to diff against, or a project directory
    left: String,
    /// Second file to diff (optional)
    right: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let left_path = Path::new(&cli.left);

    let app = if left_path.is_dir() {
        // Project browser mode
        let files = project::load(left_path)?;
        if files.is_empty() {
            eprintln!("No C++ source files found in {:?}", left_path);
            std::process::exit(1);
        }
        App::new_project(files, cli.left.clone())
    } else if let Some(ref right_path) = cli.right {
        // Two-file diff mode
        let left = fs::read_to_string(&cli.left)?;
        let right = fs::read_to_string(right_path)?;
        App::new_diff(left, right, cli.left.clone(), right_path.clone())
    } else {
        // Single-file AST browse mode
        let content = fs::read_to_string(&cli.left)?;
        App::new_single(content, cli.left.clone())
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
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
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    Ok(())
}
