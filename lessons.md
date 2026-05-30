# Lessons from ripgrep

Studied BurntSushi/ripgrep (commit: shallow clone, May 2026) and applied the
following lessons to this codebase. Each entry describes the ripgrep pattern,
the problem it solves, and how it was applied in astdiff.

---

## 1. Logger should include source file and line number

**ripgrep pattern** (`crates/core/logger.rs`):
```rust
eprintln_locked!(
    "{}|{}|{}:{}: {}",
    record.level(), record.target(),
    file, line,           // ← record.file() + record.line()
    record.args()
);
```
ripgrep's logger always emits the Rust source location of the `log::` call
site. This makes it trivial to `grep` a log file and jump straight to the
code that emitted the message.

**Applied in** `src/logger.rs`: The `FileLogger` now appends `(src/foo.rs:42)`
to every line when the record carries location info:
```
[     0.031s] [DEBUG] search               (src/search.rs:219) per-file cap reached
```

---

## 2. Per-file result cap in parallel search

**ripgrep pattern** (`crates/core/flags/hiargs.rs`):
ripgrep exposes `--max-count N` which limits matches per file. Each parallel
worker stops feeding results to the printer once the limit is hit, keeping
memory bounded regardless of file size.

**Problem in astdiff**: The grep path had a hard cap of 500 per file, but the
tree-sitter query path had no limit at all. On a large codebase `fn:` could
accumulate tens of thousands of `AstLine` objects before displaying anything.

**Applied in** `src/search.rs`:
- Introduced `MAX_RESULTS_PER_FILE = 500` as a named constant shared by both
  grep and the ts-query loop.
- The ts-query `for` loop checks `out.len() >= MAX_RESULTS_PER_FILE` and
  breaks with a `log::debug!` message so the cap is visible in the log.

---

## 3. Non-fatal per-file errors must not abort the whole search

**ripgrep pattern** (`crates/core/main.rs`):
```rust
let search_result = match searcher.search(&haystack) {
    Ok(r) => r,
    Err(err) => {
        err_message!("{}: {}", haystack.path().display(), err);
        return WalkState::Continue;   // ← keep going
    }
};
```
ripgrep never panics or returns early from the walk when a single file fails.
It logs the error and continues to the next file.

**Applied in** `src/search.rs` (`search_file`): Already returns an empty
`Vec` on `fs::read_to_string` failure. Added a `log::warn!` so the error
is visible:
```rust
let Ok(src) = fs::read_to_string(file_path) else {
    log::warn!("cannot read {}: skipping", file_path);
    return vec![];
};
```

---

## 4. Every module deserves a `//!` doc header

**ripgrep pattern**: Every crate and every significant module in ripgrep opens
with a `/*!` block that explains in plain English:
- what the module does
- the central type or function
- the workflow / data flow at a high level

This makes `cargo doc` output genuinely useful and helps new contributors
orient themselves in seconds rather than minutes.

**Applied**: Added `//!` doc comments to all astdiff source modules:
- `src/search.rs` — query types, shorthand expansion, parallel execution
- `src/project.rs` — compile_commands vs walk, TranslationUnit ordering
- `src/ast_diff.rs` — parse→flatten→diff→visibility pipeline
- `src/text_diff.rs` — line diff, inline spans, hunk/context helpers
- `src/input.rs` — UTF-8 cursor invariant
- `src/logger.rs` — format description, silent-on-error guarantee
- `src/config.rs` — already had docs; expanded

---

## 5. Builder pattern for configuration structs

**ripgrep pattern** (`crates/searcher/src/searcher/mod.rs`):
```rust
pub struct SearcherBuilder { config: Config }

impl SearcherBuilder {
    pub fn new() -> Self { ... }
    pub fn line_terminator(&mut self, t: LineTerminator) -> &mut Self { ... }
    pub fn build(&self) -> Searcher { ... }
}
```
ripgrep keeps a private `Config` struct with `Default` impls and exposes it
through a `*Builder` that validates inputs and can evolve independently of
callers.

**Observation for astdiff**: `Config` is currently simple enough (two fields)
that a builder isn't worth the boilerplate. However, `SearchQuery` construction
is currently a raw struct literal scattered in several places. If astdiff gains
more query options (case sensitivity, file-type filters, depth limits) the
`SearchQuery` struct should gain a `SearchQueryBuilder`.

**Not applied yet** — flagged as a refactor to do when the third option is
added.

---

## 6. Display-safe snippet extraction must work on char boundaries

**ripgrep pattern** (`crates/printer/`):
ripgrep's printer works entirely in byte ranges returned by the matcher and
only converts to `&str` slices at validated char boundaries. It never calls
`&str[byte_range]` without first ensuring the range is char-aligned.

**Problem in astdiff** (`src/search.rs::make_snippet`): The original code did:
```rust
let start = col.saturating_sub(4);
line.chars().skip(start).take(48)...
```
Here `col` is a **byte** offset from tree-sitter, but `chars().skip(start)`
treats it as a **char** count. On ASCII-only code this is fine, but on any
source file with non-ASCII identifiers, type names, or string literals the
skip index would be wrong (too far or too short).

**Applied in** `src/search.rs`:
```rust
// col is a byte offset; convert to char count first
let col_char = line[..col.min(line.len())].chars().count();
let start_char = col_char.saturating_sub(4);
line.chars().skip(start_char).take(48)...
```
The slice `line[..col]` is safe because tree-sitter always returns byte
offsets that lie on UTF-8 char boundaries.

---

## 7. Atomic error tracking for exit codes

**ripgrep pattern** (`crates/core/messages.rs`):
```rust
static ERRORED: AtomicBool = AtomicBool::new(false);

macro_rules! err_message {
    ($($tt:tt)*) => {
        crate::messages::set_errored();
        message!($($tt)*);
    }
}
```
ripgrep uses a global atomic to record whether any non-fatal error occurred
during a run. The exit code is then `1` if there were matches, `2` if there
was an error, regardless of whether matches were also found. This gives
scripts reliable exit code semantics.

**Observation for astdiff**: astdiff is a TUI app, not a pipeline tool, so
exit codes are less critical. However the pattern is worth keeping in mind if
astdiff ever grows a `--search` one-shot mode that writes to stdout.

**Not applied** — noted for a future non-interactive mode.

---

## Summary of changes made

| File | Change |
|---|---|
| `src/logger.rs` | Added `file:line` to every log record (lesson 1) |
| `src/search.rs` | `MAX_RESULTS_PER_FILE` constant shared by ts-query and grep (lesson 2) |
| `src/search.rs` | `log::warn!` on unreadable file in `search_file` (lesson 3) |
| `src/search.rs` | Byte→char conversion in `make_snippet` (lesson 6) |
| `src/search.rs` | Module `//!` doc header (lesson 4) |
| `src/project.rs` | Module `//!` doc header (lesson 4) |
| `src/ast_diff.rs` | Module `//!` doc header (lesson 4) |
| `src/text_diff.rs` | Module `//!` doc header (lesson 4) |
| `src/input.rs` | Module `//!` doc header (lesson 4) |
| `src/logger.rs` | Module `//!` doc header (lesson 4) |
