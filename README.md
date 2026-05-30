# mastdiff

A terminal UI for exploring and diffing C++ codebases using tree-sitter.

Combines three tools in one keyboard-driven interface: a **side-by-side text diff**, a **live AST diff**, and a **structural code search** powered by native tree-sitter queries — all without leaving the terminal.

![mastdiff screenshot](assets/mastdiff.png)

```
mastdiff src/audio.cpp src/audio_v2.cpp   # diff two files
mastdiff src/engine.cpp                   # browse a single file's AST
mastdiff ./my_project/                    # browse a whole C++ project
```

---

## Features

### Text diff
- Side-by-side or unified view
- Character-level inline diff highlights (changed bytes within a line)
- Hunk navigation (`n` / `N`)
- Whitespace-ignore mode (`w`)
- Context-only view (`c`, configurable ± N lines)
- Export to `.patch` or `.html`

### AST diff
Select any line range in the text diff, press `Enter`, and see a **structured AST diff** for just that snippet — left and right trees aligned side-by-side with `Same` / `Added` / `Removed` / `Modified` status on every node. Collapse subtrees with `Space`, cycle AST filters (`f`), and jump back to the source line with `Enter`.

### Single-file AST browser
Open any `.cpp` file and explore its full AST in a split view: source lines on the left, tree nodes on the right. Select a line range and press `Enter` to zoom the AST to that slice.

### Project browser
Point mastdiff at a directory or at a build with `compile_commands.json` and it will discover all C++ translation units. Switch between views with `1`–`4`:

- **TUs** — source files with expandable associated headers (static `#include` analysis)
- **Sources** — flat list of `.cpp`/`.cc`/`.cxx` files
- **Headers** — flat list of `.h`/`.hpp`/`.hxx` files with back-reference count
- **CMake** — targets parsed from `CMakeLists.txt` with expandable source lists

```
j / k          navigate files
1/2/3/4        switch view (TUs / Sources / Headers / CMake)
Space          expand / collapse TU or CMake target
s              filter file list by name
Enter          open file in AST browser
g              open grep search
f              open AST / tree-sitter search
Ctrl+o         open selected file in configured external editor
q              quit
```

### Structural code search

Search the entire project with shorthand queries or raw tree-sitter S-expressions. The query bar **highlights recognised prefixes** in distinct colours so you know the keyword was parsed.

| Query | Finds |
|---|---|
| `fn:update` | all function definitions whose name contains `update` |
| `fn:` | every function definition in the project |
| `call:render` | every call to `render` (direct calls only, not nested in argument lists) |
| `call:` | every function call |
| `var:player` | variable declarations named `player` |
| `var:` | all variable declarations |
| `class:Entity` | class / struct / enum named `Entity` |
| `param:dt` | function parameters named `dt` |
| `field:velocity` | struct / class fields named `velocity` |
| `include:audio` | files that `#include` a path containing `audio` |
| `type:vec3` | uses of a type identifier containing `vec3` |
| `AudioBus` | plain-text grep (case-insensitive) across all files |
| `(call_expression function: (identifier) @fn)` | raw tree-sitter query |

Press **`Alt+R`** to toggle regex mode — the `[.*]` badge in the title bar turns orange when active. In regex mode the filter part of shorthand queries (`fn:upd.*`) and the full grep pattern are treated as regular expressions.

Results show filename, line number, `@capture_name`, and a source snippet. Navigate with `↑` / `↓`; press `Ctrl+o` to open the file at that exact line in your configured editor.

**VS Code-style file filters** — Tab through the include / exclude boxes below the query bar to restrict results by glob pattern (`src/**`, `*.cpp`, `tests/**`).

**Source text selection** — drag the mouse in the source pane to select text; it is copied to the clipboard automatically on release. `Ctrl+C` copies the current selection; `Ctrl+Y` copies the highlighted AST node's info.

### External editor integration

Press `Ctrl+o` in any view to open the current file at the current line in your configured editor. Works from:
- search results (opens at the exact match line + column)
- text diff (opens the left file at the cursor line)
- AST diff (opens at the source line of the highlighted AST node)
- single-file browser (opens at the cursor line)
- project browser (opens the selected file)

**vim** suspends the TUI, hands the terminal to vim, then restores the TUI when you quit. **VS Code** opens in the background while the TUI keeps running.

---

## Installation

### Pre-built binaries

Download the latest release binary for your platform from the [Releases](../../releases) page — no dependencies required.

| Platform | File |
|---|---|
| Linux x86_64 (static musl) | `mastdiff-linux-x86_64` |
| macOS arm64 (Apple Silicon) | `mastdiff-macos-aarch64` |
| macOS x86_64 (Intel) | `mastdiff-macos-x86_64` |
| Windows x86_64 | `mastdiff-windows-x86_64.exe` |

### Build from source

