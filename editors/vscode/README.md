# mastdiff: Semantic C++ Search for VS Code

A better search for C++ codebases. Instead of plain text matching, queries run
as native [tree-sitter](https://tree-sitter.github.io/) queries over every
translation unit in your workspace — so you can search for *function
definitions*, *call sites*, *class declarations*, *fields*, and any other
syntactic structure, not just strings.

The extension is a thin client around the [mastdiff](https://github.com/fedebuonco/mastdiff)
CLI, which does the parallel parsing and querying natively.

## Requirements

The `mastdiff` binary must be installed — grab a pre-built binary from the
[releases page](https://github.com/fedebuonco/mastdiff/releases) or
`cargo build --release` from the repo root. If it's not on your `PATH`, set
`mastdiff.binaryPath`.

## Usage

Press `Ctrl+Alt+F` (`Cmd+Alt+F` on macOS) or run **mastdiff: Semantic C++
Search** from the command palette, then type a query. Results stream in live
as you type; `Enter` jumps to the selected result.

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
| `lambda:` | every lambda expression |
| `macro:LOG` | `#define`s named `LOG` |
| `ns:detail` | namespace definitions named `detail` |
| `op:==` | operator overload definitions |
| `tpl:vector` | template functions / classes |
| `throw:` | every throw statement |
| `cast:` | C-style and C++ casts |
| `AudioBus` | plain-text grep (case-insensitive) |
| `(call_expression function: (identifier) @fn)` | raw tree-sitter S-expression |

Two context-menu / palette shortcuts pre-fill the query from the symbol under
the cursor:

- **mastdiff: Find Calls to Symbol Under Cursor** → `call:<word>`
- **mastdiff: Find Function Definitions of Symbol Under Cursor** → `fn:<word>`

## Settings

| Setting | Default | Description |
|---|---|---|
| `mastdiff.binaryPath` | `mastdiff` | Path to the mastdiff binary |
| `mastdiff.include` | _(empty)_ | Comma-separated include globs, e.g. `src/**,*.hpp` |
| `mastdiff.exclude` | _(empty)_ | Comma-separated exclude globs, e.g. `tests/**,vendor/**` |
| `mastdiff.debounceMs` | `300` | Debounce before a search fires while typing |
| `mastdiff.maxResults` | `500` | Maximum results shown in the picker |
| `mastdiff.previewResults` | `true` | Preview the highlighted result while navigating |

## Building from source

```bash
cd editors/vscode
npm install
npm run compile
# then press F5 in VS Code to launch an Extension Development Host,
# or package with: npx @vscode/vsce package
```
