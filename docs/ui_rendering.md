# UI Rendering

All rendering happens in `src/ui/`.  The entry point is:

```rust
// ui/mod.rs
pub fn render(f: &mut Frame, app: &mut App, terminal_height: usize)
```

It dispatches based on `app.mode`:

| Mode | Module | Function |
|---|---|---|
| `TextDiff` | `diff_view` | `render(f, app, area)` |
| `AstDiff` | `ast_view` | `render(f, app, area)` |
| `ProjectBrowser` | `project_view` | `render(f, app, area)` |
| `Search` | `search_view` | `render(f, app, area)` |
| `Help` | — | First renders the underlying mode, then `help_view::render` on top |

---

## `ui/search_view.rs` — the main search/browse view

The most complex renderer.  It lays out three vertical columns plus a
top search bar:

```
┌─────────────────────────────────────────────────────────────────┐
│  Search bar (query input + include/exclude + mode badges)        │
├──────────────┬──────────────────────────────┬───────────────────┤
│  Results     │  Source pane                 │  AST pane         │
│  list        │  (syntax-highlighted)        │  (Tree/Timeline)  │
│              │                              │                   │
└──────────────┴──────────────────────────────┴───────────────────┘
│  Status bar                                                      │
└─────────────────────────────────────────────────────────────────┘
```

### Layout widths
- Results: 30 % of width (hidden when `search_results` is empty and no query)
- Source: remainder, split ~60/40 with AST

### Hit-test areas
After computing the layout, the renderer writes back three `Rect` values:
```rust
app.search_results_area = cols[0];
app.search_source_area  = cols[1];
app.search_ast_area     = cols[2];
```
These are used by `App::on_search_mouse` to dispatch mouse events to the
correct pane without repeating layout logic.

### `render_search_bar`
Builds a dynamic title span list.  In AST mode it loops over the `SHORTHANDS`
constant and renders each prefix in its accent colour with `BOLD`.  In grep
mode it shows a plain dim hint.  The `[.*]` and `[Aa]` badge spans are lit
(bright) when the corresponding toggle is active.

### `render_source_pane`
Renders line numbers + syntax-highlighted source text.  Overlays the current
`search_sel` selection (highlighted background) and `search_source_highlight`
(highlighted line).  The gutter is 7 characters wide (`"  1234 "`).

### `render_ast_pane`
Dispatches to `render_ast_tree` or `render_ast_timeline` based on
`app.search_ast_viz`.

#### `render_ast_tree`
- Calls `app.clamp_search_ast_scroll(view_height)` **first** (before taking
  any borrows of `app`).
- Iterates `search_ast_visible[scroll..]` up to `view_height` rows.
- Calls `render_ast_tree_node` for each visible index.
- `render_ast_tree_node` builds a `Line` with:
  - Collapse indicator (`▶` / `▼` / ` `)
  - Tree-drawing characters (`│`, `├─`, `└─`) based on depth
  - `kind` in a colour chosen by node family
  - `leaf_text` in a dim style
  - Highlighted background when the node is the current `search_ast_highlight`

Defensive bounds check: if `raw >= nodes.len()` the function returns an
`<invalid node N>` placeholder rather than panicking.

#### `render_ast_timeline`
Renders a Gantt-style chart:
- Row 0: ruler showing source line numbers at regular intervals
- Rows 1…N: one row per depth level

Each node is drawn as a horizontal bar spanning its `[source_row, source_end_row]`
range, rescaled to the chart width.  Overlapping nodes at the same depth
overwrite each other (last wins — does not matter visually).

When `search_timeline_base` is set the renderer:
1. Filters `visible_nodes` to those whose source range is contained within
   the base node's range.
2. Rescales the X axis to the base's `(source_row, source_end_row)`.
3. Makes depth labels relative (`d0` = base depth).
4. Renders the base node's bar in green to mark the zoom root.
5. Shows `↳ kind_name` in blue in the pane header.

The chart uses block-element characters (`▏▎▍▌▋▊▉█`) to sub-divide each
terminal cell for finer horizontal resolution.

---

## `ui/diff_view.rs`
Renders the two-file text diff.  Supports:
- Side-by-side (default) and unified (`u` key) layouts
- Context-only mode (only changed hunks ± N lines)
- Inline char-level diff within modified lines

---

## `ui/ast_view.rs`
Renders the AST diff after `s`/`e`/Enter in TextDiff mode.  Two columns:
left nodes and right nodes at the same index.  Nodes are colour-coded by
`NodeStatus` (Same / Added / Removed / Modified).

---

## `ui/project_view.rs`
Renders the project browser.  Four sub-views (TUs, Sources, Headers, CMake)
toggled with `1`-`4` or Tab.  Source files show an expand indicator (`▶`/`▼`)
when they have associated headers.  Shows a loading spinner while the
background loader is active.

---

## `ui/help_view.rs`
Draws a centred modal over the current view using `Clear` + a `Block`.
Content is split into two static columns built from `section()` / `row()` /
`blank()` helpers.  Each `row()` call renders the key binding in amber and
the description in lavender.

---

## `ui/single_view.rs`
Thin wrapper used in earlier code; now mostly replaced by the Search view
for single-file mode.

---

## Colour conventions

| Colour (RGB) | Used for |
|---|---|
| `(100, 140, 220)` | Primary accent (borders, section headers) |
| `(80, 120, 200)` | Section header background in help |
| `(220, 190, 80)` | Key binding labels (amber) |
| `(190, 190, 210)` | Descriptions, body text |
| `(140, 200, 140)` | Added nodes / lines |
| `(200, 100, 100)` | Removed nodes / lines |
| `(200, 200, 100)` | Modified nodes |
| `(120, 120, 150)` | Dim / secondary text |
| `(80, 200, 120)` | Timeline zoom-base bar |
