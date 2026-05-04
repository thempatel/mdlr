use mdlr_cpd::{FileTokens, NORMALIZED_ID, NORMALIZED_LIT, Token};
use std::path::PathBuf;

/// Python keywords.
const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break",
    "class", "continue", "def", "del", "elif", "else", "except", "finally",
    "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal",
    "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
    // Soft keywords
    "match", "case", "type",
];

/// Tokenize a Python source file for CPD analysis.
///
/// - Strips comments and whitespace (including indentation tokens)
/// - Normalizes identifiers to $ID and literals to $LIT
/// - Respects `mdlr:ignore-start` / `mdlr:ignore-end` markers
pub fn tokenize_py(
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

        // Carriage return
        if b == b'\r' {
            pos += 1;
            if pos < len && bytes[pos] == b'\n' {
                pos += 1;
            }
            line += 1;
            col = 0;
            continue;
        }

        // Whitespace
        if b == b' ' || b == b'\t' {
            pos += 1;
            col += 1;
            continue;
        }

        // Line continuation
        if b == b'\\' && pos + 1 < len && bytes[pos + 1] == b'\n' {
            pos += 2;
            line += 1;
            col = 0;
            continue;
        }

        // Comment
        if b == b'#' {
            let start = pos;
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

        // Skip tokens in ignored regions
        if ignoring {
            pos += 1;
            col += 1;
            continue;
        }

        let token_line = line;
        let token_col = col;

        // Triple-quoted string (""" or ''')
        if pos + 2 < len
            && ((b == b'"'
                && bytes[pos + 1] == b'"'
                && bytes[pos + 2] == b'"')
                || (b == b'\''
                    && bytes[pos + 1] == b'\''
                    && bytes[pos + 2] == b'\''))
        {
            let quote = b;
            pos += 3;
            col += 3;
            while pos + 2 < len {
                if bytes[pos] == quote
                    && bytes[pos + 1] == quote
                    && bytes[pos + 2] == quote
                {
                    pos += 3;
                    col += 3;
                    break;
                }
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
            tokens.push(Token {
                value: NORMALIZED_LIT.to_string(),
                line: token_line,
                col: token_col,
            });
            continue;
        }

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
                        // Unterminated string — break
                        break;
                    }
                    col += 1;
                    pos += 1;
                }
            }
            if pos < len && bytes[pos] == quote {
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

        // Number literal
        if b.is_ascii_digit()
            || (b == b'.' && pos + 1 < len && bytes[pos + 1].is_ascii_digit())
        {
            while pos < len
                && (bytes[pos].is_ascii_alphanumeric()
                    || bytes[pos] == b'.'
                    || bytes[pos] == b'_'
                    || bytes[pos] == b'+'
                    || bytes[pos] == b'-')
            {
                // Handle exponent sign: only allow +/- after e/E
                if (bytes[pos] == b'+' || bytes[pos] == b'-')
                    && pos > 0
                    && !matches!(bytes[pos - 1], b'e' | b'E')
                {
                    break;
                }
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

        // Identifier or keyword (including string prefixes like f"...", b"...", r"...")
        if is_ident_start(b) {
            let start = pos;
            pos += 1;
            col += 1;
            while pos < len && is_ident_continue(bytes[pos]) {
                pos += 1;
                col += 1;
            }
            let word = &source[start..pos];

            // Check for string prefixes (f, b, r, rb, br, etc.) followed by quote
            if pos < len
                && (bytes[pos] == b'"' || bytes[pos] == b'\'')
                && is_string_prefix(word)
            {
                // This is a prefixed string — treat the whole thing as a literal
                // Re-position to the quote and let the string handler above run next iter?
                // Actually, just consume the string here.
                let quote = bytes[pos];
                // Check for triple-quoted
                if pos + 2 < len
                    && bytes[pos + 1] == quote
                    && bytes[pos + 2] == quote
                {
                    pos += 3;
                    col += 3;
                    while pos + 2 < len {
                        if bytes[pos] == quote
                            && bytes[pos + 1] == quote
                            && bytes[pos + 2] == quote
                        {
                            pos += 3;
                            col += 3;
                            break;
                        }
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
                } else {
                    pos += 1;
                    col += 1;
                    while pos < len && bytes[pos] != quote {
                        if bytes[pos] == b'\\' && pos + 1 < len {
                            pos += 2;
                            col += 2;
                        } else {
                            if bytes[pos] == b'\n' {
                                break;
                            }
                            col += 1;
                            pos += 1;
                        }
                    }
                    if pos < len && bytes[pos] == quote {
                        pos += 1;
                        col += 1;
                    }
                }
                tokens.push(Token {
                    value: NORMALIZED_LIT.to_string(),
                    line: token_line,
                    col: token_col,
                });
                continue;
            }

            let value = if PY_KEYWORDS.contains(&word) {
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
            if matches!(three, "**=" | "//=" | ">>=" | "<<=") {
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
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "&="
                    | "|="
                    | "^="
                    | "**"
                    | "//"
                    | ">>"
                    | "<<"
                    | "->"
                    | ":="
                    | ".."
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

        // Decorator — treat @ as a token
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
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_string_prefix(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "f" | "b" | "r" | "u" | "rb" | "br" | "rf" | "fr" | "fb" | "bf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_python() {
        let source = "def foo(x):\n    return x + 1\n";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "def", "$ID", "(", "$ID", ")", ":", "return", "$ID", "+",
                "$LIT"
            ]
        );
    }

    #[test]
    fn test_comments_stripped() {
        let source = "# comment\nx = 5  # inline\ny = 10";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["$ID", "=", "$LIT", "$ID", "=", "$LIT"]);
    }

    #[test]
    fn test_string_literals() {
        let source = "a = \"hello\"\nb = 'world'";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["$ID", "=", "$LIT", "$ID", "=", "$LIT"]);
    }

    #[test]
    fn test_fstring() {
        let source = "s = f\"hello {name}\"";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["$ID", "=", "$LIT"]);
    }

    #[test]
    fn test_triple_quoted() {
        let source = "doc = \"\"\"multi\nline\nstring\"\"\"";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["$ID", "=", "$LIT"]);
    }

    #[test]
    fn test_keywords() {
        let source = "if x and y:\n    pass";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["if", "$ID", "and", "$ID", ":", "pass"]);
    }

    #[test]
    fn test_ignore_markers() {
        let source =
            "a = 1\n# mdlr:ignore-start\nb = 2\n# mdlr:ignore-end\nc = 3";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["$ID", "=", "$LIT", "$ID", "=", "$LIT"]);
    }

    #[test]
    fn test_decorator() {
        let source = "@decorator\ndef foo(): pass";
        let ft = tokenize_py(source, "test.py", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec!["@", "$ID", "def", "$ID", "(", ")", ":", "pass"]
        );
    }

    // ---- End-to-end CPD tests with real Python source files on disk ----
    //
    // Each test writes real Python source to disk, reads it back,
    // tokenizes with the real tokenizer, and runs clone detection.
    // A human reader can see the code and verify whether it should match.

    /// Helper: write a Python file to disk, read it back, tokenize it.
    fn tokenize_file(
        dir: &std::path::Path,
        name: &str,
        source: &str,
    ) -> FileTokens {
        let path = dir.join(name);
        std::fs::write(&path, source).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let ft = tokenize_py(&text, name, 1);

        // Also round-trip through binary on disk
        let token_path = dir.join(format!("{name}.tokens"));
        let bytes = mdlr_cpd::binary::serialize(&ft);
        std::fs::write(&token_path, &bytes).unwrap();
        let loaded = std::fs::read(&token_path).unwrap();
        mdlr_cpd::binary::deserialize(&loaded).unwrap()
    }

    /// Two functions that do the same thing with different variable names.
    /// After normalization (identifiers → $ID, literals → $LIT) they
    /// produce identical token streams and should be detected as clones.
    #[test]
    fn copy_pasted_function_different_names() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "orders.py",
            r#"
