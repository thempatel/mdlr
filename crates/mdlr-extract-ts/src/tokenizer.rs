use mdlr_cpd::{FileTokens, NORMALIZED_ID, NORMALIZED_LIT, Token};
use std::path::PathBuf;

/// JavaScript/TypeScript keywords (ES2024 + TS keywords).
const JS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "module",
    "namespace",
    "new",
    "null",
    "of",
    "override",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unique",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Tokenize a JS/TS source file for CPD analysis.
///
/// Uses a simple scanner that:
/// - Recognizes keywords, identifiers, string/number/regex literals, operators, punctuation
/// - Strips comments (line and block) and whitespace
/// - Normalizes identifiers to $ID and literals to $LIT
/// - Respects `mdlr:ignore-start` / `mdlr:ignore-end` markers
pub fn tokenize_ts(
    source: &str,
    source_path: &str,
    generation_id: u64,
) -> FileTokens {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut line: u32 = 1;
    let mut col: u16 = 0;
    let mut ignoring = false;
    let mut tokens = Vec::new();

    while pos < len {
        let b = bytes[pos];

        // Newline
        if b == b'\n' {
            pos += 1;
            line += 1;
            col = 0;
            continue;
        }

        // Whitespace
        if b == b' ' || b == b'\t' || b == b'\r' {
            pos += 1;
            col += 1;
            continue;
        }

        // Line comment
        if pos + 1 < len && b == b'/' && bytes[pos + 1] == b'/' {
            let start = pos;
            pos += 2;
            while pos < len && bytes[pos] != b'\n' {
                pos += 1;
            }
            let comment = &source[start..pos];
            if comment.contains("mdlr:ignore-start") {
                ignoring = true;
            } else if comment.contains("mdlr:ignore-end") {
                ignoring = false;
            }
            continue;
        }

        // Block comment
        if pos + 1 < len && b == b'/' && bytes[pos + 1] == b'*' {
            let start = pos;
            pos += 2;
            while pos + 1 < len
                && !(bytes[pos] == b'*' && bytes[pos + 1] == b'/')
            {
                if bytes[pos] == b'\n' {
                    line += 1;
                    col = 0;
                } else {
                    col += 1;
                }
                pos += 1;
            }
            if pos + 1 < len {
                pos += 2; // skip */
                col += 2;
            }
            let comment = &source[start..pos];
            if comment.contains("mdlr:ignore-start") {
                ignoring = true;
            } else if comment.contains("mdlr:ignore-end") {
                ignoring = false;
            }
            continue;
        }

        // Skip tokens in ignored regions (but still track newlines for line counting)
        if ignoring {
            if b == b'\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            pos += 1;
            continue;
        }

        let token_line = line;
        let token_col = col;

        // String literal (single or double quoted)
        if b == b'\'' || b == b'"' {
            let quote = b;
            pos += 1;
            col += 1;
            while pos < len && bytes[pos] != quote {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 2;
                    col += 2;
                } else {
                    if bytes[pos] == b'\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                    pos += 1;
                }
            }
            if pos < len {
                pos += 1; // skip closing quote
                col += 1;
            }
            tokens.push(Token {
                value: NORMALIZED_LIT.to_string(),
                line: token_line,
                col: token_col,
            });
            continue;
        }

        // Template literal
        if b == b'`' {
            pos += 1;
            col += 1;
            let mut depth = 0;
            while pos < len {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 2;
                    col += 2;
                } else if bytes[pos] == b'$'
                    && pos + 1 < len
                    && bytes[pos + 1] == b'{'
                {
                    depth += 1;
                    pos += 2;
                    col += 2;
                } else if bytes[pos] == b'}' && depth > 0 {
                    depth -= 1;
                    pos += 1;
                    col += 1;
                } else if bytes[pos] == b'`' && depth == 0 {
                    pos += 1;
                    col += 1;
                    break;
                } else {
                    if bytes[pos] == b'\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                    pos += 1;
                }
            }
            tokens.push(Token {
                value: NORMALIZED_LIT.to_string(),
                line: token_line,
                col: token_col,
            });
            continue;
        }

        // Number literal
        if b.is_ascii_digit()
            || (b == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            while pos < len
                && (bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'.'
                    || bytes[pos] == b'_')
            {
                pos += 1;
                col += 1;
            }
            tokens.push(Token {
                value: NORMALIZED_LIT.to_string(),
                line: token_line,
                col: token_col,
            });
            continue;
        }

        // Identifier or keyword
        if is_ident_start(b) {
            let start = pos;
            pos += 1;
            col += 1;
            while pos < len && is_ident_continue(bytes[pos]) {
                pos += 1;
                col += 1;
            }
            let word = &source[start..pos];
            let value = if JS_KEYWORDS.contains(&word) {
                word.to_string()
            } else {
                NORMALIZED_ID.to_string()
            };
            tokens.push(Token { value, line: token_line, col: token_col });
            continue;
        }

        // Multi-character operators
        if pos + 2 < len {
            let three = &source[pos..pos + 3];
            if matches!(
                three,
                "==="
                    | "!=="
                    | ">>>"
                    | "**="
                    | "&&="
                    | "||="
                    | "??="
                    | "..."
                    | "<<="
                    | ">>="
            ) {
                tokens.push(Token {
                    value: three.to_string(),
                    line: token_line,
                    col: token_col,
                });
                pos += 3;
                col += 3;
                continue;
            }
        }
        if pos + 1 < len {
            let two = &source[pos..pos + 2];
            if matches!(
                two,
                "==" | "!="
                    | "<="
                    | ">="
                    | "&&"
                    | "||"
                    | "++"
                    | "--"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "=>"
                    | "**"
                    | "??"
                    | "?."
                    | "<<"
                    | ">>"
                    | "&="
                    | "|="
                    | "^="
            ) {
                tokens.push(Token {
                    value: two.to_string(),
                    line: token_line,
                    col: token_col,
                });
                pos += 2;
                col += 2;
                continue;
            }
        }

        // Single-character tokens (operators, punctuation)
        let ch = &source[pos..pos + 1];
        tokens.push(Token {
            value: ch.to_string(),
            line: token_line,
            col: token_col,
        });
        pos += 1;
        col += 1;
    }

    FileTokens {
        source_path: PathBuf::from(source_path),
        tokens,
        cached_at: generation_id,
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_js() {
        let source = r#"function foo(x) {
    return x + 1;
}"#;
        let ft = tokenize_ts(source, "test.js", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "function", "$ID", "(", "$ID", ")", "{", "return", "$ID", "+",
                "$LIT", ";", "}"
            ]
        );
    }

    #[test]
    fn test_comments_stripped() {
        let source = r#"// comment
const x = 5; /* block comment */
const y = 10;"#;
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "const", "$ID", "=", "$LIT", ";", "const", "$ID", "=", "$LIT",
                ";"
            ]
        );
    }

    #[test]
    fn test_string_literals() {
        let source = r#"const a = "hello"; const b = 'world';"#;
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "const", "$ID", "=", "$LIT", ";", "const", "$ID", "=", "$LIT",
                ";"
            ]
        );
    }

    #[test]
    fn test_template_literal() {
        let source = "const s = `hello ${name}`;";
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["const", "$ID", "=", "$LIT", ";"]);
    }

    #[test]
    fn test_ignore_markers() {
        let source = r#"const a = 1;
// mdlr:ignore-start
const ignored = 2;
// mdlr:ignore-end
const b = 3;"#;
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "const", "$ID", "=", "$LIT", ";", "const", "$ID", "=", "$LIT",
                ";"
            ]
        );
    }

    #[test]
    fn test_arrow_function() {
        let source = "const f = (x) => x * 2;";
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "const", "$ID", "=", "(", "$ID", ")", "=>", "$ID", "*",
                "$LIT", ";"
            ]
        );
    }

    #[test]
    fn test_typescript_keywords() {
        let source = "interface Foo { readonly bar: string; }";
        let ft = tokenize_ts(source, "test.ts", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "interface",
                "$ID",
                "{",
                "readonly",
                "$ID",
                ":",
                "$ID",
                ";",
                "}"
            ]
        );
        // Note: "string" is not in our keywords list, so it's $ID. This is fine
        // for CPD since we normalize identifiers anyway.
    }
}
