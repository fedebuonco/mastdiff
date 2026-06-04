use std::collections::HashSet;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::ast_diff::{
    compute_ast_diff, filter_rows, parse_single, row_has_children, visible_rows,
    AstDiffResult, AstLine,
};
use crate::config::Config;
use crate::export;
use crate::input::TextInput;
use crate::project::{CmakeTarget, TranslationUnit};
use crate::search::{parse_query, search_project_streaming, FileFilter, SearchQuery, SearchResult};
use crate::syntax::SyntaxSpan;
use crate::text_diff::{compute_diff, context_view, hunk_positions, DiffLine};

/// Display rows consumed by each search result in the results list.
/// Used by both the renderer and the mouse click handler.
pub const ROWS_PER_RESULT: usize = 2;

// ── Modes & enums ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    TextDiff,
    AstDiff,
    ProjectBrowser,
    Search,
    Help,
}

/// Which view the project browser is showing.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectView {
    /// Flat list of source files (default).
    Tus,
    /// Flat list of source files only (.cpp/.cc/.cxx/.C).
    Sources,
    /// Flat list of header files only (.h/.hpp/.hxx/.H).
    Headers,
    /// CMake targets parsed from CMakeLists.txt.
    Cmake,
}

/// One row in the project browser display list.
#[derive(Clone, Debug)]
pub enum ProjectRow {
    /// A source file (index into `project_files`).
    Source(usize),
    /// A loose header file (Headers view).
    LooseHeader(usize),
    /// A CMake target (index into `cmake_targets`).
    CmakeTarget(usize),
    /// A source listed under a CMake target (Cmake view, expanded).
    CmakeSource { #[allow(dead_code)] tgt_idx: usize, path: String },
}

/// Which input box has keyboard focus in Search mode.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchFocus {
    Query,
    Include,
    Exclude,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AstVizMode {
    Tree,
    Timeline,
}

impl AstVizMode {
    pub fn label(&self) -> &str {
        match self {
            Self::Tree     => "Tree",
            Self::Timeline => "Timeline",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Tree     => Self::Timeline,
            Self::Timeline => Self::Tree,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AstFilter {
    All,
    Functions,
    Classes,
    Variables,
}

impl AstFilter {
    pub fn label(&self) -> &str {
        match self {
            Self::All => "all",
            Self::Functions => "functions",
            Self::Classes => "classes",
            Self::Variables => "variables",
        }
    }

    pub fn keyword(&self) -> &str {
        match self {
            Self::All => "",
            Self::Functions => "function",
            Self::Classes => "class",
            Self::Variables => "declaration",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Functions,
            Self::Functions => Self::Classes,
            Self::Classes => Self::Variables,
            Self::Variables => Self::All,
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────

pub struct App {
    // ---- identity
    pub left_path: String,
    pub right_path: String,
    pub left_lines: Vec<String>,
    pub right_lines: Vec<String>,

    // ---- text diff state
    pub diff_lines: Vec<DiffLine>,
    pub scroll: usize,
    pub cursor: usize,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,

    // ---- diff display options
    pub ignore_ws: bool,
    pub inline_diff: bool,
    pub context_only: bool,
    pub context_lines: usize,
    pub context_indices: Vec<usize>, // valid when context_only=true
    pub unified_view: bool,
    pub hunk_pos: Vec<usize>, // hunk start rows
    #[allow(dead_code)]
    pub current_hunk: usize,

    // ---- AST diff state
    pub mode: AppMode,
    pub ast_result: Option<AstDiffResult>,
    pub ast_scroll: usize,
    pub ast_cursor: usize,  // cursor in the *visible* list
    pub ast_collapsed: HashSet<usize>, // raw row indices that are collapsed
    pub ast_filter: AstFilter,
    pub ast_visible: Vec<usize>, // precomputed visible row indices
    pub ast_filter_rows: Vec<usize>, // after filter applied

    // ---- project browser
    pub project_files: Vec<TranslationUnit>,
    pub cmake_targets: Vec<CmakeTarget>,
    pub project_view: ProjectView,
    pub project_display: Vec<ProjectRow>,  // flat display list for current view+filter
    pub project_expanded: HashSet<usize>,  // source/cmake indices that are expanded
    pub project_cursor: usize,             // index into project_display
    pub project_filter: TextInput,
    pub project_filter_active: bool,

    // ---- search
    /// Path of the file currently shown in the source/AST panes when browsing
    /// a file directly (no search results). Shown as the source-pane title.
    pub search_open_file: String,
    pub search_input: TextInput,
    pub search_include: TextInput,     // "files to include" glob patterns (comma-separated)
    pub search_exclude: TextInput,     // "files to exclude" glob patterns (comma-separated)
    pub search_focus: SearchFocus,     // which input box has keyboard focus
    pub search_use_regex: bool,        // Alt+R — treat query/filter as a regex
    pub search_case_sensitive: bool,   // Alt+C — case-sensitive match (default: insensitive)
    pub search_query: SearchQuery,
    pub search_grep_mode: bool,        // true = plain grep, false = ts-query/shorthand
    pub search_results: Vec<SearchResult>,
    pub search_selected: usize,
    pub search_scroll: usize,         // top of the results list
    pub search_source_lines: Vec<String>,
    pub search_source_tokens: Vec<Vec<SyntaxSpan>>, // syntax-highlight spans, indexed by line
    pub search_source_scroll: usize,
    pub search_source_highlight: usize, // line to highlight (abs line in file)
    pub search_ast_nodes: Vec<AstLine>,
    pub search_ast_scroll: usize,
    pub search_ast_highlight: usize,  // raw index into search_ast_nodes (result match)
    pub search_ast_collapsed: HashSet<usize>, // raw indices of collapsed nodes
    pub search_ast_visible: Vec<usize>,       // visible raw indices (after collapsing)
    pub search_ast_cursor: usize,             // cursor in search_ast_visible (interactive nav)
    pub search_ast_viz: AstVizMode,           // current AST visualization mode
    pub search_timeline_base: Option<usize>,  // raw node index that is the zoom base in timeline
    search_timeline_last_click: Option<(std::time::Instant, u16, u16)>, // for double-click detection
    pub search_prev_mode: AppMode,    // mode to return to on Esc

    // ---- help overlay
    pub help_prev_mode: AppMode,      // mode to return to when closing help

    // ---- UI
    pub should_quit: bool,
    pub status_msg: String,

    // ---- config
    pub config: Config,
    /// Set to Some(file, line, col) when the user presses Ctrl+o.
    /// The main event loop drains this and opens the file in the configured editor.
    pub pending_open: Option<(String, usize, usize)>,

    // ---- mouse hit-test areas (updated each frame by the search view renderer)
    pub search_results_area: Rect,
    pub search_source_area:  Rect,
    pub search_ast_area:     Rect,

    // ---- source pane text selection (drag-to-select → Ctrl+C or auto-copy on release)
    /// Anchor position of the current drag selection: (line, char_col) in file coords
    pub search_sel_anchor: Option<(usize, usize)>,
    /// Normalized selection range: (start, end) both in (line, char_col) file coords.
    /// start <= end always.
    pub search_sel: Option<((usize, usize), (usize, usize))>,

    // ---- streaming project load
    /// True while the background loader thread is still running.
    pub loading: bool,
    /// Monotonically incrementing counter driven by the main loop (one tick ≈ 50 ms).
    /// Used by the renderer to animate the spinner.
    pub spinner_tick: u64,

    // ---- streaming search
    /// True while a background search thread is running.
    pub search_running: bool,
    /// Receives result batches from the background search thread.
    pub search_rx: Option<mpsc::Receiver<Vec<SearchResult>>>,
    /// Set to `true` to ask a running search to stop early.
    pub search_cancel: Arc<AtomicBool>,
    /// When set, fire a new search after this instant + 300 ms debounce.
    pub search_debounce: Option<std::time::Instant>,
}

impl App {
    // ── Constructors ──────────────────────────────────────────────────────

    pub fn new_diff(left: String, right: String, left_path: String, right_path: String, config: Config) -> Self {
        let left_lines: Vec<String> = left.lines().map(|l| l.to_string()).collect();
        let right_lines: Vec<String> = right.lines().map(|l| l.to_string()).collect();
        let diff_lines = compute_diff(&left, &right, false);
        let hunk_pos = hunk_positions(&diff_lines);

        Self {
            left_path,
            right_path,
            left_lines,
            right_lines,
            diff_lines,
            scroll: 0,
            cursor: 0,
            selection_start: None,
            selection_end: None,
            ignore_ws: false,
            inline_diff: true,
            context_only: false,
            context_lines: 3,
            context_indices: vec![],
            unified_view: false,
            hunk_pos,
            current_hunk: 0,
            mode: AppMode::TextDiff,
            ast_result: None,
            ast_scroll: 0,
            ast_cursor: 0,
            ast_collapsed: HashSet::new(),
            ast_filter: AstFilter::All,
            ast_visible: vec![],
            ast_filter_rows: vec![],
            project_files: vec![],
            cmake_targets: vec![],
            project_view: ProjectView::Tus,
            project_display: vec![],
            project_expanded: HashSet::new(),
            project_cursor: 0,
            project_filter: TextInput::new(),
            project_filter_active: false,
            search_open_file: String::new(),
            search_input: TextInput::new(),
            search_include: TextInput::new(),
            search_exclude: TextInput::new(),
            search_focus: SearchFocus::Query,
            search_use_regex: false,
            search_case_sensitive: false,
            search_query: parse_query(""),
            search_grep_mode: false,
            search_results: vec![],
            search_selected: 0,
            search_scroll: 0,
            search_source_lines: vec![],
            search_source_tokens: vec![],
            search_source_scroll: 0,
            search_source_highlight: 0,
            search_ast_nodes: vec![],
            search_ast_scroll: 0,
            search_ast_highlight: usize::MAX,
            search_ast_collapsed: HashSet::new(),
            search_ast_visible: vec![],
            search_ast_cursor: 0,
            search_ast_viz: AstVizMode::Tree,
            search_timeline_base: None,
            search_timeline_last_click: None,
            search_prev_mode: AppMode::TextDiff,
            help_prev_mode: AppMode::TextDiff,
            should_quit: false,
            status_msg: Self::text_diff_hint(false),
            config,
            pending_open: None,
            search_results_area: Rect::default(),
            search_source_area:  Rect::default(),
            search_ast_area:     Rect::default(),
            search_sel_anchor: None,
            search_sel: None,
            loading: false,
            spinner_tick: 0,
            search_running: false,
            search_rx: None,
            search_cancel: Arc::new(AtomicBool::new(false)),
            search_debounce: None,
        }
    }

    /// Open a single file as a mini one-file project: shows the project browser
    /// with that file listed and immediately opens it in the search/file view.
    pub fn new_single_file(path: String, content: String, config: Config) -> Self {
        use crate::project::TranslationUnit;
        use std::path::Path as StdPath;

        let file_size = content.len() as u64;
        let is_header = matches!(
            StdPath::new(&path).extension().and_then(|e| e.to_str()),
            Some("h") | Some("hpp") | Some("hxx") | Some("H")
        );
        let tu = TranslationUnit {
            file_path: path.clone(),
            file_size,
            is_header,
        };
        // Use the file's parent directory as the "project root" (for relative paths in search).
        let project_root = StdPath::new(&path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();

        let mut app = Self::new_project_loading(project_root, config);
        // Immediately finish loading with just this one file.
        app.finish_project_load(crate::project::ProjectData {
            files: vec![tu],
            cmake_targets: vec![],
        });
        // Pre-open the file in the search/file view.
        app.open_project_file(path, content);
        app
    }

    /// Synchronous constructor — kept for integration tests and any callers that
    /// have the full `ProjectData` up-front. In the binary, project mode now
    /// uses [`new_project_loading`] with a streaming background loader instead.
    #[allow(dead_code)]
    pub fn new_project(data: crate::project::ProjectData, dir_path: String, config: Config) -> Self {
        let mut app = Self::new_project_loading(dir_path, config);
        app.finish_project_load(data);
        app
    }

    /// Start the project browser immediately with an empty file list.
    /// The caller is expected to populate it via [`handle_load_msg`] /
    /// [`finish_project_load`] once the background loader delivers data.
    pub fn new_project_loading(dir_path: String, config: Config) -> Self {
        Self {
            left_path: dir_path.clone(),
            right_path: dir_path,
            left_lines: vec![],
            right_lines: vec![],
            diff_lines: vec![],
            scroll: 0,
            cursor: 0,
            selection_start: None,
            selection_end: None,
            ignore_ws: false,
            inline_diff: false,
            context_only: false,
            context_lines: 3,
            context_indices: vec![],
            unified_view: false,
            hunk_pos: vec![],
            current_hunk: 0,
            mode: AppMode::ProjectBrowser,
            ast_result: None,
            ast_scroll: 0,
            ast_cursor: 0,
            ast_collapsed: HashSet::new(),
            ast_filter: AstFilter::All,
            ast_visible: vec![],
            ast_filter_rows: vec![],
            project_files: vec![],
            cmake_targets: vec![],
            project_view: ProjectView::Tus,
            project_display: vec![],
            project_expanded: HashSet::new(),
            project_cursor: 0,
            project_filter: TextInput::new(),
            project_filter_active: false,
            search_open_file: String::new(),
            search_input: TextInput::new(),
            search_include: TextInput::new(),
            search_exclude: TextInput::new(),
            search_focus: SearchFocus::Query,
            search_use_regex: false,
            search_case_sensitive: false,
            search_query: parse_query(""),
            search_grep_mode: false,
            search_results: vec![],
            search_selected: 0,
            search_scroll: 0,
            search_source_lines: vec![],
            search_source_tokens: vec![],
            search_source_scroll: 0,
            search_source_highlight: 0,
            search_ast_nodes: vec![],
            search_ast_scroll: 0,
            search_ast_highlight: usize::MAX,
            search_ast_collapsed: HashSet::new(),
            search_ast_visible: vec![],
            search_ast_cursor: 0,
            search_ast_viz: AstVizMode::Tree,
            search_timeline_base: None,
            search_timeline_last_click: None,
            search_prev_mode: AppMode::ProjectBrowser,
            help_prev_mode: AppMode::ProjectBrowser,
            should_quit: false,
            status_msg: String::from(" ⚙ Indexing… "),
            config,
            pending_open: None,
            search_results_area: Rect::default(),
            search_source_area:  Rect::default(),
            search_ast_area:     Rect::default(),
            search_sel_anchor: None,
            search_sel: None,
            loading: true,
            spinner_tick: 0,
            search_running: false,
            search_rx: None,
            search_cancel: Arc::new(AtomicBool::new(false)),
            search_debounce: None,
        }
    }

    /// Called when the background loader delivers a batch or the final result.
    pub fn handle_load_msg(&mut self, msg: crate::project::LoadMsg) {
        let _s = crate::tracer::span("app::handle_load_msg");
        use crate::project::LoadMsg;
        match msg {
            LoadMsg::FilesBatch(tus) => {
                self.project_files.extend(tus);
                self.rebuild_project_display();
            }
            LoadMsg::Finished(data) => {
                self.finish_project_load(data);
            }
        }
    }

    /// Replace the (possibly partial) file list with the fully-analysed data
    /// and mark loading as complete.
    pub fn finish_project_load(&mut self, data: crate::project::ProjectData) {
        let _s = crate::tracer::span("app::finish_project_load");
        self.project_files = data.files;
        self.cmake_targets = data.cmake_targets;
        self.loading = false;
        self.rebuild_project_display();
        self.status_msg = String::from(
            " j/k:navigate  Enter:open  s:filter  g:grep  f:ast  q:quit",
        );
    }

    /// Open a file from the project browser (or as a single-file launch) using
    /// the search/file view.  Project state (`project_files`, `project_cursor`,
    /// etc.) is preserved intact so that `Esc` can navigate back.
    fn open_project_file(&mut self, path: String, content: String) {
        let _s = crate::tracer::span("app::open_project_file");
        self.load_file_into_search_pane(&path, &content);
        // Clear any prior search so the results list is empty and only the file
        // content is visible in the right pane.
        self.search_cancel.store(true, Ordering::Relaxed);
        self.search_rx = None;
        self.search_running = false;
        self.search_results.clear();
        self.search_selected = 0;
        self.search_scroll = 0;
        self.search_input.clear();
        self.search_debounce = None;
        self.search_focus = SearchFocus::Query;
        self.search_prev_mode = AppMode::ProjectBrowser;
        self.mode = AppMode::Search;
        self.status_msg = format!(
            " {}  g:grep  f:ast-search  Esc:back  Ctrl+o:editor ",
            short_path(&path),
        );
    }

    /// Load file content into the search source/AST panes without affecting
    /// search results or mode.  Used both by [`open_project_file`] and by the
    /// search result loader.
    fn load_file_into_search_pane(&mut self, path: &str, content: &str) {
        let _s = crate::tracer::span("app::load_file_into_search_pane");
        self.search_open_file = path.to_string();
        self.search_source_lines = content.lines().map(|l| l.to_string()).collect();
        self.search_source_tokens = crate::syntax::highlight(content);
        self.search_ast_nodes = parse_single(content).unwrap_or_default();
        self.search_ast_collapsed = HashSet::new();
        self.search_source_scroll = 0;
        self.search_source_highlight = usize::MAX; // no highlight when just browsing
        self.search_ast_scroll = 0;
        self.search_ast_highlight = usize::MAX;
        self.search_ast_cursor = 0;
        self.rebuild_search_ast_visible();
    }

    /// Recompute `search_ast_visible` from the current nodes + collapsed set.
    pub fn rebuild_search_ast_visible(&mut self) {
        let _s = crate::tracer::span("app::rebuild_search_ast_visible");
        let dummy: Vec<AstLine> = vec![];
        self.search_ast_visible =
            visible_rows(&self.search_ast_nodes, &dummy, &self.search_ast_collapsed);
    }

    // ── Scroll clamping (called by renderer) ──────────────────────────────

    pub fn clamp_scroll(&mut self, view_height: usize) {
        if view_height == 0 {
            return;
        }
        if self.cursor >= self.scroll + view_height {
            self.scroll = self.cursor - view_height + 1;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        }
    }

    // ── Event dispatch ────────────────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) {
        let _s = crate::tracer::span("app::handle_key");
        log::trace!("key {:?} mod={:?} in mode {:?}", key.code, key.modifiers, self.mode);

        // Global: '?' opens help from any mode (except when typing in a search box)
        if key.code == KeyCode::Char('?')
            && key.modifiers == KeyModifiers::NONE
            && self.mode != AppMode::Search
            && self.mode != AppMode::Help
        {
            self.help_prev_mode = self.mode.clone();
            self.mode = AppMode::Help;
            return;
        }

        match self.mode {
            AppMode::TextDiff => self.on_text_diff(key),
            AppMode::AstDiff => self.on_ast_diff(key),
            AppMode::ProjectBrowser => self.on_project_browser(key),
            AppMode::Search => self.on_search(key),
            AppMode::Help => self.on_help(key),
        }
    }

    fn on_help(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.mode = self.help_prev_mode.clone();
            }
            _ => {}
        }
    }

    // ── Text-diff mode ────────────────────────────────────────────────────

    fn total_rows(&self) -> usize {
        if self.context_only {
            self.context_indices.len()
        } else {
            self.diff_lines.len()
        }
    }

    /// Translate a display-row index to a diff_lines index.
    pub fn display_to_diff(&self, display_row: usize) -> usize {
        if self.context_only {
            self.context_indices
                .get(display_row)
                .copied()
                .unwrap_or(display_row)
        } else {
            display_row
        }
    }

    fn on_text_diff(&mut self, key: KeyEvent) {
        let total = self.total_rows();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < total {
                    self.cursor += 1;
                }
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(20);
                self.scroll = self.scroll.saturating_sub(20);
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 20).min(total.saturating_sub(1));
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = total.saturating_sub(1);
            }

            // ── Selection
            KeyCode::Char('s') => {
                self.selection_start = Some(self.cursor);
                self.selection_end = None;
                self.status_msg = format!(
                    " Start: row {}. Move and press 'e' to set end. ",
                    self.cursor + 1
                );
            }
            KeyCode::Char('e') => {
                if let Some(start) = self.selection_start {
                    let end = self.cursor;
                    let (a, b) = if start <= end { (start, end) } else { (end, start) };
                    self.selection_start = Some(a);
                    self.selection_end = Some(b);
                    self.status_msg =
                        format!(" Selection: rows {}-{}. Press Enter for AST diff. ", a + 1, b + 1);
                } else {
                    self.status_msg = String::from(" Press 's' first to mark start. ");
                }
            }
            KeyCode::Enter => {
                if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
                    self.launch_ast_diff(start, end);
                } else {
                    self.status_msg =
                        String::from(" Select a range first ('s' then 'e'), then press Enter. ");
                }
            }
            KeyCode::Esc => {
                self.selection_start = None;
                self.selection_end = None;
                self.status_msg = Self::text_diff_hint(self.ignore_ws);
            }

            // ── Hunk jumping
            KeyCode::Char('n') => self.jump_hunk(1),
            KeyCode::Char('N') => self.jump_hunk(-1),

            // ── Display toggles
            KeyCode::Char('u') => {
                self.unified_view = !self.unified_view;
                self.status_msg = format!(
                    " Unified view: {}. ",
                    if self.unified_view { "ON" } else { "OFF" }
                );
            }
            KeyCode::Char('c') => {
                self.context_only = !self.context_only;
                if self.context_only {
                    self.context_indices = context_view(&self.diff_lines, self.context_lines);
                    self.cursor = 0;
                    self.scroll = 0;
                }
                self.status_msg = format!(
                    " Context-only: {} (±{} lines). ",
                    if self.context_only { "ON" } else { "OFF" },
                    self.context_lines
                );
            }
            KeyCode::Char('w') => {
                self.ignore_ws = !self.ignore_ws;
                let left = self.left_lines.join("\n");
                let right = self.right_lines.join("\n");
                self.diff_lines = compute_diff(&left, &right, self.ignore_ws);
                self.hunk_pos = hunk_positions(&self.diff_lines);
                if self.context_only {
                    self.context_indices = context_view(&self.diff_lines, self.context_lines);
                }
                self.cursor = 0;
                self.scroll = 0;
                self.status_msg = format!(
                    " Ignore whitespace: {}. ",
                    if self.ignore_ws { "ON" } else { "OFF" }
                );
            }
            KeyCode::Char('i') => {
                self.inline_diff = !self.inline_diff;
                self.status_msg = format!(
                    " Inline char diff: {}. ",
                    if self.inline_diff { "ON" } else { "OFF" }
                );
            }

            // ── Open in external editor
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                let diff_idx = self.display_to_diff(self.cursor);
                if let Some(dl) = self.diff_lines.get(diff_idx) {
                    let line = dl.left_lineno.unwrap_or(0);
                    self.pending_open = Some((self.left_path.clone(), line, 0));
                    self.status_msg = format!(
                        " Opening {} +{} in {} ",
                        self.left_path, line + 1, self.config.open_in.label()
                    );
                }
            }

            // ── Export
            KeyCode::Char('x') => {
                let path = "mastdiff_output.patch";
                match export::to_patch(
                    &self.diff_lines,
                    &self.left_path,
                    &self.right_path,
                    path,
                    self.context_lines,
                ) {
                    Ok(_) => self.status_msg = format!(" Exported patch → {} ", path),
                    Err(e) => self.status_msg = format!(" Export failed: {} ", e),
                }
            }
            KeyCode::Char('X') => {
                let path = "mastdiff_output.html";
                match export::to_html(
                    &self.diff_lines,
                    &self.left_path,
                    &self.right_path,
                    path,
                ) {
                    Ok(_) => self.status_msg = format!(" Exported HTML → {} ", path),
                    Err(e) => self.status_msg = format!(" Export failed: {} ", e),
                }
            }

            _ => {}
        }
    }

