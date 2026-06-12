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
mod ast_cache;
mod ast_diff;
mod config;
mod export;
mod input;
mod logger;
mod project;
mod search;
mod syntax;
mod text_diff;
mod tracer;
mod ui;

use app::App;
use config::OpenIn;

#[derive(Parser)]
#[command(name = "mastdiff", about = "C++ diff visualizer with AST + project search")]
struct Cli {
    /// Path to a C++ file, a second file to diff against, or a project directory
    #[arg(required_unless_present = "search")]
    left: Option<String>,
    /// Second file to diff (optional)
    right: Option<String>,
    /// Write a Chrome trace to this path on exit (requires --features bench build)
    #[arg(long)]
    trace: Option<String>,

    /// Run a headless search over PATH (default ".") and print results — no TUI.
    /// Accepts the same queries as the interactive search: shorthand prefixes
    /// (fn:, call:, var:, class:, …), raw tree-sitter S-expressions, or plain text.
    #[arg(long, value_name = "QUERY")]
    search: Option<String>,
    /// Print headless search results as JSON Lines (one object per line)
    #[arg(long, requires = "search")]
    json: bool,
    /// Comma-separated include globs for headless search (e.g. "src/**,*.hpp")
    #[arg(long, requires = "search", value_name = "GLOBS")]
    include: Option<String>,
    /// Comma-separated exclude globs for headless search (e.g. "tests/**,vendor/**")
    #[arg(long, requires = "search", value_name = "GLOBS")]
    exclude: Option<String>,
    /// Treat the query's text filter as a regular expression
    #[arg(long, requires = "search")]
    regex: bool,
    /// Match case-sensitively (default is case-insensitive)
    #[arg(long, requires = "search")]
    case_sensitive: bool,
}

/// Headless search mode (`--search`): enumerate files, run one search pass,
/// print results to stdout, exit. Powers editor integrations such as the
/// VS Code extension in editors/vscode.
fn headless_search(root: &str, raw_query: &str, cli: &Cli) -> Result<()> {
    use std::io::Write;

    let root_path = Path::new(root);
    let files: Vec<String> = if root_path.is_dir() {
        let data = project::load(root_path)?;
        let include = search::FileFilter::parse(cli.include.as_deref().unwrap_or(""));
        let exclude = search::FileFilter::parse(cli.exclude.as_deref().unwrap_or(""));
        let project_root = root.trim_end_matches('/').to_string();
        data.files
            .iter()
            .map(|tu| tu.file_path.clone())
            .filter(|path| {
                // Same semantics as the TUI search: globs match the relative
                // path inside the project root so `src/**` works as expected.
                let rel = path
                    .strip_prefix(&format!("{}/", project_root))
                    .or_else(|| path.strip_prefix(&project_root))
                    .unwrap_or(path.as_str());
                let pass_include = include.is_empty() || include.matches(rel) || include.matches(path);
                let pass_exclude = exclude.is_empty() || (!exclude.matches(rel) && !exclude.matches(path));
                pass_include && pass_exclude
            })
            .collect()
    } else {
        vec![root.to_string()]
    };
    log::info!("headless search: {:?} over {} files", raw_query, files.len());

    let mut query = search::parse_query(raw_query);
    query.use_regex = cli.regex;
    query.case_sensitive = cli.case_sensitive;

    let results = search::search_project(&files, &query);

    // The stored snippet is a short window around the match (sized for the
    // TUI); editors want the whole line, so re-read it from the hit files.
    let mut file_lines: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for r in &results {
        let lines = file_lines.entry(r.file_path.clone()).or_insert_with(|| {
            fs::read_to_string(&r.file_path)
                .map(|src| src.lines().map(str::to_string).collect())
                .unwrap_or_default()
        });
        let full_line = lines
            .get(r.line)
            .map(|l| l.trim().to_string())
            .unwrap_or_else(|| r.snippet.clone());
        if cli.json {
            // Lines and columns are 1-based in the output, like grep/vimgrep.
            let obj = serde_json::json!({
                "file": r.file_path,
                "line": r.line + 1,
                "col": r.col + 1,
                "text": full_line,
                "kind": r.node_kind,
                "capture": r.capture_name,
            });
            writeln!(out, "{}", obj)?;
        } else {
            writeln!(out, "{}:{}:{}: {}", r.file_path, r.line + 1, r.col + 1, full_line)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    tracer::init();

    // Load config first so we know the desired log level before logging anything.
    let cfg = config::Config::load();
    let _ = logger::init("mastdiff.log", cfg.log_level.to_level_filter());
    log::info!(
        "mastdiff starting — editor={} log_level={}",
        cfg.open_in.label(),
        cfg.log_level.label()
    );

    let cli = Cli::parse();

    // Headless search mode — print results and exit, no TUI.
    if let Some(ref raw_query) = cli.search {
        let root = cli.left.as_deref().unwrap_or(".");
        return headless_search(root, raw_query, &cli);
    }

    let left = cli.left.clone().expect("clap enforces left unless --search");
    let left_path = Path::new(&left);
    log::debug!("cli args: left={:?} right={:?}", left, cli.right);

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
        App::new_project_loading(left.clone(), cfg.clone())
    } else if let Some(ref right_path) = cli.right {
        // Two-file diff mode
        let left_src = fs::read_to_string(&left)?;
        let right = fs::read_to_string(right_path)?;
        log::info!("diff mode: {:?} vs {:?}", left, right_path);
        App::new_diff(left_src, right, left.clone(), right_path.clone(), cfg.clone())
    } else {
        // Single-file mode — treated as a one-file project, opens directly in file view
        let content = fs::read_to_string(&left)?;
        log::info!("single-file mode: {:?}", left);
        App::new_single_file(left.clone(), content, cfg.clone())
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
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }
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

    if let Some(ref path) = cli.trace {
        tracer::save(path)?;
    }

    Ok(())
}
