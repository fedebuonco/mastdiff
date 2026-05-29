use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent};

use crate::ast_diff::{
    compute_ast_diff, filter_rows, parse_single, row_has_children, visible_rows,
    AstDiffResult, AstLine,
};
use crate::export;
use crate::text_diff::{compute_diff, context_view, hunk_positions, DiffLine};

// ── Modes & enums ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    TextDiff,
    AstDiff,
    SingleFile,
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
    #[allow(dead_code)]
    pub single_file: bool,

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
    pub single_ast: Vec<AstLine>, // for single-file mode
    pub single_collapsed: HashSet<usize>,
    pub single_visible: Vec<usize>,
    pub single_scroll: usize,
    pub single_cursor: usize,
    pub single_filter: AstFilter,
    pub single_filter_rows: Vec<usize>,

    // ---- UI
    pub should_quit: bool,
    pub status_msg: String,
}

impl App {
    // ── Constructors ──────────────────────────────────────────────────────

    pub fn new_diff(left: String, right: String, left_path: String, right_path: String) -> Self {
        let left_lines: Vec<String> = left.lines().map(|l| l.to_string()).collect();
        let right_lines: Vec<String> = right.lines().map(|l| l.to_string()).collect();
        let diff_lines = compute_diff(&left, &right, false);
        let hunk_pos = hunk_positions(&diff_lines);

        Self {
            left_path,
            right_path,
            left_lines,
            right_lines,
            single_file: false,
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
            single_ast: vec![],
            single_collapsed: HashSet::new(),
            single_visible: vec![],
            single_scroll: 0,
            single_cursor: 0,
            single_filter: AstFilter::All,
            single_filter_rows: vec![],
            should_quit: false,
            status_msg: Self::text_diff_hint(false),
        }
    }