def process_orders(orders):
    results = []
    for order in orders:
        if order.total > 100:
            results.append({
                "id": order.id,
                "discount": order.total * 0.1,
                "status": "eligible",
            })
        else:
            results.append({
                "id": order.id,
                "discount": 0,
                "status": "ineligible",
            })
    return results
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "payments.py",
            r#"
def handle_payments(payments):
    output = []
    for payment in payments:
        if payment.total > 100:
            output.append({
                "id": payment.id,
                "discount": payment.total * 0.1,
                "status": "eligible",
            })
        else:
            output.append({
                "id": payment.id,
                "discount": 0,
                "status": "ineligible",
            })
    return output
"#,
        );

        let clones = mdlr_cpd::find_clones(&[a.clone(), b.clone()], 25);
        assert!(
            !clones.is_empty(),
            "should detect copy-pasted function with renamed variables"
        );

        let metrics = mdlr_cpd::compute_duplication(&clones, &[a, b], None);
        assert!(metrics.max > 50.0, "both files should show high duplication");
    }

    /// Two completely different Python files — an API client and a math
    /// module. Should produce zero clones.
    #[test]
    fn unrelated_code_no_false_positive() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "api_client.py",
            r#"
import requests

class ApiClient:
    def __init__(self, base_url, api_key):
        self.base_url = base_url
        self.headers = {"Authorization": f"Bearer {api_key}"}
        self.session = requests.Session()

    def get(self, path):
        response = self.session.get(self.base_url + path, headers=self.headers)
        response.raise_for_status()
        return response.json()

    def post(self, path, data):
        response = self.session.post(
            self.base_url + path, json=data, headers=self.headers
        )
        response.raise_for_status()
        return response.json()
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "math_utils.py",
            r#"