Requires Rust 1.85+.

```bash
git clone https://github.com/yourname/mastdiff
cd mastdiff
cargo build --release
# binary at target/release/mastdiff
```

---

## Configuration

Create `~/.config/mastdiff/config.toml` (XDG-aware; falls back to `~/.config` if `$XDG_CONFIG_HOME` is unset):

```toml
# Which editor Ctrl+o opens files in
open_in = "vim"      # "vim" (default) | "vscode"

# Minimum log level written to mastdiff.log in the working directory
log_level = "info"   # "off" | "error" | "warn" | "info" | "debug" | "trace"
```

Both settings are optional; the file itself is optional.

---

## Key bindings

### Text diff
| Key | Action |
|---|---|
| `j` / `k` or `↑` / `↓` | scroll |
| `n` / `N` | next / previous hunk |
| `s` | mark selection start |
| `e` | mark selection end |
| `Enter` | open AST diff for selection |
| `u` | toggle unified view |
| `c` | toggle context-only view |
| `w` | toggle whitespace-ignore |
| `i` | toggle inline char diff |
| `x` | export `.patch` |
| `X` | export `.html` |
| `Ctrl+o` | open left file at cursor in editor |
| `q` | quit |

### AST diff
| Key | Action |
|---|---|
| `j` / `k` | navigate nodes |
| `Space` | collapse / expand subtree |
| `f` | cycle filter (all → functions → classes → variables) |
| `Enter` | jump to source line in text diff |
| `Ctrl+o` | open file at node's source line in editor |
| `Esc` / `b` | back to text diff |

### Search
| Key | Action |
|---|---|
| type | edit query in search bar |
| `Tab` / `Shift+Tab` | cycle focus: query → include filter → exclude filter |
| `Enter` | run search |
| `↑` / `↓` | navigate results (shows live source + AST preview) |
| `Alt+R` | toggle regex mode |
| `drag` (mouse) | select text in source pane (auto-copies on release) |
| `Ctrl+C` | copy source selection to clipboard |
| `Ctrl+Y` | copy highlighted AST node to clipboard |
| `Ctrl+o` | open result in external editor |
| `?` | open keyboard reference |
| `Esc` | back |

---

## Logging

mastdiff writes structured logs to `mastdiff.log` in the working directory.

```
[     0.001s] [INFO ] config               config loaded: open_in=vscode log_level=debug
[     0.003s] [INFO ] project              project loader: directory walk of "src/"
[     0.015s] [INFO ] project              project loader: 12 translation units found
[     0.021s] [INFO ] app                  mode: ProjectBrowser → Search (ast)
[     0.022s] [DEBUG] search               parse_query: shorthand "fn:" filter="" nested_filter=false
[     0.031s] [INFO ] search               search: 47 total results across 12 files
[     0.031s] [DEBUG] search               search: 8 hits in src/renderer.cpp
```

Set `log_level = "trace"` to see every keystroke and nested-call filter decision.

---

## Project structure

```
src/
  main.rs           entry point, CLI, event loop, external editor handoff
  app.rs            all application state and key-event handlers
  config.rs         Config struct (open_in, log_level), loaded from TOML
  logger.rs         custom file logger with elapsed timestamps
  search.rs         tree-sitter query engine + grep + regex, SearchResult
  project.rs        project loader (compile_commands.json or directory walk)
  ast_diff.rs       tree-sitter parse, flatten, diff, collapse/filter helpers
  text_diff.rs      line-level and character-level diff (similar crate)
  syntax.rs         tree-sitter syntax highlighting for the source pane
  input.rs          single-line text field with byte-accurate cursor
  export.rs         .patch and .html export
  ui/
    diff_view.rs    text diff renderer
    ast_view.rs     AST diff renderer
    single_view.rs  single-file AST browser renderer
    project_view.rs project browser renderer (TUs / Sources / Headers / CMake)
    search_view.rs  search overlay renderer
    help_view.rs    keyboard reference overlay
examples/
  sample_project/   five-file C++ game engine used by integration tests
tests/
  integration.rs    23 integration tests against the sample project
```

---

## How tree-sitter search works

Each query is compiled once with `tree_sitter::Query::new()` and run with `QueryCursor::captures()`. The shorthand prefixes (`fn:`, `call:`, etc.) expand to pre-written S-expression queries with a `@match` capture; the text after the colon is used as a substring filter (or regex when `Alt+R` is active) applied only to the captured node's text, not the whole line.

All files are searched in parallel with Rayon. Results are deduplicated by byte offset (so multiple overlapping patterns in one query never double-count the same node) and sorted by file path then line number.

The `call:` shorthand additionally runs `call_is_nested()` on each captured node, which walks up the tree through `*_expression` ancestors: if it reaches an `argument_list` before any statement boundary, the call is a nested argument and is silently dropped.
