# Project Discovery (`src/project.rs`)

The project module is responsible for finding every C++ file in a directory,
associating headers with their source files, and parsing CMake targets.
It runs entirely in a background thread and streams results to the UI.

---

## Public types

### `TranslationUnit`
```rust
pub struct TranslationUnit {
    pub file_path:           String,
    pub file_size:           u64,
    pub is_header:           bool,       // .h/.hpp/.hxx/.H
    pub associated_headers:  Vec<String>, // headers #include'd by this source
    pub included_by:         Vec<String>, // sources that #include this header
}
```

### `CmakeTarget`
```rust
pub struct CmakeTarget {
    pub name:    String,   // target name from add_executable / add_library
    pub kind:    String,   // "executable" or "library"
    pub sources: Vec<String>, // source paths listed literally in the call
}
```

### `ProjectData`
Returned by `load()` and sent as the final `LoadMsg::Finished` message.
Contains `files: Vec<TranslationUnit>` and `cmake_targets: Vec<CmakeTarget>`.

### `LoadMsg`
```rust
pub enum LoadMsg {
    FilesBatch(Vec<TranslationUnit>),  // partial batch for progressive display
    Finished(ProjectData),             // final complete data
}
```

---

## Discovery algorithm (`load_streaming`)

```
1. Walk directory with walkdir (follows symlinks, skips hidden dirs).
2. Collect .cpp/.cc/.cxx/.C files → sources
   Collect .h/.hpp/.hxx/.H files  → headers
3. Build header_map: basename → absolute_path (for #include resolution).
4. For each source file:
   a. Parse with tree-sitter using QUERY_INCLUDE to extract #include "..." paths.
   b. Resolve each include string against header_map.
   c. Populate associated_headers.
   d. Send a FilesBatch every 50 files for progressive display.
5. Build reverse index: for each source→header edge, push source path into
   header.included_by.
6. Optionally load compile_commands.json from common build dirs
   (build/, cmake-build-debug/, cmake-build-release/, out/build/, …).
   If found, enrich file_size from the JSON (it already has the path list
   but sizes are not stored in compile_commands — this step is a no-op
   for sizes; it mainly confirms the file list is compile_commands-derived).
7. Parse all CMakeLists.txt files found during the walk.
8. Send LoadMsg::Finished(ProjectData { files, cmake_targets }).
```

### CMake parsing
A simple regex-based parser reads `add_executable(name src1 src2 …)` and
`add_library(name [STATIC|SHARED|…] src1 …)` calls.  It does **not** evaluate
CMake variables, so targets that use `${SOURCES}` variable expansion will have
an empty sources list — only literal paths are captured.

---

## `ProjectRow` (in `app.rs`)

The project browser does not display `Vec<TranslationUnit>` directly.
Instead `App::rebuild_project_display()` flattens the data into a
`Vec<ProjectRow>` that the renderer iterates linearly:

```rust
pub enum ProjectRow {
    Source(usize),                          // index into project_files
    Header { parent_idx, path, size },      // expanded child of a Source
    LooseHeader(usize),                     // header in Headers view
    CmakeTarget(usize),                     // index into cmake_targets
    CmakeSource { tgt_idx, path },          // expanded child of a CmakeTarget
}
```

`rebuild_project_display` is called whenever:
- The view switches (1–4 / Tab)
- The filter string changes
- A row is expanded/collapsed (Space)
- A `LoadMsg` batch arrives

The cursor is clamped to `project_display.len().saturating_sub(1)` after each
rebuild so it never points past the end.