def fibonacci(n):
    if n <= 1:
        return n
    a, b = 0, 1
    for _ in range(2, n + 1):
        a, b = b, a + b
    return b

def is_prime(n):
    if n < 2:
        return False
    for i in range(2, int(n ** 0.5) + 1):
        if n % i == 0:
            return False
    return True
"#,
        );

        let clones = mdlr_cpd::find_clones(&[a, b], 25);
        assert!(
            clones.is_empty(),
            "unrelated Python code should produce no clones, got {}",
            clones.len()
        );
    }

    /// Same file has two copy-pasted handlers with different names.
    #[test]
    fn self_clone_within_single_file() {
        let tmp = tempfile::tempdir().unwrap();

        let ft = tokenize_file(
            tmp.path(),
            "handlers.py",
            r#"
def get_admin_dashboard(admin_id):
    user = db.find_by_id(admin_id)
    if not user:
        raise ValueError("not found")
    stats = compute_stats(user.activity)
    notifications = fetch_notifications(user.id)
    return {
        "user": user,
        "stats": stats,
        "notifications": notifications,
        "last_login": user.last_login,
    }

def something_unrelated():
    print("separator between the two clones")

def get_user_dashboard(user_id):
    user = db.find_by_id(user_id)
    if not user:
        raise ValueError("not found")
    stats = compute_stats(user.activity)
    notifications = fetch_notifications(user.id)
    return {
        "user": user,
        "stats": stats,
        "notifications": notifications,
        "last_login": user.last_login,
    }
"#,
        );

        let clones = mdlr_cpd::find_clones(&[ft], 20);
        assert!(
            !clones.is_empty(),
            "should detect self-clone within single Python file"
        );
        assert_eq!(clones[0].file_a, clones[0].file_b);
    }

    /// Three files share the same validation logic — should find clone
    /// pairs between all three (≥3 pairs: A-B, A-C, B-C).
    #[test]
    fn triplicate_validation_across_files() {
        let tmp = tempfile::tempdir().unwrap();

        let make_validator = |entity: &str| -> String {
            format!(
                r#"
def validate_{entity}(data):
    errors = []
    if not data.get("name"):
        errors.append("name is required")
    if not data.get("email") or "@" not in data["email"]:
        errors.append("valid email is required")
    if data.get("age") is not None and data["age"] < 0:
        errors.append("age must be non-negative")
    if errors:
        return {{"valid": False, "errors": errors}}
    return {{"valid": True, "errors": []}}
"#,
                entity = entity
            )
        };

        let a = tokenize_file(
            tmp.path(),
            "validate_user.py",
            &make_validator("user"),
        );
        let b = tokenize_file(
            tmp.path(),
            "validate_admin.py",
            &make_validator("admin"),
        );
        let c = tokenize_file(
            tmp.path(),
            "validate_guest.py",
            &make_validator("guest"),
        );

        let clones = mdlr_cpd::find_clones(&[a, b, c], 20);
        assert!(
            clones.len() >= 3,
            "three identical validators should produce ≥3 clone pairs, got {}",
            clones.len()
        );
    }

    /// Bubble sort vs binary search — structurally different algorithms.
    /// Should NOT match at a reasonable threshold.
    #[test]
    fn different_algorithms_no_match() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "sort.py",
            r#"
