# AST Parsing, Diff, and Search

---

## `src/ast_diff.rs` — AST parsing and diffing

### `AstLine`

The fundamental unit of the AST representation.  Both the search view and
the diff view work with `Vec<AstLine>`.

```rust
pub struct AstLine {
    pub depth:          usize,           // nesting level (0 = root)
    pub kind:           String,          // tree-sitter node kind, e.g. "function_definition"
    pub leaf_text:      Option<String>,  // display text for leaf nodes (None for internal nodes)
    pub status:         NodeStatus,      // Same | Added | Removed | Modified
    pub empty:          bool,            // padding row for diff alignment (no real node)
    pub source_row:     usize,           // first source line (0-indexed, relative to snippet)
    pub source_end_row: usize,           // last source line (inclusive)
}
```

`leaf_text` for named leaf nodes is the UTF-8 source slice with newlines
replaced by `↵` and tabs by `→` so that the display stays on one line.
Strings are wrapped in `"..."`.

### Parsing pipeline

```
src text
  └─ tree_sitter::Parser::parse()  →  tree_sitter::Tree
       └─ flatten_tree(root, src)  →  Vec<FlatNode>
            └─ parse_single()      →  Vec<AstLine>
```

`flatten_node` performs a pre-order DFS.  It skips anonymous nodes that
have children (punctuation such as `{`, `;`) because they add noise without
value.  Named leaf nodes (identifiers, literals) capture their source text.

### Diff pipeline (`compute_ast_diff`)

```
left_src + right_src
  → parse both into Vec<FlatNode>
  → serialize each FlatNode to a one-line string (depth|kind|leaf_text)
  → run similar::TextDiff (LCS) on the two serialised lists
  → re-inflate the diff output back into Vec<AstLine> pairs,
    padding the shorter side with empty AstLines so indices align
  → return AstDiffResult { left_nodes, right_nodes, left_source_start, … }
```

The LCS approach means that matching nodes have the same index in both
`left_nodes` and `right_nodes`, making side-by-side rendering trivial.

### Visibility helpers

| Function | Purpose |
|---|---|
| `visible_rows(left, right, collapsed)` | Returns `Vec<usize>` of raw indices that are not hidden by a collapsed ancestor |
| `row_has_children(raw, left, right)` | True if any node with a higher depth immediately follows `raw` |
| `filter_rows(nodes, keyword)` | Returns indices of nodes whose `kind` contains `keyword` |

These functions are **pure** — they do not mutate `App` state.
`App` stores the results in `ast_visible` / `ast_filter_rows` and refreshes
them by calling `rebuild_ast_visible()` whenever collapse or filter changes.

---

## `src/search.rs` — Code search engine

### `SearchQuery`

```rust
pub enum SearchQuery {
    TsQuery {
        ts_source: String,       // raw S-expression forwarded to tree-sitter
        text_filter: String,     // additional substring filter on the captured text
        use_regex: bool,
        case_sensitive: bool,
    },
    Grep {
        pattern: String,
        use_regex: bool,
        case_sensitive: bool,
    },
}
```

Produced by `parse_query(input: &str)` from the raw text the user typed.

### `parse_query` — shorthand expansion

`parse_query` checks the input against a table of `(prefix, ts_query,
needs_text_filter)` triples.  When a match is found the ts_query is used
verbatim (it always captures `@match`).  If `needs_text_filter` is true, the
text after the colon becomes the `text_filter`.

Current shorthands:

| Prefix | Matches | text_filter? |
|---|---|---|
| `fn:<name>` | function definitions | yes |
| `call:<name>` | call sites | yes |
| `var:<name>` | variable declarations | yes |
| `class:<name>` | class / struct / enum definitions | yes |
| `field:<name>` | struct/class field declarations | yes |
| `param:<name>` | function parameters | yes |
| `type:<name>` | type identifiers | yes |
| `include:<hdr>` | `#include` directives | yes |
| `lambda:` | lambda expressions | no |
| `macro:<name>` | `#define` / macro definitions | yes |
| `ns:<name>` | namespace definitions | yes |
| `op:<sym>` | operator overloads | yes |
| `using:<name>` | using declarations | yes |
| `tpl:<name>` | template function/class declarations | yes |
| `throw:` | throw statements | no |
| `cast:<type>` | cast expressions (C-style + static/dynamic/…) | yes |

If no shorthand matches and the input starts with `(` or `[`, it is treated
as a raw tree-sitter S-expression query (the user writes it in full).
Otherwise, plain grep mode.

### `SearchResult`

```rust
pub struct SearchResult {
    pub file:    String,   // absolute path
    pub line:    usize,    // 0-indexed line of the match
    pub col:     usize,    // 0-indexed char column
    pub snippet: String,   // the matched line text
}
```

### `search_project_streaming`

Parallelised over files using Rayon.  Each file is loaded once; the query
is applied; matching results are sent in batches through the `mpsc::Sender`.
The `Arc<AtomicBool>` cancel flag is checked at the start of each file so
that a new search can preempt the current one with minimal latency.

For tree-sitter queries, a `QueryCursor` is created once per file.
For grep, a `regex::Regex` is compiled once outside the parallel loop and
shared (it is `Send + Sync`).

### `FileFilter`

Glob patterns from `search_include` / `search_exclude` are compiled into a
`FileFilter` struct before the parallel search begins.  Each file path is
matched against the include list (if non-empty) and rejected if matched by
the exclude list.

---

## `src/syntax.rs` — Syntax highlighting

`highlight(src: &str) -> Vec<Vec<SyntaxSpan>>` returns one `Vec<SyntaxSpan>`
per source line.  Each `SyntaxSpan` carries a byte range and a `Color`.

The highlighter is a simple hand-written regex-based classifier — not a full
tree-sitter highlight query — so it stays fast on large files and requires no
highlight grammar.  It recognises:

- C++ keywords
- String and character literals
- Comments (`//` and `/* … */`)
- Preprocessor directives
- Numeric literals
- Type-like identifiers (capitalized words)

The spans are consumed by `search_view::render_source_pane` which merges them
with the selection highlight.