    pub fn new_single(content: String, path: String) -> Self {
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let ast = parse_single(&content).unwrap_or_default();
        let single_visible: Vec<usize> = (0..ast.len()).collect();
        let single_filter_rows = single_visible.clone();

        Self {
            left_path: path.clone(),
            right_path: path,
            left_lines: lines,
            right_lines: vec![],
            single_file: true,
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
            mode: AppMode::SingleFile,
            ast_result: None,
            ast_scroll: 0,
            ast_cursor: 0,
            ast_collapsed: HashSet::new(),
            ast_filter: AstFilter::All,
            ast_visible: vec![],
            ast_filter_rows: vec![],
            single_ast: ast,
            single_collapsed: HashSet::new(),
            single_visible: single_visible.clone(),
            single_scroll: 0,
            single_cursor: 0,
            single_filter: AstFilter::All,
            single_filter_rows,
            should_quit: false,
            status_msg: String::from(
                " q:quit  j/k:scroll  s/e:select  Enter:AST of selection  Space:fold  f:filter ",
            ),
        }
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
        match self.mode {
            AppMode::TextDiff => self.on_text_diff(key),
            AppMode::AstDiff => self.on_ast_diff(key),
            AppMode::SingleFile => self.on_single_file(key),
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

            // ── Export
            KeyCode::Char('x') => {
                let path = "astdiff_output.patch";
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
                let path = "astdiff_output.html";
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

            // ── Back
            KeyCode::Esc | KeyCode::Char('b') => {
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

    // ── Single-file mode ──────────────────────────────────────────────────

    fn on_single_file(&mut self, key: KeyEvent) {
        let total = self.left_lines.len();
        let vis_len = self.single_filter_rows.len();
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
            KeyCode::PageDown => self.cursor = (self.cursor + 20).min(total.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => self.cursor = total.saturating_sub(1),

            // ── Selection → AST zoom
            KeyCode::Char('s') => {
                self.selection_start = Some(self.cursor);
                self.selection_end = None;
                self.status_msg = format!(
                    " Start: line {}. Move and press 'e'. ",
                    self.cursor + 1
                );
            }
            KeyCode::Char('e') => {
                if let Some(s) = self.selection_start {
                    let end = self.cursor;
                    let (a, b) = if s <= end { (s, end) } else { (end, s) };
                    self.selection_start = Some(a);
                    self.selection_end = Some(b);
                    self.status_msg =
                        format!(" Lines {}-{}. Enter to zoom AST. Esc to clear. ", a + 1, b + 1);
                }
            }
            KeyCode::Enter => {
                let (start, end) = match (self.selection_start, self.selection_end) {
                    (Some(a), Some(b)) => (a, b),
                    _ => (0, total.saturating_sub(1)),
                };
                // Filter single AST to source lines in range
                let matching: Vec<usize> = self
                    .single_visible
                    .iter()
                    .copied()
                    .filter(|&r| {
                        let row = self.single_ast.get(r).map(|n| n.source_row).unwrap_or(0);
                        row >= start && row <= end
                    })
                    .collect();
                self.single_filter_rows = if matching.is_empty() {
                    self.single_visible.clone()
                } else {
                    matching
                };
                self.single_scroll = 0;
                self.single_cursor = 0;
                self.status_msg = format!(
                    " AST for lines {}-{}. Esc to restore. Space:fold  f:filter ",
                    start + 1,
                    end + 1
                );
            }
            KeyCode::Esc => {
                self.selection_start = None;
                self.selection_end = None;
                self.single_filter_rows = self.single_visible.clone();
                self.single_scroll = 0;
                self.single_cursor = 0;
                self.status_msg = String::from(
                    " q:quit  j/k:scroll  s/e:select  Enter:AST of selection  Space:fold  f:filter ",
                );
            }

            // ── AST collapse (right pane)
            KeyCode::Char(' ') => {
                if let Some(&raw) = self.single_filter_rows.get(self.single_cursor) {
                    let dummy: Vec<AstLine> = vec![];
                    if row_has_children(raw, &self.single_ast, &dummy) {
                        if self.single_collapsed.contains(&raw) {
                            self.single_collapsed.remove(&raw);
                        } else {
                            self.single_collapsed.insert(raw);
                        }
                        self.rebuild_single_visible();
                    }
                }
            }

            // ── AST cursor (right pane)
            KeyCode::Left => {
                if self.single_cursor > 0 {
                    self.single_cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.single_cursor + 1 < vis_len {
                    self.single_cursor += 1;
                }
            }

            // ── Filter
            KeyCode::Char('f') => {
                self.single_filter = self.single_filter.next();
                self.rebuild_single_visible();
                self.single_cursor = 0;
                self.single_scroll = 0;
                self.status_msg =
                    format!(" AST filter: {} ", self.single_filter.label());
            }

            _ => {}
        }
    }

    fn rebuild_single_visible(&mut self) {
        let dummy: Vec<AstLine> = vec![];
        self.single_visible = visible_rows(&self.single_ast, &dummy, &self.single_collapsed);
        let keyword = self.single_filter.keyword();
        self.single_filter_rows = if keyword.is_empty() {
            self.single_visible.clone()
        } else {
            let filtered_set: HashSet<usize> =
                filter_rows(&self.single_ast, keyword).into_iter().collect();
            self.single_visible
                .iter()
                .copied()
                .filter(|r| filtered_set.contains(r))
                .collect()
        };
    }

    // ── AST diff launch ───────────────────────────────────────────────────

    fn launch_ast_diff(&mut self, disp_start: usize, disp_end: usize) {
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
                self.status_msg = format!(" AST parse error: {} ", e);
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

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

    pub fn clamp_single_scroll(&mut self, view_height: usize) {
        if view_height == 0 {
            return;
        }
        let vis_len = self.single_filter_rows.len();
        if self.single_cursor >= self.single_scroll + view_height {
            self.single_scroll = self.single_cursor - view_height + 1;
        }
        if self.single_cursor < self.single_scroll {
            self.single_scroll = self.single_cursor;
        }
        if self.single_scroll + view_height > vis_len {
            self.single_scroll = vis_len.saturating_sub(view_height);
        }
    }

    fn text_diff_hint(ignore_ws: bool) -> String {
        format!(
            " q:quit  s/e:select  Enter:AST  n/N:hunk  u:unified  c:context  w:ws({})  i:inline  x:patch  X:html ",
            if ignore_ws { "ON" } else { "OFF" }
        )
    }
}
