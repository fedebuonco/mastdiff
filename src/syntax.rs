//! Syntax highlighting via tree-sitter.
//!
//! [`highlight`] parses a C++ source string and returns a per-line list of
//! [`SyntaxSpan`]s — char-offset ranges paired with a display colour.  Only
//! leaf nodes (no children) are emitted; multi-line spans (block comments,
//! raw strings) are split at line boundaries.
//!
//! The colour palette intentionally mirrors VS Code's Dark+ theme so the
//! output feels familiar to C++ developers.

use ratatui::style::Color;
use tree_sitter::Parser;

// ── Public types ──────────────────────────────────────────────────────────

/// A coloured sub-range of a single source line.
/// `start` and `end` are **char** offsets (not bytes) within the line.
#[derive(Clone, Debug)]
pub struct SyntaxSpan {
    pub start: usize,
    pub end:   usize,
    pub color: Color,
}

/// Parse `src` and return one `Vec<SyntaxSpan>` per line.
/// Returns an empty outer vec on parse failure.
pub fn highlight(src: &str) -> Vec<Vec<SyntaxSpan>> {
    let mut parser = Parser::new();
    let language   = tree_sitter_cpp::language();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(src, None) else {
        return vec![];
    };

    // Precompute byte offset of the start of each line.
    let mut line_starts: Vec<usize> = vec![0];
    for (i, &b) in src.as_bytes().iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let n_lines = line_starts.len();
    let mut out: Vec<Vec<SyntaxSpan>> = vec![vec![]; n_lines];

    // Walk only leaf nodes (child_count == 0).
    let mut cursor = tree.walk();
    'outer: loop {
        let node = cursor.node();
        if node.child_count() == 0 {
            let Some(color) = kind_color(node.kind(), !node.is_named()) else {
                // advance past this leaf
                loop {
                    if cursor.goto_next_sibling() { continue 'outer; }
                    if !cursor.goto_parent() { break 'outer; }
                }
            };
            let start_byte = node.start_byte();
            let end_byte   = node.end_byte();
            if start_byte >= end_byte { /* empty node */ } else {
                emit_span(src, &line_starts, start_byte, end_byte, color, &mut out);
            }
            loop {
                if cursor.goto_next_sibling() { continue 'outer; }
                if !cursor.goto_parent() { break 'outer; }
            }
        } else if !cursor.goto_first_child() {
            loop {
                if cursor.goto_next_sibling() { continue 'outer; }
                if !cursor.goto_parent() { break 'outer; }
            }
        }
    }

    out
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Emit one span per covered line for a byte range.
fn emit_span(
    src: &str,
    line_starts: &[usize],
    start_byte: usize,
    end_byte: usize,
    color: Color,
    out: &mut [Vec<SyntaxSpan>],
) {
    // Binary-search for the first line containing start_byte.
    let first_line = line_starts.partition_point(|&ls| ls <= start_byte).saturating_sub(1);
    let last_line  = line_starts.partition_point(|&ls| ls < end_byte).saturating_sub(1);

    for line_idx in first_line..=last_line {
        if line_idx >= out.len() { break; }
        let ls = line_starts[line_idx];
        let le = line_starts.get(line_idx + 1).copied().unwrap_or(src.len());
        // Clamp byte span to this line (exclude trailing \n).
        let span_start = start_byte.max(ls);
        let span_end   = end_byte.min(le).min(if le > 0 && src.as_bytes().get(le - 1) == Some(&b'\n') { le - 1 } else { le });
        if span_start >= span_end { continue; }

        let line_src = &src[ls..le.min(src.len())];
        let char_start = line_src[..span_start - ls].chars().count();
        let char_end   = char_start + line_src[span_start - ls..span_end - ls].chars().count();
        if char_start < char_end {
            out[line_idx].push(SyntaxSpan { start: char_start, end: char_end, color });
        }
    }
}

/// Map a tree-sitter node kind to a display colour.
/// Returns `None` for "uninteresting" nodes (plain identifiers, punctuation
/// that should stay at the default terminal colour).
fn kind_color(kind: &str, is_anonymous: bool) -> Option<Color> {
    // Named structural kinds first.
    match kind {
        // Comments
        "comment" =>
            return Some(Color::Rgb(106, 153, 85)),

        // String / character literals
        "string_literal" | "raw_string_literal" | "char_literal" | "system_lib_string" =>
            return Some(Color::Rgb(206, 145, 120)),

        // Numeric literals
        "number_literal" | "integer_literal" | "float_literal" =>
            return Some(Color::Rgb(181, 206, 168)),

        // Type names (user-defined)
        "type_identifier" =>
            return Some(Color::Rgb(78, 201, 176)),

        // Built-in types and qualifiers that tree-sitter marks as named
        "primitive_type" | "type_qualifier" | "storage_class_specifier" =>
            return Some(Color::Rgb(86, 156, 214)),

        // Namespace / scope identifiers look like types
        "namespace_identifier" =>
            return Some(Color::Rgb(78, 201, 176)),

        // Field names in member access
        "field_identifier" =>
            return Some(Color::Rgb(156, 220, 254)),

        // Plain identifier — leave at terminal default
        "identifier" => return None,

        _ => {}
    }

    // Anonymous nodes carry the literal text as their `kind`.
    if is_anonymous {
        return match kind {
            // C++ keywords → purple (VS Code keyword colour)
            "if" | "else" | "while" | "for" | "do" | "switch" | "case" | "default"
            | "return" | "break" | "continue" | "goto"
            | "class" | "struct" | "union" | "enum" | "namespace" | "template"
            | "typename" | "new" | "delete" | "this" | "nullptr" | "true" | "false"
            | "const" | "constexpr" | "consteval" | "constinit"
            | "static" | "virtual" | "override" | "final" | "explicit" | "inline"
            | "extern" | "volatile" | "mutable" | "register"
            | "public" | "private" | "protected"
            | "void" | "int" | "char" | "bool" | "float" | "double"
            | "long" | "short" | "unsigned" | "signed" | "auto"
            | "sizeof" | "alignof" | "typeof" | "decltype" | "noexcept"
            | "typedef" | "using" | "operator" | "friend" | "try" | "catch" | "throw"
            | "#include" | "#define" | "#ifdef" | "#ifndef" | "#endif"
            | "#if" | "#else" | "#elif" | "#pragma"
                => Some(Color::Rgb(197, 134, 192)),

            // Operators that deserve slight emphasis
            "+" | "-" | "*" | "/" | "%" | "=" | "==" | "!=" | "<" | ">" | "<=" | ">="
            | "&&" | "||" | "!" | "&" | "|" | "^" | "~" | "<<" | ">>"
            | "+=" | "-=" | "*=" | "/=" | "++" | "--" | "->" | "::"
                => Some(Color::Rgb(180, 180, 200)),

            // Punctuation — de-emphasise slightly
            "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "?"
                => Some(Color::Rgb(100, 100, 120)),

            _ => None,
        };
    }

    None
}