def bubble_sort(arr):
    n = len(arr)
    for i in range(n - 1):
        for j in range(n - i - 1):
            if arr[j] > arr[j + 1]:
                arr[j], arr[j + 1] = arr[j + 1], arr[j]
    return arr
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "search.py",
            r#"
def binary_search(arr, target):
    low = 0
    high = len(arr) - 1
    while low <= high:
        mid = (low + high) // 2
        if arr[mid] == target:
            return mid
        elif arr[mid] < target:
            low = mid + 1
        else:
            high = mid - 1
    return -1
"#,
        );

        let clones = mdlr_cpd::find_clones(&[a, b], 25);
        assert!(
            clones.is_empty(),
            "different algorithms should not match, got {} clone(s)",
            clones.len()
        );
    }

    /// Full metrics pipeline: two duplicated files + one unique file.
    /// Verify duplication percentages and that the unique file is clean.
    #[test]
    fn metrics_pipeline_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "fetch_a.py",
            r#"
def fetch_and_process(url, options):
    response = requests.get(url, **options)
    if not response.ok:
        raise RuntimeError(f"request failed: {response.status_code}")
    data = response.json()
    filtered = [item for item in data["items"] if item["active"]]
    mapped = [
        {"id": item["id"], "name": item["name"], "score": item["value"] * 1.5}
        for item in filtered
    ]
    return mapped
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "fetch_b.py",
            r#"
def load_and_transform(endpoint, config):
    response = requests.get(endpoint, **config)
    if not response.ok:
        raise RuntimeError(f"request failed: {response.status_code}")
    data = response.json()
    filtered = [item for item in data["items"] if item["active"]]
    mapped = [
        {"id": item["id"], "name": item["name"], "score": item["value"] * 1.5}
        for item in filtered
    ]
    return mapped
"#,
        );

        let c = tokenize_file(
            tmp.path(),
            "config.py",
            r#"
CONFIG = {
    "port": 3000,
    "host": "localhost",
    "debug": True,
    "max_retries": 5,
}
"#,
        );

        let all = vec![a, b, c];
        let clones = mdlr_cpd::find_clones(&all, 20);
        let metrics = mdlr_cpd::compute_duplication(&clones, &all, None);

        assert!(metrics.clone_count > 0, "should detect clones");

        // Config file should have 0% duplication
        let config_file = metrics
            .files
            .iter()
            .find(|f| f.file.to_string_lossy().contains("config"));
        if let Some(cf) = config_file {
            assert_eq!(
                cf.duplicated_lines, 0,
                "config file should have 0 duplicated lines"
            );
        }
    }
}
