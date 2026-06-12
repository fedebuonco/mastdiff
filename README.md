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
Side-by-side or unified view with character-level inline highlights, hunk navigation (`n`/`N`), whitespace-ignore mode (`w`), context-only view (`c`), and `.patch` / `.html` export.

### AST diff
Select any line range in the text diff, press `Enter`, and see a structured AST diff for that snippet — left and right trees aligned with `Same` / `Added` / `Removed` / `Modified` status on every node. Collapse subtrees with `Space`, cycle AST filters (`f`).

### Project browser
Point mastdiff at a directory or a build with `compile_commands.json` and it discovers all C++ translation units. The TUI opens immediately while files stream in. Switch views with `1`–`4`: **TUs**, **Sources**, **Headers**, **CMake**.

### Structural code search

Live search with a 300 ms debounce; results stream in as you type. Press `Enter` to trigger immediately, `Alt+R` to toggle regex mode.

| Query | Finds |
|---|---|
| `fn:update` | function definitions containing `update` |
| `fn:` | every function definition |
| `call:render` | calls to `render` (direct only, not nested in arguments) |
| `var:player` | variable declarations named `player` |
| `class:Entity` | class / struct / enum named `Entity` |
| `param:dt` | function parameters named `dt` |
| `field:velocity` | struct / class fields named `velocity` |
| `include:audio` | files that `#include` a path containing `audio` |
| `AudioBus` | plain-text grep (case-insensitive) |
| `(call_expression function: (identifier) @fn)` | raw tree-sitter S-expression |

Results show filename, line, capture name, and a source snippet. Tab through the include / exclude boxes to filter by glob (`src/**`, `*.cpp`). Navigate with `↑`/`↓`; `Ctrl+o` opens the exact line in your editor.

### Headless search (CLI)

The same search engine is scriptable — `--search` prints results and exits, no TUI:

```bash
mastdiff --search "fn:update" ./my_project/            # file:line:col: text
mastdiff --search "call:render" --json ./my_project/   # JSON Lines, for tooling
mastdiff --search "fn:" --include "src/**" --exclude "tests/**" .
```

Flags: `--json`, `--include <globs>`, `--exclude <globs>`, `--regex`, `--case-sensitive`.

### VS Code extension

[`editors/vscode/`](editors/vscode/) ships **mastdiff: Semantic C++ Search** — the structural search as a live picker inside VS Code. Press `Ctrl+Alt+F` (`Cmd+Alt+F` on macOS), type `call:render` or any query from the table above, and jump straight to a result. Context-menu actions search for calls to / definitions of the symbol under the cursor. The extension is a thin client over the headless CLI; see its [README](editors/vscode/README.md) for setup.

---

## Installation

### Pre-built binaries

Download the latest release from the [Releases](../../releases) page — no dependencies required.

| Platform | File |
|---|---|
| Linux x86_64 (musl) | `mastdiff-linux-x86_64` |
| macOS arm64 | `mastdiff-macos-aarch64` |
| macOS x86_64 | `mastdiff-macos-x86_64` |
| Windows x86_64 | `mastdiff-windows-x86_64.exe` |

### Build from source

Requires Rust 1.85+.

```bash
git clone https://github.com/fedebuonco/mastdiff
cd mastdiff
cargo build --release
# binary at target/release/mastdiff
```

---

## Configuration

Create `~/.config/mastdiff/config.toml` (respects `$XDG_CONFIG_HOME`). All settings are optional.

```toml
# External editor opened by Ctrl+o
open_in = "vim"          # "vim" (default) | "vscode"

# Minimum log level written to mastdiff.log
log_level = "info"       # "off" | "error" | "warn" | "info" | "debug" | "trace"

# In-memory AST parse cache cap (MiB). Set to 0 to disable.
# Warm-cache searches skip tree-sitter re-parsing for unchanged files.
ast_cache_mb = 128       # default: 128
```

---

## Key bindings

### Text diff
| Key | Action |
|---|---|
| `j`/`k`, `↑`/`↓` | scroll |
| `n`/`N` | next / previous hunk |
| `s` / `e` | mark selection start / end |
| `Enter` | open AST diff for selection |
| `u` | toggle unified view |
| `c` | toggle context-only view |
| `w` | toggle whitespace-ignore |
| `i` | toggle inline char diff |
| `x` / `X` | export `.patch` / `.html` |
| `Ctrl+o` | open in editor |
| `q` | quit |

### AST diff
| Key | Action |
|---|---|
| `j`/`k` | navigate nodes |
| `Space` | collapse / expand subtree |
| `f` | cycle filter (all → functions → classes → variables) |
| `Enter` | jump to source line |
| `Ctrl+o` | open at node's source line in editor |
| `Esc`/`b` | back to text diff |

### Search
| Key | Action |
|---|---|
| type | edit query — fires after 300 ms debounce |
| `Enter` | run immediately |
| `Tab`/`Shift+Tab` | cycle focus: query → include → exclude |
| `↑`/`↓` | navigate results |
| `Alt+R` | toggle regex mode |
| `Ctrl+C` | copy source selection |
| `Ctrl+Y` | copy highlighted AST node |
| `Ctrl+o` | open result in editor |
| `?` | keyboard reference |
| `Esc` | back |
