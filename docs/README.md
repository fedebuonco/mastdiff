# mastdiff — Developer Documentation

Welcome to the mastdiff developer docs.  These pages describe the internal
architecture, data model, and extension points of the codebase.

---

## Documents

| File | Contents |
|---|---|
| [architecture.md](architecture.md) | High-level component map, startup modes (incl. headless `--search`/`--daemon`), main event loop, `AppMode` state machine, data-flow diagram, threading model |
| [app_state.md](app_state.md) | Complete reference for the `App` struct — every field group explained, all key methods listed |
| [ast_and_search.md](ast_and_search.md) | `AstLine` struct, parse/flatten/diff pipeline, `SearchQuery` type, shorthand expansion table, `search_project_streaming` internals, AST cache + persistent symbol cache, syntax highlighting |
| [project_discovery.md](project_discovery.md) | Project loader: directory walk, header association, CMake parsing, `LoadMsg` streaming protocol, `ProjectRow` flat display list |
| [ui_rendering.md](ui_rendering.md) | Renderer dispatch, search view layout, source pane, AST tree and timeline renderers, colour conventions |
| [performance.md](performance.md) | Benchmark binary + tracer, search optimisations, in-memory AST cache, and the persistent content-addressed symbol cache (L1/L2) |
| [extending.md](extending.md) | Step-by-step guides: add a search shorthand, add an AST viz mode, add an `AppMode`, tune performance, add a config key |

---

## Quick orientation

```
mastdiff <dir>            → project browser (streaming load)
mastdiff <file>           → single-file browse (search view)
mastdiff <left> <right>   → two-file diff
```

All state lives in `src/app.rs::App`.  The UI in `src/ui/` reads from `App`
and writes back only hit-test areas and scroll clamping.  Business logic
(mode transitions, search dispatch, file loading) stays in `app.rs`.

Logs go to `mastdiff.log` in the working directory.
Set `log_level = "debug"` in `~/.config/mastdiff/config.toml` for verbose output.
