# mastdiff — Architecture Overview

mastdiff is a terminal-UI (TUI) application written in Rust that visualises
C++ source diffs with AST awareness and a searchable project browser.
It is built on top of [ratatui](https://github.com/ratatui-org/ratatui) for
rendering, [crossterm](https://github.com/crossterm-rs/crossterm) for input,
and [tree-sitter](https://tree-sitter.github.io) for parsing C++.

---

## High-level component map

```
main.rs
  │
  ├─ config.rs          User configuration (~/.config/mastdiff/config.toml)
  ├─ logger.rs          File-based logging (mastdiff.log)
  │
  ├─ app.rs             Central state machine (App struct + event handlers)
  │    │
  │    ├─ text_diff.rs  Myers diff of two text files (DiffLine vec)
  │    ├─ ast_diff.rs   tree-sitter parse → AstLine flat tree; LCS diff
  │    ├─ search.rs     Query parsing, shorthand expansion, parallel search
  │    ├─ project.rs    Project discovery (files, headers, CMake targets)
  │    ├─ syntax.rs     Syntax highlighting (SyntaxSpan vec per line)
  │    ├─ input.rs      Editable text-input widget (TextInput)
  │    └─ export.rs     Patch and HTML export
  │
  └─ ui/
       ├─ mod.rs        Top-level render() dispatcher
       ├─ diff_view.rs  Text diff renderer (side-by-side & unified)
       ├─ ast_view.rs   AST diff renderer (two-pane, collapsible tree)
       ├─ project_view.rs  Project browser renderer
       ├─ search_view.rs   Search view renderer (3-pane: results/source/AST)
       ├─ single_view.rs   Single-file view (source + AST, no diff)
       └─ help_view.rs     Keyboard reference modal
```

---

## Startup modes

`main.rs` decides which `App` constructor to call based on CLI arguments:

| CLI invocation | App constructor | Initial mode |
|---|---|---|
| `mastdiff <dir>` | `App::new_project_loading` | `ProjectBrowser` |
| `mastdiff <file>` | `App::new_single_file` | `Search` (file pre-loaded) |
| `mastdiff <file1> <file2>` | `App::new_diff` | `TextDiff` |

In project mode a background thread is spawned immediately and sends
`project::LoadMsg` batches to the main loop over an `mpsc::channel`.
The TUI is interactive from the first frame — the loading spinner animates
while files trickle in.

---

## Main event loop (main.rs)

```
loop:
  app.spinner_tick += 1
  app.tick_search()                // drain streaming search results
  drain project_rx                 // integrate loader batches

  terminal.draw(|f| ui::render(f, &mut app, height))

  if event::poll(50 ms):
    match event::read():
      Mouse(e) → app.handle_mouse(e)
      Key(e)   → app.handle_key(e)
                 if app.pending_open → open editor (vim / vscode)

  if app.should_quit → break
```

The 50 ms poll gives roughly 20 frames/second while keeping CPU idle when
there is no input.

---

## AppMode state machine

```
          ┌──────────────────────────────────────────────────────────┐
          │                        Help (overlay)                    │
          │  ? from any mode → Help; Esc/q/? → prev mode            │
          └──────────────────────────────────────────────────────────┘

  ProjectBrowser ──g/f──► Search ──Esc──► ProjectBrowser
       │                    │
       │Enter (open file)   │Enter (no results, browse AST)
       └──────────────────►─┘

  TextDiff ──s+e+Enter──► AstDiff ──Esc──► TextDiff
```

The current mode is stored in `app.mode: AppMode`.
`app.handle_key` dispatches to the appropriate `on_*` method.

---

## Data-flow for a search

```
User types query
  → TextInput mutates search_input
  → nudge_search_debounce()          (arm 300 ms timer)

tick_search() on every frame
  → if debounce elapsed and query changed → run_search()
  → drain search_rx batches → append to search_results

run_search():
  → parse_query(input) → SearchQuery
  → cancel previous search_cancel token
  → spawn thread: search_project_streaming(files, query, cancel, tx)
  → set search_running = true

User selects result (↑/↓ or click)
  → load_search_result(idx)
  → load_file_into_search_pane(path, content)
  → parse_single(content) → search_ast_nodes
  → syntax::highlight(content) → search_source_tokens
  → rebuild_search_ast_visible()
```

---

## Renderer ↔ App coupling

The renderers in `ui/` receive `&mut App` and are allowed to write back
exactly two categories of state:

1. **Hit-test areas** — `search_results_area`, `search_source_area`,
   `search_ast_area` — set each frame so that mouse events can be resolved
   without re-doing layout arithmetic.
2. **Scroll clamping** — `app.clamp_search_ast_scroll(view_height)` and
   `app.clamp_ast_scroll(view_height)` are called by the renderer once it
   knows the pixel height of the pane.  This keeps the cursor visible and
   prevents overscroll.

Renderers must **not** mutate search results, mode, or any other
non-UI-geometry state — all business logic lives in `app.rs`.

---

## Threading model

```
┌─────────────────────────────────────────────────────────────────┐
│  Main thread  (TUI event loop, ~50 ms tick)                     │
│  Owns all App state. Never blocks — uses try_recv everywhere.   │
└──────────────┬──────────────────────────────┬───────────────────┘
               │                              │
    mpsc::channel (unbounded)      mpsc::sync_channel(512) (bounded)
    LoadMsg batches                Vec<SearchResult> batches
               │                              │
┌──────────────▼──────────┐   ┌──────────────▼────────────────────┐
│  Project-loader thread  │   │  Search-dispatcher thread          │
│  One, spawned on open.  │   │  Spawned fresh on each search.     │
│                         │   │  Old one abandoned (cancel token). │
│  walkdir + tree-sitter  │   │                                    │
│  #include extraction    │   │  PreparedSearch::new()  ← once     │
│  CMake parsing          │   │  files.par_iter()  ────────────►   │
│                         │   │    for_each_with(tx, ...)          │
│  Sends FilesBatch(25)   │   │                                    │
│  Sends Finished(data)   │   │  Arc<AtomicBool> cancel signal     │
└─────────────────────────┘   └────────────────────────────────────┘
                                              │
                                    Rayon global thread pool
                                    One task per file; workers
                                    share &PreparedSearch (Send+Sync)
```

| Thread | Lives | Channel | Cancel |
|--------|-------|---------|--------|
| Main | whole process | — | — |
| Project loader | until load completes | `mpsc::channel` (unbounded) | channel close |
| Search dispatcher | one per search call | `mpsc::sync_channel(512)` | `Arc<AtomicBool>` |
| Rayon workers (×N) | global pool, persistent | via dispatcher's `SyncSender` clone | same `AtomicBool` |

**Key design properties:**
- The 512-batch bound on the search channel provides back-pressure — if the
  main thread falls behind, workers block briefly rather than accumulating
  unbounded memory.
- `Parser` (tree-sitter) is `!Send` so it is created fresh per file inside
  each Rayon task.  The `Query` object is `Send + Sync` and is compiled once
  per search in `PreparedSearch`, then shared across all workers.
- Cancellation is cooperative: Rayon tasks check `cancel.load(Relaxed)`
  before each file.  Abandoned search threads drain naturally; they are never
  killed.

See [`docs/performance.md`](performance.md) for profiling methodology and
search optimisation details.