    fn jump_hunk(&mut self, dir: i64) {
        if self.hunk_pos.is_empty() {
            self.status_msg = String::from(" No hunks. ");
            return;
        }
        let cursor_diff = self.display_to_diff(self.cursor);
        if dir > 0 {
            // next hunk after cursor
            if let Some(&h) = self.hunk_pos.iter().find(|&&h| h > cursor_diff) {
                self.cursor = if self.context_only {
                    self.context_indices
                        .iter()
                        .position(|&ci| ci >= h)
                        .unwrap_or(self.cursor)
                } else {
                    h
                };
            } else {
                self.status_msg = String::from(" No next hunk. ");
            }
        } else {
            // prev hunk before cursor
            if let Some(&h) = self.hunk_pos.iter().rev().find(|&&h| h < cursor_diff) {
                self.cursor = if self.context_only {
                    self.context_indices
                        .iter()
                        .position(|&ci| ci >= h)
                        .unwrap_or(self.cursor)
                } else {
                    h
                };
            } else {
                self.status_msg = String::from(" No previous hunk. ");
            }
        }
    }

    // ── AST-diff mode ─────────────────────────────────────────────────────

    fn on_ast_diff(&mut self, key: KeyEvent) {
        let vis_len = self.ast_filter_rows.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.ast_cursor > 0 {
                    self.ast_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.ast_cursor + 1 < vis_len {
                    self.ast_cursor += 1;
                }
            }
            KeyCode::PageUp => self.ast_cursor = self.ast_cursor.saturating_sub(20),
            KeyCode::PageDown => {
                self.ast_cursor = (self.ast_cursor + 20).min(vis_len.saturating_sub(1))
            }
            KeyCode::Home | KeyCode::Char('g') => self.ast_cursor = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.ast_cursor = vis_len.saturating_sub(1)
            }

