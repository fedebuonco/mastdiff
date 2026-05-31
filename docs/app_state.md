# App State Reference

This document describes the `App` struct in `src/app.rs` — the single source
of truth for all application state.  It also covers the enums that the struct
depends on and the key methods that mutate it.

---

## Enums

### `AppMode`
Controls which view is active and which key/mouse handler fires.

| Variant | Description |
|---|---|
| `TextDiff` | Two-file diff view (side-by-side or unified) |
| `AstDiff` | AST diff view (launched from a selection in TextDiff) |
| `ProjectBrowser` | File / CMake target browser |
| `Search` | Combined search + file-browse view |
| `Help` | Keyboard reference overlay (painted over the previous mode) |

### `AstVizMode`
Selects how the AST is rendered inside the search view's AST pane.

| Variant | Description |
|---|---|
| `Tree` | Indented collapsible node tree |
| `Timeline` | Gantt-style chart: depth on Y axis, source position on X axis |

Cycle with `v`. Reset scroll and timeline zoom base on each cycle.

### `AstFilter`
Applies a keyword filter to the AST diff view's node list.

| Variant | Keyword matched against node `kind` |
|---|---|
| `All` | *(no filter)* |
| `Functions` | `"function"` |
| `Classes` | `"class"` |
| `Variables` | `"declaration"` |

Cycle with `Shift+F`.

### `SearchFocus`
Which text input box has keyboard focus in Search mode.

| Variant | Box |
|---|---|
| `Query` | Main search / query input |
| `Include` | "Files to include" glob patterns |
| `Exclude` | "Files to exclude" glob patterns |

### `ProjectView`
Which sub-list the project browser shows.

| Variant | Contents |
|---|---|
| `Tus` | Source files, expandable to show associated headers |
| `Sources` | Flat list of source files only |
| `Headers` | Flat list of header files only |
| `Cmake` | CMake targets, expandable to show listed sources |

---

## App struct — field groups

### Identity
| Field | Type | Purpose |
|---|---|---|
| `left_path` | `String` | Left file path (or project dir) |
| `right_path` | `String` | Right file path (or same as left for project) |
| `left_lines` / `right_lines` | `Vec<String>` | Loaded text lines |

### Text diff state
| Field | Type | Purpose |
|---|---|---|
| `diff_lines` | `Vec<DiffLine>` | Computed diff rows |
| `scroll` | `usize` | Top visible row index |
| `cursor` | `usize` | Selected row index |
| `selection_start` / `_end` | `Option<usize>` | Range selected for AST diff |
| `ignore_ws` | `bool` | Whitespace-insensitive diff |
| `inline_diff` | `bool` | Show char-level inline diff |
| `context_only` | `bool` | Show only changed hunks + context |
| `context_lines` | `usize` | Context lines around each hunk |
| `context_indices` | `Vec<usize>` | Maps display rows → diff_lines when context_only=true |
| `unified_view` | `bool` | Single-column unified display |
| `hunk_pos` | `Vec<usize>` | Absolute diff-line indices of hunk starts |

### AST diff state
| Field | Type | Purpose |
|---|---|---|
| `ast_result` | `Option<AstDiffResult>` | Parsed and aligned AST pair |
| `ast_scroll` | `usize` | Scroll offset in AST view |
| `ast_cursor` | `usize` | Cursor position in visible list |
| `ast_collapsed` | `HashSet<usize>` | Raw row indices that are collapsed |
| `ast_filter` | `AstFilter` | Active kind filter |
| `ast_visible` | `Vec<usize>` | Visible raw indices after collapse |
| `ast_filter_rows` | `Vec<usize>` | Visible indices after collapse **and** filter |

### Project browser
| Field | Type | Purpose |
|---|---|---|
| `project_files` | `Vec<TranslationUnit>` | All discovered C++ files |
| `cmake_targets` | `Vec<CmakeTarget>` | CMake targets (may be empty) |
| `project_view` | `ProjectView` | Active sub-list |
| `project_display` | `Vec<ProjectRow>` | Flat display list for current view+filter |
| `project_expanded` | `HashSet<usize>` | Source/cmake indices that are expanded |
| `project_cursor` | `usize` | Cursor in `project_display` |
| `project_filter` | `TextInput` | Live filter string |
| `project_filter_active` | `bool` | Whether the filter box has focus |

`project_display` is the *rendered* list and is rebuilt by
`rebuild_project_display()` whenever view, filter, or expansion changes.

