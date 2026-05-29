use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{fs, io, panic, time::Duration};

mod app;
mod ast_diff;
mod export;
mod text_diff;
mod ui;

use app::App;

#[derive(Parser)]
#[command(name = "astdiff", about = "C++ diff visualizer with AST support")]
struct Cli {
    /// Left file (or single file for AST browse mode)
    left: String,
    /// Right file (optional — omit for single-file AST browse)
    right: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let left_content = fs::read_to_string(&cli.left)?;

    let app = if let Some(ref right_path) = cli.right {
        let right_content = fs::read_to_string(right_path)?;
        App::new_diff(left_content, right_content, cli.left.clone(), right_path.clone())
    } else {
        App::new_single(left_content, cli.left.clone())
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
