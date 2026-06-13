# Extension Guide

How to add new features to the most common extension points.

---

## Adding a new search shorthand

Search shorthands live in two files: `src/search.rs` and
`src/ui/search_view.rs`.

### 1. Write the tree-sitter query (`src/search.rs`)

Add a `const QUERY_XXX: &str = "..."` constant.  The query must capture
exactly one group named `@match`.  Multiple top-level patterns are fine —
tree-sitter unions them automatically.

Example — matching `static_assert` calls:
```rust
const QUERY_STATIC_ASSERT: &str = "(static_assert_declaration condition: _ @match)";
```

### 2. Register the shorthand (`src/search.rs`)

Add a row to the `INDEX_BUCKETS` table — the single source of truth used by
both `parse_query()` and the persistent symbol cache:
```rust
(16, "sassert:", QUERY_STATIC_ASSERT, false),
//   ^bucket id   ^prefix   ^ts_query   ^filter_nested_calls
```

The bucket id must be **unique and appended, never renumbered** — it is
embedded in the on-disk symbol cache. (Renumbering or changing a bucket's
query means bumping `SCHEMA` in `symcache.rs` so old caches are discarded.)

`filter_nested_calls` is a `call:`-specific flag (skip calls nested inside
another call's argument list); leave it `false` for normal shorthands.

That's all that's needed — the text after the colon is automatically used as
a substring filter against the captured node's text (e.g. `fn:myFunc`), and
the new bucket flows into the symbol cache's combined extraction query for
free. A query whose node types are absent from the bundled grammar simply
won't compile and is skipped (and returns nothing live), so no schema bump is
required for grammar-unsupported additions.

### 3. Colour-code the hint chip (`src/ui/search_view.rs`)

Find the `SHORTHANDS` constant (a `&[(&str, Color)]` slice) and add an entry
for your new prefix with a distinctive colour:
```rust
("sassert:", Color::Rgb(255, 160, 100)),
```

The search bar builds chip spans by iterating this slice, so your prefix will
appear automatically with the right colour.

### 4. Update the help page (`src/ui/help_view.rs`)

In `left_column()` add a `row!` call in the `SEARCH — AST operators` section:
```rust
row(&mut lines, "sassert:",  "static_assert conditions");
```

---

## Adding a new AST visualisation mode

### 1. Add a variant to `AstVizMode` (`src/app.rs`)

```rust
pub enum AstVizMode {
    Tree,
    Timeline,
    MyNewViz,    // ← add here
}
```

Update `label()` and `next()` to include the new variant:
```rust
fn label(&self) -> &str {
    match self {
        Self::Tree      => "Tree",
        Self::Timeline  => "Timeline",
        Self::MyNewViz  => "MyNewViz",
    }
}
fn next(&self) -> Self {
    match self {
        Self::Tree      => Self::Timeline,
        Self::Timeline  => Self::MyNewViz,
        Self::MyNewViz  => Self::Tree,
    }
}
```

### 2. Write the renderer (`src/ui/search_view.rs`)

Add a function:
```rust
fn render_ast_my_new_viz(f: &mut Frame, app: &App, area: Rect) {
    // draw into `area` using app.search_ast_nodes, app.search_ast_scroll, etc.
}
```

Dispatch it inside `render_ast_pane`:
```rust
AstVizMode::MyNewViz => render_ast_my_new_viz(f, app, area),
```

### 3. Handle mouse clicks (optional) (`src/app.rs`)

In `handle_ast_viz_click` add a branch:
```rust
if self.search_ast_viz == AstVizMode::MyNewViz {
    // map (cx, cy) → raw node index, call highlight_ast_node_in_source(raw)
    return;
}
```

### 4. Update the help page

In `right_column()` update the `v` key description to include your new mode.

---

## Adding a new `AppMode`

1. Add the variant to `AppMode` in `src/app.rs`.
2. Add an `on_<mode>` key handler method in `impl App`.
3. Add a dispatch arm in `handle_key` and `handle_mouse`.
4. Add a renderer in `src/ui/` and dispatch it from `ui/mod.rs::render`.
5. Handle `?` to open Help and return to the new mode:
   the `help_prev_mode` pattern already handles this — just make sure
   you set `app.help_prev_mode = self.mode.clone()` before switching.

---

## Changing the 50 ms poll interval

The main loop uses:
```rust
if event::poll(Duration::from_millis(50))? { … }
```
Increasing this number reduces CPU use but slows down input responsiveness
and spinner animation.  Decreasing it increases CPU use.

The search debounce is hardcoded to 300 ms in `nudge_search_debounce`.
Adjust that constant independently if needed.

---

## Adding a new config key

1. Add a field to `Config` in `src/config.rs` with `#[serde(default)]`.
2. Add a corresponding type (or reuse `String` / `bool`).
3. Access it anywhere via `app.config.your_field`.
4. Document the key in `src/ui/help_view.rs` under `CONFIG`.