### Search / file-browse state
| Field | Type | Purpose |
|---|---|---|
| `search_open_file` | `String` | Path of the file currently in the source pane |
| `search_input` | `TextInput` | Query / grep text |
| `search_include` / `_exclude` | `TextInput` | Glob filters for file paths |
| `search_focus` | `SearchFocus` | Which input has focus |
| `search_use_regex` | `bool` | Alt+R — treat query as a regex |
| `search_case_sensitive` | `bool` | Alt+C — case-sensitive match |
| `search_query` | `SearchQuery` | Parsed query (updated on Enter) |
| `search_grep_mode` | `bool` | true = plain grep, false = tree-sitter query |
| `search_results` | `Vec<SearchResult>` | Matched results (streamed in) |
| `search_selected` | `usize` | Index of the selected result |
| `search_scroll` | `usize` | Top of the results list |
| `search_source_lines` | `Vec<String>` | Source lines of the open file |
| `search_source_tokens` | `Vec<Vec<SyntaxSpan>>` | Syntax-highlight spans per line |
| `search_source_scroll` | `usize` | Scroll offset of the source pane |
| `search_source_highlight` | `usize` | Line to highlight (absolute file line, `usize::MAX` = none) |
| `search_ast_nodes` | `Vec<AstLine>` | Flat AST of the open file |
| `search_ast_scroll` | `usize` | Scroll offset of the AST pane |
| `search_ast_highlight` | `usize` | Raw index of the result's matched node |
| `search_ast_collapsed` | `HashSet<usize>` | Collapsed raw node indices |
| `search_ast_visible` | `Vec<usize>` | Precomputed visible raw indices |
| `search_ast_cursor` | `usize` | Cursor in `search_ast_visible` |
| `search_ast_viz` | `AstVizMode` | Active AST visualisation |
| `search_timeline_base` | `Option<usize>` | Raw index used as zoom root in Timeline |
| `search_prev_mode` | `AppMode` | Mode to return to on Esc |

### Source pane selection
| Field | Type | Purpose |
|---|---|---|
| `search_sel_anchor` | `Option<(usize, usize)>` | Drag start position `(line, char_col)` |
| `search_sel` | `Option<((usize, usize), (usize, usize))>` | Normalised selection `(start, end)` both as `(line, char_col)` |

### UI hit-test areas (updated every frame by the renderer)
| Field | Type |
|---|---|
| `search_results_area` | `Rect` |
| `search_source_area` | `Rect` |
| `search_ast_area` | `Rect` |

### Async / streaming
| Field | Type | Purpose |
|---|---|---|
| `loading` | `bool` | Background project loader still running |
| `spinner_tick` | `u64` | Monotonic tick counter for spinner animation |
| `search_running` | `bool` | Background search thread active |
| `search_rx` | `Option<Receiver<Vec<SearchResult>>>` | Receives result batches |
| `search_cancel` | `Arc<AtomicBool>` | Set to `true` to abort a running search |
| `search_debounce` | `Option<Instant>` | Armed when the query changes; fires after 300 ms |

### Misc
| Field | Type | Purpose |
|---|---|---|
| `should_quit` | `bool` | Set to `true` to exit the main loop |
| `status_msg` | `String` | Status bar text (bottom of screen) |
| `config` | `Config` | Loaded user configuration |
| `pending_open` | `Option<(String, usize, usize)>` | `(file, line, col)` to open in external editor |

---

## Key methods

### Constructors
| Method | Creates |
|---|---|
| `new_diff(left, right, …)` | Two-file diff mode |
| `new_single_file(path, content, …)` | Single-file browse mode (Search) |
| `new_project_loading(dir, …)` | Project browser, empty file list |
| `new_project(data, dir, …)` | Synchronous (tests only) |

### Project loading
| Method | Purpose |
|---|---|
| `handle_load_msg(msg)` | Integrate a `LoadMsg::FilesBatch` or `Finished` from the background thread |
| `finish_project_load(data)` | Replace partial list, mark loading done, rebuild display |
| `rebuild_project_display()` | Recompute `project_display` from current view + filter + expansion |

### File opening
| Method | Purpose |
|---|---|
| `open_project_file(path, content)` | Open a file in Search mode (clears search, sets mode) |
| `load_file_into_search_pane(path, content)` | Load file into source/AST panes without changing mode or results |
| `rebuild_search_ast_visible()` | Recompute `search_ast_visible` after collapse changes |

### Scroll clamping (called by renderers)
| Method | Purpose |
|---|---|
| `clamp_search_ast_scroll(view_height)` | Keep AST cursor visible in Search view |
| `clamp_ast_scroll(view_height)` | Keep AST cursor visible in AST diff view |
| `clamp_scroll(view_height)` | Keep diff cursor visible in Text diff view |

### Search
| Method | Purpose |
|---|---|
| `run_search()` | Parse query, cancel any running search, spawn worker thread |
| `tick_search()` | Drain `search_rx` batches; fire debounced search |
| `load_search_result(idx)` | Load the file for result `idx` into source/AST panes |
| `nudge_search_debounce()` | Arm the 300 ms debounce timer |

### Click / highlight
| Method | Purpose |
|---|---|
| `handle_ast_viz_click(col, row)` | Dispatch Tree or Timeline click; single vs double |
| `highlight_ast_node_in_source(raw)` | Scroll source pane, set `search_sel`, sync `search_ast_cursor` |
| `screen_to_source_pos(col, row)` | Terminal coordinates → `(line, char_col)` in file space |
| `drag_source_pos(col, row)` | Like above but auto-scrolls at pane edges |

### Clipboard
| Method | Purpose |
|---|---|
| `copy_source_selection()` | Copy `search_sel` text to clipboard; update status |
| `copy_ast_node()` | Copy highlighted AST node info to clipboard |