            // ── Collapse/expand
            KeyCode::Char(' ') => {
                if let Some(&raw) = self.ast_filter_rows.get(self.ast_cursor) {
                    if let Some(ref result) = self.ast_result {
                        if row_has_children(raw, &result.left_nodes, &result.right_nodes) {
                            if self.ast_collapsed.contains(&raw) {
                                self.ast_collapsed.remove(&raw);
                            } else {
                                self.ast_collapsed.insert(raw);
                            }
                            self.rebuild_ast_visible();
                        }
                    }
                }
            }

            // ── Filter cycling
            KeyCode::Char('f') => {
                self.ast_filter = self.ast_filter.next();
                self.rebuild_ast_visible();
                self.ast_cursor = 0;
                self.ast_scroll = 0;
                self.status_msg =
                    format!(" AST filter: {} | Space:fold  f:cycle  Enter:→source  Esc:back ", self.ast_filter.label());
            }

            // ── Jump to source
            KeyCode::Enter => {
                self.jump_ast_to_source();
            }

            // ── Open in external editor
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                if let (Some(ref result), Some(&raw)) =
                    (&self.ast_result, self.ast_filter_rows.get(self.ast_cursor))
                {
                    let src_row = result
                        .left_nodes
                        .get(raw)
                        .filter(|n| !n.empty)
                        .map(|n| n.source_row)
                        .unwrap_or(0);
                    let abs_line = result.left_source_start + src_row;
                    self.pending_open = Some((self.left_path.clone(), abs_line, 0));
                    self.status_msg = format!(
                        " Opening {} +{} in {} ",
                        self.left_path, abs_line + 1, self.config.open_in.label()
                    );
                }
            }

            // ── Back
            KeyCode::Esc | KeyCode::Char('b') => {
                log::debug!("mode: AstDiff → TextDiff");
                self.mode = AppMode::TextDiff;
                self.ast_scroll = 0;
                self.status_msg = Self::text_diff_hint(self.ignore_ws);
            }

            _ => {}
        }
    }

    fn jump_ast_to_source(&mut self) {
        let Some(ref result) = self.ast_result else {
            return;
        };
        let Some(&raw) = self.ast_filter_rows.get(self.ast_cursor) else {
            return;
        };
        let src_row = result
            .left_nodes
            .get(raw)
            .filter(|n| !n.empty)
            .map(|n| n.source_row)
            .unwrap_or(0);
        let abs_line = result.left_source_start + src_row;

        // Find the diff display row that contains abs_line on the left
        let target = self.diff_lines.iter().position(|dl| {
            dl.left_lineno.map(|l| l == abs_line).unwrap_or(false)
        });
        if let Some(diff_row) = target {
            let display_row = if self.context_only {
                self.context_indices
                    .iter()
                    .position(|&ci| ci >= diff_row)
                    .unwrap_or(diff_row)
            } else {
                diff_row
            };
            self.mode = AppMode::TextDiff;
            self.cursor = display_row;
            self.status_msg = format!(
                " Jumped to line {}. ",
                abs_line + 1
            );
        }
    }

    fn rebuild_ast_visible(&mut self) {
        let _s = crate::tracer::span("app::rebuild_ast_visible");
        if let Some(ref result) = self.ast_result {
            self.ast_visible =
                visible_rows(&result.left_nodes, &result.right_nodes, &self.ast_collapsed);
            // Apply filter on top of visibility
            let keyword = self.ast_filter.keyword();
            self.ast_filter_rows = if keyword.is_empty() {
                self.ast_visible.clone()
            } else {
                // Filter the visible list further by kind keyword
                let left_filtered = filter_rows(&result.left_nodes, keyword);
                let left_set: HashSet<usize> = left_filtered.into_iter().collect();
                self.ast_visible
                    .iter()
                    .copied()
                    .filter(|r| left_set.contains(r))
                    .collect()
            };
        }
    }

    // ── AST diff launch ───────────────────────────────────────────────────

    fn launch_ast_diff(&mut self, disp_start: usize, disp_end: usize) {
        let _s = crate::tracer::span("app::launch_ast_diff");
        let ds = self.display_to_diff(disp_start);
        let de = self.display_to_diff(disp_end);

        let left_start = self.diff_lines[ds]
            .left_lineno
            .or_else(|| {
                self.diff_lines[..ds]
                    .iter()
                    .rev()
                    .find_map(|d| d.left_lineno)
            })
            .unwrap_or(0);
        let left_end = self.diff_lines[de]
            .left_lineno
            .or_else(|| {
                self.diff_lines[de + 1..]
                    .iter()
                    .find_map(|d| d.left_lineno)
            })
            .unwrap_or_else(|| self.left_lines.len().saturating_sub(1));

        let right_start = self.diff_lines[ds]
            .right_lineno
            .or_else(|| {
                self.diff_lines[..ds]
                    .iter()
                    .rev()
                    .find_map(|d| d.right_lineno)
            })
            .unwrap_or(0);
        let right_end = self.diff_lines[de]
            .right_lineno
            .or_else(|| {
                self.diff_lines[de + 1..]
                    .iter()
                    .find_map(|d| d.right_lineno)
            })
            .unwrap_or_else(|| self.right_lines.len().saturating_sub(1));

        let left_text = self.left_lines
            [left_start..=left_end.min(self.left_lines.len().saturating_sub(1))]
            .join("\n");
        let right_text = self.right_lines
            [right_start..=right_end.min(self.right_lines.len().saturating_sub(1))]
            .join("\n");

        match compute_ast_diff(&left_text, &right_text, left_start, right_start) {
            Ok(result) => {
                self.ast_collapsed.clear();
                self.ast_visible =
                    visible_rows(&result.left_nodes, &result.right_nodes, &self.ast_collapsed);
                self.ast_filter_rows = self.ast_visible.clone();
                self.ast_filter = AstFilter::All;
                self.ast_result = Some(result);
                self.mode = AppMode::AstDiff;
                self.ast_scroll = 0;
                self.ast_cursor = 0;
                self.status_msg = String::from(
                    " AST | Esc:back  j/k:move  Space:fold  f:filter  Enter:→source  q:quit ",
                );
            }
            Err(e) => {
                log::error!("AST parse failed: {}", e);
                self.status_msg = format!(" AST parse error: {} ", e);
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    pub fn clamp_search_ast_scroll(&mut self, view_height: usize) {
        if view_height == 0 {
            return;
        }
        let vis_len = self.search_ast_visible.len();
        if self.search_ast_cursor < self.search_ast_scroll {
            self.search_ast_scroll = self.search_ast_cursor;
        }
        if self.search_ast_cursor >= self.search_ast_scroll + view_height {
            self.search_ast_scroll = self.search_ast_cursor - view_height + 1;
        }
        if vis_len > 0 && self.search_ast_scroll + view_height > vis_len {
            self.search_ast_scroll = vis_len.saturating_sub(view_height);
        }
    }

    pub fn clamp_ast_scroll(&mut self, view_height: usize) {
        if view_height == 0 {
            return;
        }
        let vis_len = self.ast_filter_rows.len();
        if self.ast_cursor >= self.ast_scroll + view_height {
            self.ast_scroll = self.ast_cursor - view_height + 1;
        }
        if self.ast_cursor < self.ast_scroll {
            self.ast_scroll = self.ast_cursor;
        }
        if self.ast_scroll + view_height > vis_len {
            self.ast_scroll = vis_len.saturating_sub(view_height);
        }
    }

    // ── Mouse handling ────────────────────────────────────────────────────

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match self.mode {
            AppMode::Search => self.on_search_mouse(mouse),
            AppMode::Help
            | AppMode::TextDiff
            | AppMode::AstDiff
            | AppMode::ProjectBrowser => {}
        }
    }

    fn on_search_mouse(&mut self, mouse: MouseEvent) {
        let (col, row) = (mouse.column, mouse.row);
        match mouse.kind {
            // ── Scroll ────────────────────────────────────────────────────
            MouseEventKind::ScrollUp => {
                if rect_hit(self.search_source_area, col, row) {
                    self.search_source_scroll = self.search_source_scroll.saturating_sub(3);
                } else if rect_hit(self.search_ast_area, col, row) {
                    self.search_ast_scroll = self.search_ast_scroll.saturating_sub(3);
                } else if rect_hit(self.search_results_area, col, row) && self.search_scroll > 0 {
                    self.search_scroll -= 1;
                }
            }
            MouseEventKind::ScrollDown => {
                let n = self.search_results.len();
                if rect_hit(self.search_source_area, col, row) {
                    let max = self.search_source_lines.len().saturating_sub(1);
                    self.search_source_scroll = (self.search_source_scroll + 3).min(max);
                } else if rect_hit(self.search_ast_area, col, row) {
                    let max = self.search_ast_nodes.len().saturating_sub(1);
                    self.search_ast_scroll = (self.search_ast_scroll + 3).min(max);
                } else if rect_hit(self.search_results_area, col, row) && n > 0 {
                    self.search_scroll = (self.search_scroll + 1).min(n.saturating_sub(1));
                }
            }
            // ── Mouse down: start selection in source pane, click in results ─
            MouseEventKind::Down(MouseButton::Left) => {
                if rect_hit(self.search_source_area, col, row) {
                    // Start drag selection
                    let pos = self.screen_to_source_pos(col, row);
                    self.search_sel_anchor = pos;
                    self.search_sel = pos.map(|p| (p, p));
                } else if rect_hit(self.search_ast_area, col, row) {
                    self.handle_ast_viz_click(col, row);
                } else if rect_hit(self.search_results_area, col, row) && !self.search_results.is_empty() {
                    // Click to select result
                    let inner_y = self.search_results_area.y + 1;
                    if row >= inner_y {
                        let row_in_pane = (row - inner_y) as usize;
                        let idx = self.search_scroll + row_in_pane / ROWS_PER_RESULT;
                        if idx < self.search_results.len() {
                            log::debug!("mouse click: result #{}", idx);
                            self.search_selected = idx;
                            self.load_search_result(idx);
                            // Clear source selection when changing result
                            self.search_sel = None;
                            self.search_sel_anchor = None;
                        }
                    }
                }
            }
            // ── Mouse drag: extend source selection, scroll if past edge ──
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.search_sel_anchor.is_some() {
                    let cur = self.drag_source_pos(col, row);
                    let anchor = self.search_sel_anchor.unwrap();
                    let (start, end) = if anchor <= cur { (anchor, cur) } else { (cur, anchor) };
                    self.search_sel = Some((start, end));
                }
            }
            // ── Mouse up: finalize selection and auto-copy ────────────────
            MouseEventKind::Up(MouseButton::Left) => {
                if self.search_sel_anchor.is_some() {
                    let cur = self.drag_source_pos(col, row);
                    let anchor = self.search_sel_anchor.unwrap();
                    let (start, end) = if anchor <= cur { (anchor, cur) } else { (cur, anchor) };
                    self.search_sel = Some((start, end));
                    self.search_sel_anchor = None;
                    // Auto-copy if we have a non-empty selection
                    if self.search_sel.is_some_and(|(s, e)| s != e) {
                        self.copy_source_selection();
                    }
                }
            }
            _ => {}
        }
    }

    /// Handle a left-click inside the AST pane.
    /// Single click → scroll source pane to the clicked line.
    /// Double click → set/clear the timeline zoom base.
    fn handle_ast_viz_click(&mut self, col: u16, row: u16) {
        let area    = self.search_ast_area;
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_w = area.width.saturating_sub(2) as usize;
        let inner_h = area.height.saturating_sub(2) as usize;
        if col < inner_x || row < inner_y { return; }
        let cx = (col - inner_x) as usize;
        let cy = (row - inner_y) as usize;
        if cx >= inner_w || cy >= inner_h { return; }

        let total_src = self.search_source_lines.len();
        if total_src == 0 { return; }

        // ── Tree view: clicking a row moves the cursor there ─────────────────
        if self.search_ast_viz == AstVizMode::Tree {
            let vis_pos = self.search_ast_scroll + cy;
            if vis_pos >= self.search_ast_visible.len() { return; }
            let raw = self.search_ast_visible[vis_pos];
            if raw >= self.search_ast_nodes.len() { return; }
            self.search_ast_cursor = vis_pos;
            self.highlight_ast_node_in_source(raw);
            return;
        }

        // Detect double-click: two clicks within 400 ms at the same position (±1 cell).
        let now = std::time::Instant::now();
        let is_double = self.search_timeline_last_click
            .map(|(t, lc, lr)| {
                now.duration_since(t).as_millis() < 400
                    && (col as i32 - lc as i32).abs() <= 1
                    && (row as i32 - lr as i32).abs() <= 1
            })
            .unwrap_or(false);
        self.search_timeline_last_click = Some((now, col, row));

        // ── Timeline ─────────────────────────────────────────────────────────
        let label_w = 4usize;
        let chart_w = inner_w.saturating_sub(label_w);
        if chart_w == 0 { return; }

        let nodes = &self.search_ast_nodes;
        let non_empty: Vec<(usize, _)> = nodes.iter()
            .enumerate()
            .filter(|(_, n)| !n.empty && !n.kind.is_empty())
            .collect();
        if non_empty.is_empty() { return; }

        // Compute the zoom window (respects current base if set).
        let (min_row, max_row, base_depth) =
            if let Some(br) = self.search_timeline_base.filter(|&br| br < nodes.len()) {
                let n = &nodes[br];
                (n.source_row, n.source_end_row, n.depth)
            } else {
                let mn = non_empty.iter().map(|(_, n)| n.source_row).min().unwrap_or(0);
                let mx = non_empty.iter().map(|(_, n)| n.source_end_row).max().unwrap_or(0);
                (mn, mx, 0)
            };
        let total_span = (max_row - min_row + 1).max(1);

        // cy == 0 is the ruler header row; depth rows start at cy == 1.
        let clicked_depth = if cy == 0 {
            // Clicked on the ruler — just navigate source, no zoom change.
            let source_line = if cx >= label_w {
                min_row + (cx - label_w) * total_span / chart_w
            } else { min_row };
            let clamped = source_line.min(total_src.saturating_sub(1));
            self.search_source_scroll    = clamped;
            self.search_source_highlight = clamped;
            return;
        } else {
            let depth_scroll = self.search_ast_scroll;
            base_depth + depth_scroll + (cy - 1)
        };

        // Map chart column → source position.
        let source_pos = if cx >= label_w {
            min_row + (cx - label_w) * total_span / chart_w
        } else {
            min_row
        };

        // Find the AST node at (depth, source_pos) and, if double-clicking the
        // base, pre-compute the zoom-out target. Do it all inside a borrow scope
        // so the immutable borrows of `nodes`/`non_empty` are released before the
        // mutable `highlight_ast_node_in_source` call below.
        let (hit_raw, zoom_parent) = {
            let hit = non_empty.iter().find(|(_, n)| {
                n.depth == clicked_depth
                    && n.source_row <= source_pos
                    && n.source_end_row >= source_pos
            }).map(|(i, _)| *i);

            // Pre-compute zoom-out parent (only needed if is_double + hit == current base).
            let parent = if is_double {
                if let Some(raw) = hit.filter(|&r| self.search_timeline_base == Some(r)) {
                    let nd = &nodes[raw];
                    non_empty.iter()
                        .filter(|(_, n)| {
                            n.depth < nd.depth
                                && n.source_row  <= nd.source_row
                                && n.source_end_row >= nd.source_end_row
                        })
                        .max_by_key(|(_, n)| n.depth)
                        .map(|(i, _)| *i)
                } else {
                    None
                }
            } else {
                None
            };
            // `nodes` and `non_empty` borrows end here.
            (hit, parent)
        };

        // ── Single-click: navigate + highlight ───────────────────────────────
        if let Some(raw) = hit_raw {
            self.highlight_ast_node_in_source(raw);
        } else {
            let clamped = source_pos.min(total_src.saturating_sub(1));
            self.search_source_scroll    = clamped;
            self.search_source_highlight = clamped;
        }

        // ── Double-click: set/clear zoom base ────────────────────────────────
        if is_double {
            match hit_raw {
                Some(raw) if self.search_timeline_base == Some(raw) => {
                    self.search_timeline_base = zoom_parent;
                }
                Some(raw) => {
                    self.search_timeline_base = Some(raw);
                }
                None => {
                    self.search_timeline_base = None;
                }
            }
            self.search_ast_scroll = 0;
            log::debug!(
                "timeline zoom: base={:?}",
                self.search_timeline_base
                    .and_then(|r| self.search_ast_nodes.get(r))
                    .map(|n| n.kind.as_str())
            );
        }
    }

    /// Scroll the source pane to `raw`'s position and highlight its token/span.
    /// Called by both tree-view clicks and timeline clicks.
    fn highlight_ast_node_in_source(&mut self, raw: usize) {
        if raw >= self.search_ast_nodes.len() { return; }
        let total_src = self.search_source_lines.len();
        if total_src == 0 { return; }

        let node = &self.search_ast_nodes[raw];

        // Scroll source to the node's first line.
        self.search_source_scroll    = node.source_row.min(total_src.saturating_sub(1));
        self.search_source_highlight = node.source_row;

        // Build the selection that will be highlighted in the source pane.
        let sel = if let Some(leaf) = &node.leaf_text {
            // Leaf: try to pinpoint the exact token within the line.
            let leaf_clean = leaf.replace('↵', "\n").replace('→', "\t");
            let leaf_trim  = leaf_clean.trim_matches('"');
            self.search_source_lines
                .get(node.source_row)
                .and_then(|line_text| {
                    line_text.find(leaf_trim).map(|byte_off| {
                        let start_col = line_text[..byte_off].chars().count();
                        let end_col   = start_col + leaf_trim.chars().count();
                        ((node.source_row, start_col), (node.source_row, end_col))
                    })
                })
                .unwrap_or_else(|| {
                    let end = self.search_source_lines
                        .get(node.source_row)
                        .map(|l| l.chars().count())
                        .unwrap_or(0);
                    ((node.source_row, 0), (node.source_row, end))
                })
        } else {
            // Non-leaf: select the full line range of this subtree.
            let end_col = self.search_source_lines
                .get(node.source_end_row)
                .map(|l| l.chars().count())
                .unwrap_or(0);
            ((node.source_row, 0), (node.source_end_row, end_col))
        };
        self.search_sel = Some(sel);

        // Sync tree cursor.
        if let Some(vis_pos) = self.search_ast_visible.iter().position(|&r| r == raw) {
            self.search_ast_cursor = vis_pos;
        }

        log::debug!(
            "ast click: node={} L{}–{} sel={:?}",
            node.kind, node.source_row + 1, node.source_end_row + 1, sel
        );
    }

    /// Convert a screen (col, row) in terminal coordinates to a (line, char_col) pair
    /// in source-file space, taking the scroll offset and gutter width into account.
    /// Returns None if the position is outside the inner content area.
    fn screen_to_source_pos(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.search_source_area;
        let inner_x = area.x + 1;
        let inner_y = area.y + 1;
        let inner_w = area.width.saturating_sub(2);
        let inner_h = area.height.saturating_sub(2);
        if col < inner_x || row < inner_y { return None; }
        let cx = col - inner_x;
        let cy = row - inner_y;
        if cx >= inner_w || cy >= inner_h { return None; }

        // Gutter: "{:4} " = 5 chars, prefix "  " or "▶ " = 2 chars → total 7 chars
        const GUTTER: u16 = 7;
        let char_col = cx.saturating_sub(GUTTER) as usize;
        let file_line = self.search_source_scroll + cy as usize;
        if file_line >= self.search_source_lines.len() { return None; }
        Some((file_line, char_col))
    }

    /// Like `screen_to_source_pos` but always returns a valid position, even when
    /// the mouse is above or below the visible source area.  When the cursor is
    /// outside the top/bottom edge the scroll is advanced by one line in that
    /// direction so that holding the mouse there produces continuous scrolling.
    fn drag_source_pos(&mut self, col: u16, row: u16) -> (usize, usize) {
        let area = self.search_source_area;
        let inner_x  = area.x + 1;
        let inner_y  = area.y + 1;
        let inner_h  = area.height.saturating_sub(2) as usize;
        let n_lines  = self.search_source_lines.len();

        const GUTTER: u16 = 7;
        // Horizontal char column (clamped to gutter edge and line length later)
        let char_col = if col >= inner_x {
            (col - inner_x).saturating_sub(GUTTER) as usize
        } else {
            0
        };

        if row < inner_y {
            // ── Above the pane: scroll up one line, snap to start of that line ──
            self.search_source_scroll = self.search_source_scroll.saturating_sub(1);
            let file_line = self.search_source_scroll;
            (file_line, 0)
        } else {
            let cy = (row - inner_y) as usize;
            if cy >= inner_h {
                // ── Below the pane: scroll down one line, snap to end of that line ──
                let max_scroll = n_lines.saturating_sub(1);
                self.search_source_scroll = (self.search_source_scroll + 1).min(max_scroll);
                // Point at the last visible line
                let file_line = (self.search_source_scroll + inner_h).min(n_lines).saturating_sub(1);
                let line_len  = self.search_source_lines
                    .get(file_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                (file_line, line_len)
            } else {
                // ── Inside the pane: normal mapping ──────────────────────────
                let file_line = (self.search_source_scroll + cy).min(n_lines.saturating_sub(1));
                let line_len  = self.search_source_lines
                    .get(file_line)
                    .map(|l| l.chars().count())
                    .unwrap_or(0);
                (file_line, char_col.min(line_len))
            }
        }
    }

    /// Extract the selected text from search_source_lines and copy it to the clipboard.
    /// Updates status_msg with the result.
    fn copy_source_selection(&mut self) {
        let Some(((sl, sc), (el, ec))) = self.search_sel else {
            self.status_msg = String::from(" No selection — drag to select text ");
            return;
        };
        let mut parts: Vec<String> = Vec::new();
        for line_idx in sl..=el {
            let raw = match self.search_source_lines.get(line_idx) {
                Some(l) => l.as_str(),
                None => break,
            };
            let chars: Vec<char> = raw.chars().collect();
            let from = if line_idx == sl { sc } else { 0 };
            let to   = if line_idx == el { ec.min(chars.len()) } else { chars.len() };
            let from = from.min(chars.len());
            parts.push(chars[from..to].iter().collect());
        }
        let text = parts.join("\n");
        let char_count = text.chars().count();
        match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
            Ok(_) => {
                self.status_msg = format!(" Copied {} chars to clipboard ", char_count);
            }
            Err(e) => {
                self.status_msg = format!(" Clipboard error: {} ", e);
            }
        }
    }

    /// Copy the currently highlighted AST node's kind, text, and line info to the clipboard.
    fn copy_ast_node(&mut self) {
        let Some(node) = self.search_ast_nodes.get(self.search_ast_highlight) else {
            self.status_msg = String::from(" No AST node selected ");
            return;
        };
        let text = if let Some(ref leaf) = node.leaf_text {
            format!("{} \"{}\" :L{}", node.kind, leaf, node.source_row + 1)
        } else {
            format!("{} :L{}", node.kind, node.source_row + 1)
        };
        let char_count = text.chars().count();
        match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
            Ok(_) => {
                self.status_msg = format!(" Copied {} chars (AST node) to clipboard ", char_count);
            }
            Err(e) => {
                self.status_msg = format!(" Clipboard error: {} ", e);
            }
        }
    }

    fn text_diff_hint(ignore_ws: bool) -> String {
        format!(
            " q:quit  s/e:select  Enter:AST  n/N:hunk  u:unified  c:context  w:ws({})  i:inline  x:patch  X:html ",
            if ignore_ws { "ON" } else { "OFF" }
        )
    }

    // ── Project browser mode ──────────────────────────────────────────────

    fn on_project_browser(&mut self, key: KeyEvent) {
        let n = self.project_display.len();

        // When the filter bar is active, most keys go to the text input.
        if self.project_filter_active {
            match key.code {
                KeyCode::Esc => {
                    self.project_filter_active = false;
                    self.project_filter.clear();
                    self.rebuild_project_display();
                    self.status_msg = String::from(
                        " j/k:navigate  Enter:open  1-4:views  s:filter  g:grep  f:ast  q:quit",
                    );
                }
                KeyCode::Enter => {
                    self.project_filter_active = false;
                    self.status_msg = format!(
                        " Filter: {:?}  1-4:views  s:edit  Esc:clear ",
                        self.project_filter.as_str()
                    );
                }
                KeyCode::Backspace => {
                    self.project_filter.backspace();
                    self.rebuild_project_display();
                }
                KeyCode::Left  => self.project_filter.move_left(),
                KeyCode::Right => self.project_filter.move_right(),
                KeyCode::Char(c) => {
                    self.project_filter.push(c);
                    self.rebuild_project_display();
                    self.project_cursor = 0;
                }
                _ => {}
            }
            return;
        }

        // Normal command mode
        match key.code {
            // ── Navigation ────────────────────────────────────────────────
            KeyCode::Up | KeyCode::Char('k') => {
                if self.project_cursor > 0 { self.project_cursor -= 1; }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.project_cursor + 1 < n { self.project_cursor += 1; }
            }
            KeyCode::PageUp   => self.project_cursor = self.project_cursor.saturating_sub(20),
            KeyCode::PageDown => self.project_cursor = (self.project_cursor + 20).min(n.saturating_sub(1)),
            KeyCode::Home     => self.project_cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.project_cursor = n.saturating_sub(1),

            // ── Expand/collapse (Space) ───────────────────────────────────
            KeyCode::Char(' ') => {
                self.project_toggle_expand();
            }

            // ── View switching (1-4 / Tab) ────────────────────────────────
            KeyCode::Char('1') => self.set_project_view(ProjectView::Tus),
            KeyCode::Char('2') => self.set_project_view(ProjectView::Sources),
            KeyCode::Char('3') => self.set_project_view(ProjectView::Headers),
            KeyCode::Char('4') => {
                if !self.cmake_targets.is_empty() {
                    self.set_project_view(ProjectView::Cmake);
                }
            }
            KeyCode::Tab => {
                let next = match self.project_view {
                    ProjectView::Tus     => ProjectView::Sources,
                    ProjectView::Sources => ProjectView::Headers,
                    ProjectView::Headers => {
                        if !self.cmake_targets.is_empty() { ProjectView::Cmake }
                        else { ProjectView::Tus }
                    }
                    ProjectView::Cmake   => ProjectView::Tus,
                };
                self.set_project_view(next);
            }

            // ── Filter (s) ────────────────────────────────────────────────
            KeyCode::Char('s') => {
                self.project_filter_active = true;
                self.status_msg = String::from(" Type to filter  Enter:confirm  Esc:clear ");
            }

            // ── Search (g=grep, f=AST) ────────────────────────────────────
            KeyCode::Char('g') => {
                log::info!("mode: ProjectBrowser → Search (grep)");
                self.search_input.clear();
                self.search_results.clear();
                self.search_selected = 0;
                self.search_grep_mode = true;
                self.search_prev_mode = AppMode::ProjectBrowser;
                self.mode = AppMode::Search;
                self.status_msg = String::from(" GREP  type text  Enter:search  Esc:back ");
                // Pre-load the selected file so the source pane is immediately populated.
                if let Some(path) = self.selected_project_path() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        self.load_file_into_search_pane(&path, &content);
                    }
                }
            }
            KeyCode::Char('f') => {
                log::info!("mode: ProjectBrowser → Search (ast)");
                self.search_input.clear();
                self.search_results.clear();
                self.search_selected = 0;
                self.search_grep_mode = false;
                self.search_prev_mode = AppMode::ProjectBrowser;
                self.mode = AppMode::Search;
                self.status_msg = String::from(
                    " AST  fn: call: var: class: type: include: param: field:  or  (ts-query) @cap  Enter:search  Esc:back ",
                );
                // Pre-load the selected file so the source pane is immediately populated.
                if let Some(path) = self.selected_project_path() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        self.load_file_into_search_pane(&path, &content);
                    }
                }
            }

            // ── Open in TUI (Enter) ───────────────────────────────────────
            KeyCode::Enter => {
                if let Some(path) = self.selected_project_path() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        log::info!("opening file: {}", path);
                        self.open_project_file(path, content);
                    } else {
                        log::warn!("cannot read file: {}", path);
                        self.status_msg = String::from(" Cannot read file. ");
                    }
                }
            }

            // ── Open in external editor (Ctrl+o) ──────────────────────────
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(path) = self.selected_project_path() {
                    self.pending_open = Some((path.clone(), 0, 0));
                    self.status_msg = format!(" Opening {} in {} ", path, self.config.open_in.label());
                }
            }

            KeyCode::Esc => {
                self.project_filter.clear();
                self.project_filter_active = false;
                self.rebuild_project_display();
            }

            _ => {}
        }
    }

    // ── Project display helpers ───────────────────────────────────────────

    fn set_project_view(&mut self, view: ProjectView) {
        self.project_view = view;
        self.project_cursor = 0;
        self.rebuild_project_display();
        let hint = match self.project_view {
            ProjectView::Tus     => " 1:TUs  2:Sources  3:Headers  4:CMake  s:filter  g:grep  f:ast",
            ProjectView::Sources => " 1:TUs  2:Sources  3:Headers  4:CMake  s:filter  g:grep  f:ast",
            ProjectView::Headers => " 1:TUs  2:Sources  3:Headers  4:CMake  s:filter  g:grep  f:ast",
            ProjectView::Cmake   => " 1:TUs  2:Sources  3:Headers  4:CMake  Space:expand  s:filter",
        };
        self.status_msg = hint.to_string();
    }

    fn project_toggle_expand(&mut self) {
        // Only CMake targets are expandable; the TU view no longer shows headers inline.
        let Some(row) = self.project_display.get(self.project_cursor).cloned() else { return };
        match row {
            ProjectRow::CmakeTarget(idx) => {
                if self.cmake_targets[idx].sources.is_empty() { return; }
                if self.project_expanded.contains(&idx) {
                    self.project_expanded.remove(&idx);
                } else {
                    self.project_expanded.insert(idx);
                }
                self.rebuild_project_display();
            }
            _ => {}
        }
    }

    fn selected_project_path(&self) -> Option<String> {
        match self.project_display.get(self.project_cursor)? {
            ProjectRow::Source(idx)              => Some(self.project_files[*idx].file_path.clone()),
            ProjectRow::LooseHeader(idx)         => Some(self.project_files[*idx].file_path.clone()),
            ProjectRow::CmakeSource { path, .. } => Some(path.clone()),
            ProjectRow::CmakeTarget(_)       => None, // targets aren't files
        }
    }

    pub fn rebuild_project_display(&mut self) {
        let _s = crate::tracer::span("app::rebuild_project_display");
        let filter = self.project_filter.as_str().to_lowercase();
        let mut rows: Vec<ProjectRow> = Vec::new();

        match self.project_view {
            ProjectView::Tus => {
                for (idx, tu) in self.project_files.iter().enumerate() {
                    if tu.is_header { continue; }
                    if !filter.is_empty() && !tu.file_path.to_lowercase().contains(&filter) {
                        continue;
                    }
                    rows.push(ProjectRow::Source(idx));
                }
            }
            ProjectView::Sources => {
                for (idx, tu) in self.project_files.iter().enumerate() {
                    if tu.is_header { continue; }
                    if !filter.is_empty() && !tu.file_path.to_lowercase().contains(&filter) {
                        continue;
                    }
                    rows.push(ProjectRow::Source(idx));
                }
            }
            ProjectView::Headers => {
                for (idx, tu) in self.project_files.iter().enumerate() {
                    if !tu.is_header { continue; }
                    if !filter.is_empty() && !tu.file_path.to_lowercase().contains(&filter) {
                        continue;
                    }
                    rows.push(ProjectRow::LooseHeader(idx));
                }
            }
            ProjectView::Cmake => {
                for (idx, tgt) in self.cmake_targets.iter().enumerate() {
                    if !filter.is_empty() && !tgt.name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    rows.push(ProjectRow::CmakeTarget(idx));
                    if self.project_expanded.contains(&idx) {
                        for src in &tgt.sources {
                            rows.push(ProjectRow::CmakeSource {
                                tgt_idx: idx,
                                path: src.clone(),
                            });
                        }
                    }
                }
            }
        }

        self.project_display = rows;
        self.project_cursor = self.project_cursor.min(
            self.project_display.len().saturating_sub(1)
        );
    }

    // ── Search mode ───────────────────────────────────────────────────────

    /// Move the AST cursor by `delta` rows (negative = up) and sync the source pane.
    fn ast_cursor_move(&mut self, delta: i64) {
        let vis_len = self.search_ast_visible.len();
        if vis_len == 0 { return; }
        let new_cursor = (self.search_ast_cursor as i64 + delta)
            .clamp(0, (vis_len as i64) - 1) as usize;
        self.search_ast_cursor = new_cursor;

        // Sync source pane to the node's source line.
        if let Some(&raw) = self.search_ast_visible.get(new_cursor) {
            if let Some(node) = self.search_ast_nodes.get(raw) {
                let row = node.source_row;
                self.search_source_highlight = row;
                // Keep the highlighted line near the middle of the source pane.
                self.search_source_scroll = row.saturating_sub(5);
            }
        }
    }

    /// Arm the 300 ms debounce timer. Called whenever the user mutates a search
    /// input field so that a search fires automatically after a short pause.
    fn nudge_search_debounce(&mut self) {
        self.search_debounce = Some(std::time::Instant::now());
    }

    fn on_search(&mut self, key: KeyEvent) {
        // Global keys that work regardless of which input has focus.
        match key.code {
            // ── Run search immediately (any focus) — also clears debounce
            KeyCode::Enter => {
                self.search_debounce = None;
                self.run_search();
                return;
            }

            // ── Navigate results OR AST (only when query box has focus)
            KeyCode::Up if self.search_focus == SearchFocus::Query => {
                if !self.search_results.is_empty() {
                    if self.search_selected > 0 {
                        self.search_selected -= 1;
                        self.load_search_result(self.search_selected);
                    }
                } else {
                    self.ast_cursor_move(-1);
                }
                return;
            }
            KeyCode::Down if self.search_focus == SearchFocus::Query => {
                if !self.search_results.is_empty() {
                    if self.search_selected + 1 < self.search_results.len() {
                        self.search_selected += 1;
                        self.load_search_result(self.search_selected);
                    }
                } else {
                    self.ast_cursor_move(1);
                }
                return;
            }
            KeyCode::PageUp if self.search_focus == SearchFocus::Query => {
                if !self.search_results.is_empty() {
                    self.search_selected = self.search_selected.saturating_sub(10);
                    self.load_search_result(self.search_selected);
                } else {
                    self.ast_cursor_move(-10);
                }
                return;
            }
            KeyCode::PageDown if self.search_focus == SearchFocus::Query => {
                if !self.search_results.is_empty() {
                    let n = self.search_results.len();
                    self.search_selected = (self.search_selected + 10).min(n.saturating_sub(1));
                    self.load_search_result(self.search_selected);
                } else {
                    self.ast_cursor_move(10);
                }
                return;
            }

            // ── Fold / unfold AST node (file-browse mode, no results)
            KeyCode::Char(' ') if self.search_results.is_empty()
                               && !self.search_source_lines.is_empty() => {
                if let Some(&raw) = self.search_ast_visible.get(self.search_ast_cursor) {
                    let dummy: Vec<AstLine> = vec![];
                    if row_has_children(raw, &self.search_ast_nodes, &dummy) {
                        if self.search_ast_collapsed.contains(&raw) {
                            self.search_ast_collapsed.remove(&raw);
                        } else {
                            self.search_ast_collapsed.insert(raw);
                        }
                        self.rebuild_search_ast_visible();
                        // Keep cursor in bounds after rebuild.
                        let vis_len = self.search_ast_visible.len();
                        if self.search_ast_cursor >= vis_len && vis_len > 0 {
                            self.search_ast_cursor = vis_len - 1;
                        }
                    }
                }
                return;
            }

            // ── Tab / Shift+Tab: cycle focus between Query → Include → Exclude
            KeyCode::Tab => {
                self.search_focus = match self.search_focus {
                    SearchFocus::Query   => SearchFocus::Include,
                    SearchFocus::Include => SearchFocus::Exclude,
                    SearchFocus::Exclude => SearchFocus::Query,
                };
                return;
            }
            KeyCode::BackTab => {
                self.search_focus = match self.search_focus {
                    SearchFocus::Query   => SearchFocus::Exclude,
                    SearchFocus::Include => SearchFocus::Query,
                    SearchFocus::Exclude => SearchFocus::Include,
                };
                return;
            }

            // ── Open result in external editor (Ctrl+o)
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(result) = self.search_results.get(self.search_selected) {
                    self.pending_open =
                        Some((result.file_path.clone(), result.line, result.col));
                    self.status_msg = format!(
                        " Opening {}:{} in {} ",
                        result.short_path(), result.line + 1, self.config.open_in.label()
                    );
                }
                return;
            }

            // ── Toggle regex mode (Alt+R) — retriggers search automatically
            KeyCode::Char('r') if key.modifiers == KeyModifiers::ALT => {
                self.search_use_regex = !self.search_use_regex;
                self.nudge_search_debounce();
                return;
            }

            // ── Toggle case-sensitive mode (Alt+C) — retriggers search automatically
            KeyCode::Char('c') if key.modifiers == KeyModifiers::ALT => {
                self.search_case_sensitive = !self.search_case_sensitive;
                self.nudge_search_debounce();
                return;
            }

            // ── Copy current source selection to clipboard (Ctrl+C)
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.copy_source_selection();
                return;
            }

            // ── Copy current AST node info to clipboard (Ctrl+Y)
            KeyCode::Char('y') if key.modifiers == KeyModifiers::CONTROL => {
                self.copy_ast_node();
                return;
            }

            // ── Cycle AST visualization mode (Alt+v)
            KeyCode::Char('v') if key.modifiers == KeyModifiers::ALT => {
                self.search_ast_viz = self.search_ast_viz.next();
                self.search_ast_scroll = 0;
                self.search_timeline_base = None; // reset zoom when switching views
                return;
            }

            // ── Back — returns to previous mode (project browser or diff)
            KeyCode::Esc => {
                log::debug!("mode: Search → {:?}", self.search_prev_mode);
                self.mode = self.search_prev_mode.clone();
                self.search_focus = SearchFocus::Query;
                self.status_msg = String::from(
                    " j/k:navigate  Enter:open  s:filter  g:grep  f:ast  q:quit",
                );
                return;
            }

            _ => {}
        }

        // Route text-editing keys to the focused input.
        let input = match self.search_focus {
            SearchFocus::Query   => &mut self.search_input,
            SearchFocus::Include => &mut self.search_include,
            SearchFocus::Exclude => &mut self.search_exclude,
        };

        // Track whether the key can mutate the text content (not just move the cursor).
        let mutates = matches!(key.code,
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete
        ) || (key.code == KeyCode::Char('k') && key.modifiers == KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Char(c) if key.modifiers == KeyModifiers::NONE
                             || key.modifiers == KeyModifiers::SHIFT => {
                input.push(c);
            }
            KeyCode::Backspace                                         => { input.backspace(); }
            KeyCode::Delete                                            => { input.delete_forward(); }
            KeyCode::Left                                              => { input.move_left(); }
            KeyCode::Right                                             => { input.move_right(); }
            KeyCode::Home                                              => { input.move_home(); }
            KeyCode::End                                               => { input.move_end(); }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => { input.move_home(); }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => { input.move_end(); }
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => { input.kill_to_end(); }
            _ => {}
        }
        // input borrow ends here — safe to call &mut self methods below.

        // Re-arm the debounce whenever a key that can change the query text is pressed.
        // Pure cursor movements (Left/Right/Home/End/Ctrl+A/E) don't retrigger.
        if mutates {
            self.nudge_search_debounce();
        }
    }

    fn run_search(&mut self) {
        let _s = crate::tracer::span("app::run_search");
        let raw = self.search_input.as_str().to_string();
        if raw.is_empty() {
            self.status_msg = String::from(" Empty query. Type a search term. ");
            return;
        }
        log::info!("run_search: {:?} grep_mode={}", raw, self.search_grep_mode);

        self.search_query = if self.search_grep_mode {
            // Force grep regardless of what the user typed
            crate::search::SearchQuery {
                raw: raw.clone(),
                ts_query_src: String::new(),
                capture_filter: String::new(),
                grep_mode: true,
                filter_nested_calls: false,
                use_regex: self.search_use_regex,
                case_sensitive: self.search_case_sensitive,
            }
        } else {
            let mut q = parse_query(&raw);
            q.use_regex = self.search_use_regex;
            q.case_sensitive = self.search_case_sensitive;
            q
        };

        // Build the list of files to search.
        // In single-file mode (no project loaded) search only the open file;
        // in project mode apply the VS Code-style include/exclude glob filters.
        let file_paths: Vec<String> = if self.project_files.is_empty() {
            // Single-file or diff mode — scope search to the currently open file.
            vec![self.left_path.clone()]
        } else {
            let include = FileFilter::parse(self.search_include.as_str());
            let exclude = FileFilter::parse(self.search_exclude.as_str());
            let project_root = self.left_path.trim_end_matches('/').to_string();
            self.project_files
                .iter()
                .map(|tu| tu.file_path.clone())
                .filter(|path| {
                    // Match against the relative path inside the project root so that
                    // patterns like `src/**` work without requiring the full absolute path.
                    let rel = path
                        .strip_prefix(&format!("{}/", project_root))
                        .or_else(|| path.strip_prefix(&project_root))
                        .unwrap_or(path.as_str());
                    let pass_include = include.is_empty() || include.matches(rel) || include.matches(path);
                    let pass_exclude = exclude.is_empty() || (!exclude.matches(rel) && !exclude.matches(path));
                    pass_include && pass_exclude
                })
                .collect()
        };

        let n_files = file_paths.len();
        log::info!(
            "run_search: {}/{} files after include={:?} exclude={:?}",
            n_files,
            if self.project_files.is_empty() { 1 } else { self.project_files.len() },
            self.search_include.as_str(), self.search_exclude.as_str()
        );

        // Cancel any in-progress search, issue a new cancel token.
        self.search_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Arc::clone(&cancel);

        // Reset result state.
        self.search_results.clear();
        self.search_selected = 0;
        self.search_scroll = 0;
        self.search_source_lines = vec![];
        self.search_source_scroll = 0;
        self.search_source_highlight = 0;
        self.search_ast_nodes = vec![];
        self.search_ast_visible = vec![]; // must stay in sync with nodes
        self.search_ast_cursor = 0;
        self.search_ast_scroll = 0;
        self.search_ast_highlight = 0;
        self.search_running = true;

        // Bounded channel — 512 batches of results queued at most.
        let (tx, rx) = mpsc::sync_channel::<Vec<SearchResult>>(4096);
        self.search_rx = Some(rx);

        let query = self.search_query.clone();
        std::thread::spawn(move || {
            search_project_streaming(&file_paths, &query, tx, cancel);
        });

        self.status_msg = format!(
            " {} Searching {:?} across {} file{}… ",
            Self::spinner_frame(0),
            raw,
            n_files,
            if n_files == 1 { "" } else { "s" },
        );
    }

    /// Drain pending result batches from the background search thread.
    /// Call once per main-loop tick (≈ every 50 ms).
    pub fn tick_search(&mut self) {
        let _s = crate::tracer::span("app::tick_search");
        // ── Debounce: fire a new search 300 ms after the last keystroke ────
        if let Some(t) = self.search_debounce {
            if t.elapsed() >= std::time::Duration::from_millis(300) {
                self.search_debounce = None;
                if self.search_input.as_str().is_empty() {
                    // Query was cleared — cancel any running search and wipe results.
                    self.search_cancel.store(true, Ordering::Relaxed);
                    self.search_rx = None;
                    self.search_running = false;
                    self.search_results.clear();
                    self.search_selected = 0;
                    self.search_scroll = 0;
                    self.status_msg = String::from(" Type a query to search… ");
                } else {
                    self.run_search();
                }
            }
        }

        if self.search_rx.is_none() {
            return;
        }

        let prev_empty = self.search_results.is_empty();
        let mut done = false;

        // Drain up to 50 batches per tick so the UI is never stalled.
        for _ in 0..50 {
            match self.search_rx.as_ref().unwrap().try_recv() {
                Ok(batch) => {
                    self.search_results.extend(batch);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    done = true;
                    break;
                }
            }
        }

        // After draining, peek once more to catch a disconnect that arrived
        // right after the last batch (avoids a 50 ms delay in finalising).
        if !done {
            if let Err(mpsc::TryRecvError::Disconnected) =
                self.search_rx.as_ref().unwrap().try_recv()
            {
                done = true;
            }
        }

        // Load the first result as soon as any arrive.
        if prev_empty && !self.search_results.is_empty() {
            self.load_search_result(0);
        }

        if done {
            self.search_rx = None;
            self.search_running = false;

            // Sort: by file path then line number.
            self.search_results
                .sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

            // Re-load result 0 after sort so the right-pane stays consistent.
            self.search_selected = 0;
            self.search_scroll = 0;
            if !self.search_results.is_empty() {
                self.load_search_result(0);
            }

            let n = self.search_results.len();
            let raw = self.search_query.raw.clone();
            self.status_msg = format!(
                " {} result{} for {:?}  ↑↓:navigate  Ctrl+o:open in {}  Esc:back ",
                n,
                if n == 1 { "" } else { "s" },
                raw,
                self.config.open_in.label()
            );
        } else if self.search_running {
            let n = self.search_results.len();
            let frame = Self::spinner_frame(self.spinner_tick);
            self.status_msg = format!(
                " {} Searching…  {} result{} so far ",
                frame,
                n,
                if n == 1 { "" } else { "s" }
            );
        }
    }

    fn spinner_frame(tick: u64) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[(tick / 2) as usize % FRAMES.len()]
    }

    fn load_search_result(&mut self, idx: usize) {
        let _s = crate::tracer::span("app::load_search_result");
        let Some(result) = self.search_results.get(idx).cloned() else {
            return;
        };
        log::debug!(
            "load_search_result: #{} {}:{} kind={} capture=@{}",
            idx, result.file_path, result.line + 1, result.node_kind, result.capture_name
        );

        // Clamp results scroll so selected is visible (handled by renderer, but track scroll)
        self.search_scroll = if self.search_selected < self.search_scroll {
            self.search_selected
        } else {
            self.search_scroll
        };

        // Load source lines + syntax highlighting
        let Ok(content) = fs::read_to_string(&result.file_path) else {
            return;
        };
        self.search_source_lines = content.lines().map(|l| l.to_string()).collect();
        self.search_source_tokens = crate::syntax::highlight(&content);
        self.search_source_highlight = result.line;
        self.search_source_scroll = result.line.saturating_sub(5);

        // Parse AST
        let nodes = parse_single(&content).unwrap_or_default();
        // Find the node at the match line with matching kind
        let highlight = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.source_row == result.line)
            .max_by_key(|(_, n)| {
                if n.kind == result.node_kind { 1000 + n.depth } else { n.depth }
            })
            .map(|(i, _)| i)
            .unwrap_or(0);

        self.search_ast_nodes = nodes;
        self.search_ast_highlight = highlight;
        // Reset collapsed state so the highlighted node is always visible.
        self.search_ast_collapsed = HashSet::new();
        self.rebuild_search_ast_visible();
        // Place cursor on the highlighted node.
        self.search_ast_cursor = self.search_ast_visible
            .iter()
            .position(|&r| r == highlight)
            .unwrap_or(0);
        self.search_ast_scroll = self.search_ast_cursor.saturating_sub(5);
    }

}

// ── Free helpers ─────────────────────────────────────────────────────────

/// Returns true if `(col, row)` is inside `area` (inclusive of border).
fn short_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn rect_hit(area: Rect, col: u16, row: u16) -> bool {
    area.width > 0
        && area.height > 0
        && col >= area.x
        && col < area.x + area.width
        && row >= area.y
        && row < area.y + area.height
}
