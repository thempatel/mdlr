use mdlr_cpd::{FileTokens, Token, NORMALIZED_ID, NORMALIZED_LIT};
use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxKind, SyntaxToken, T};
use std::path::PathBuf;

/// Tokenize a Rust source file for CPD analysis.
///
/// - Strips comments and whitespace
/// - Normalizes identifiers to $ID and literals to $LIT
/// - Respects `mdlr:ignore-start` / `mdlr:ignore-end` markers in comments
pub fn tokenize_rust(
    source: &str,
    source_path: &str,
    generation_id: u64,
) -> FileTokens {
    let parse = SourceFile::parse(source, Edition::CURRENT);
    let tree = parse.tree();

    let mut result = Vec::new();
    let mut ignoring = false;

    let line_starts = compute_line_starts(source);

    // Walk all tokens in the syntax tree (flat token stream)
    for token in tree.syntax().descendants_with_tokens() {
        let token = match token.into_token() {
            Some(t) => t,
            None => continue,
        };

        let kind = token.kind();

        // Check for ignore markers in comments before filtering them
        if kind == SyntaxKind::COMMENT {
            let text = token.text();
            if text.contains("mdlr:ignore-start") {
                ignoring = true;
            } else if text.contains("mdlr:ignore-end") {
                ignoring = false;
            }
            continue;
        }

        // Skip whitespace
        if kind == SyntaxKind::WHITESPACE {
            continue;
        }

        // Skip tokens in ignored regions
        if ignoring {
            continue;
        }

        let offset = u32::from(token.text_range().start());
        let (line, col) = offset_to_line_col(&line_starts, offset);

        let normalized = normalize_token(&token, kind);
        result.push(Token { value: normalized, line, col });
    }

    FileTokens {
        source_path: PathBuf::from(source_path),
        tokens: result,
        cached_at: generation_id,
    }
}

fn normalize_token(token: &SyntaxToken, kind: SyntaxKind) -> String {
    if is_identifier(kind) {
        NORMALIZED_ID.to_string()
    } else if is_literal(kind) {
        NORMALIZED_LIT.to_string()
    } else {
        token.text().to_string()
    }
}

fn compute_line_starts(source: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push((i + 1) as u32);
        }
    }
    starts
}

fn offset_to_line_col(line_starts: &[u32], offset: u32) -> (u32, u16) {
    match line_starts.binary_search(&offset) {
        Ok(line) => ((line + 1) as u32, 0),
        Err(line) => {
            let line_start = line_starts[line - 1];
            (line as u32, (offset - line_start) as u16)
        }
    }
}

fn is_identifier(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::IDENT | T![self] | T![super] | T![crate] | T![Self]
    )
}

fn is_literal(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::INT_NUMBER
            | SyntaxKind::FLOAT_NUMBER
            | SyntaxKind::STRING
            | SyntaxKind::BYTE_STRING
            | SyntaxKind::C_STRING
            | SyntaxKind::CHAR
            | SyntaxKind::BYTE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokenization() {
        let source = r#"fn main() {
    let x = 42;
}"#;
        let ft = tokenize_rust(source, "test.rs", 1);
        assert!(!ft.tokens.is_empty());

        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "fn", "$ID", "(", ")", "{", "let", "$ID", "=", "$LIT", ";",
                "}"
            ]
        );
    }

    #[test]
    fn test_comments_stripped() {
        let source = r#"// this is a comment
fn foo() {}"#;
        let ft = tokenize_rust(source, "test.rs", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["fn", "$ID", "(", ")", "{", "}"]);
    }

    #[test]
    fn test_ignore_markers() {
        let source = r#"fn before() {}
// mdlr:ignore-start
fn ignored() {}
// mdlr:ignore-end
fn after() {}"#;
        let ft = tokenize_rust(source, "test.rs", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(
            values,
            vec![
                "fn", "$ID", "(", ")", "{", "}", "fn", "$ID", "(", ")", "{",
                "}"
            ]
        );
    }

    #[test]
    fn test_string_literals_normalized() {
        let source = r#"let s = "hello world";"#;
        let ft = tokenize_rust(source, "test.rs", 1);
        let values: Vec<&str> =
            ft.tokens.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(values, vec!["let", "$ID", "=", "$LIT", ";"]);
    }

    #[test]
    fn test_line_numbers() {
        let source = "fn\nfoo\n(\n)";
        let ft = tokenize_rust(source, "test.rs", 1);
        let lines: Vec<u32> = ft.tokens.iter().map(|t| t.line).collect();
        assert_eq!(lines, vec![1, 2, 3, 4]);
    }

    // ---- End-to-end CPD tests with real Rust source files on disk ----

    /// Helper: write a Rust file to disk, read it back, tokenize it,
    /// round-trip through binary serialization.
    fn tokenize_file(
        dir: &std::path::Path,
        name: &str,
        source: &str,
    ) -> FileTokens {
        let path = dir.join(name);
        std::fs::write(&path, source).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let ft = tokenize_rust(&text, name, 1);

        // Round-trip through binary on disk
        let token_path = dir.join(format!("{name}.tokens"));
        let bytes = mdlr_cpd::binary::serialize(&ft);
        std::fs::write(&token_path, &bytes).unwrap();
        let loaded = std::fs::read(&token_path).unwrap();
        mdlr_cpd::binary::deserialize(&loaded).unwrap()
    }

    /// Two Rust functions that do the same thing with different names.
    /// After normalization (identifiers → $ID, literals → $LIT) they
    /// should be detected as clones.
    #[test]
    fn copy_pasted_function_different_names() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "orders.rs",
            r#"
fn process_orders(orders: &[Order]) -> Vec<OrderResult> {
    let mut results = Vec::new();
    for order in orders {
        if order.total > 100 {
            results.push(OrderResult {
                id: order.id,
                discount: order.total * 0.1,
                status: "eligible",
            });
        } else {
            results.push(OrderResult {
                id: order.id,
                discount: 0.0,
                status: "ineligible",
            });
        }
    }
    results
}
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "payments.rs",
            r#"
fn handle_payments(payments: &[Payment]) -> Vec<PaymentResult> {
    let mut output = Vec::new();
    for payment in payments {
        if payment.total > 100 {
            output.push(PaymentResult {
                id: payment.id,
                discount: payment.total * 0.1,
                status: "eligible",
            });
        } else {
            output.push(PaymentResult {
                id: payment.id,
                discount: 0.0,
                status: "ineligible",
            });
        }
    }
    output
}
"#,
        );

        let clones = mdlr_cpd::find_clones(&[a.clone(), b.clone()], 25);
        assert!(
            !clones.is_empty(),
            "should detect copy-pasted Rust function with renamed variables"
        );

        let metrics = mdlr_cpd::compute_duplication(&clones, &[a, b], None);
        assert!(metrics.max > 50.0, "both files should show high duplication");
    }

    /// Completely different Rust code — a web handler vs a sorting algo.
    /// Should produce zero clones.
    #[test]
    fn unrelated_code_no_false_positive() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "handler.rs",
            r#"
use axum::{Json, extract::Path};

async fn get_user(Path(id): Path<u64>) -> Json<User> {
    let user = db::find_user(id).await.expect("user not found");
    let profile = user.load_profile().await;
    Json(User {
        id: user.id,
        name: user.name.clone(),
        email: profile.email,
        created_at: user.created_at,
    })
}

async fn list_users() -> Json<Vec<UserSummary>> {
    let users = db::list_all().await;
    let summaries = users
        .into_iter()
        .map(|u| UserSummary { id: u.id, name: u.name })
        .collect();
    Json(summaries)
}
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "sort.rs",
            r#"
fn merge_sort(arr: &mut [i32]) {
    let len = arr.len();
    if len <= 1 {
        return;
    }
    let mid = len / 2;
    merge_sort(&mut arr[..mid]);
    merge_sort(&mut arr[mid..]);
    let mut merged = Vec::with_capacity(len);
    let (mut i, mut j) = (0, mid);
    while i < mid && j < len {
        if arr[i] <= arr[j] {
            merged.push(arr[i]);
            i += 1;
        } else {
            merged.push(arr[j]);
            j += 1;
        }
    }
    merged.extend_from_slice(&arr[i..mid]);
    merged.extend_from_slice(&arr[j..len]);
    arr.copy_from_slice(&merged);
}
"#,
        );

        let clones = mdlr_cpd::find_clones(&[a, b], 25);
        assert!(
            clones.is_empty(),
            "unrelated Rust code should produce no clones, got {}",
            clones.len()
        );
    }

    /// Same file has two copy-pasted handler functions.
    #[test]
    fn self_clone_within_single_file() {
        let tmp = tempfile::tempdir().unwrap();

        let ft = tokenize_file(
            tmp.path(),
            "handlers.rs",
            r#"
fn handle_admin_request(admin_id: u64) -> Response {
    let user = db::find_by_id(admin_id).expect("not found");
    let stats = compute_stats(&user.activity);
    let notifications = fetch_notifications(user.id);
    Response {
        user,
        stats,
        notifications,
        last_login: user.last_login,
    }
}

fn unrelated_helper() {
    println!("this separates the two clones");
}

fn handle_user_request(user_id: u64) -> Response {
    let user = db::find_by_id(user_id).expect("not found");
    let stats = compute_stats(&user.activity);
    let notifications = fetch_notifications(user.id);
    Response {
        user,
        stats,
        notifications,
        last_login: user.last_login,
    }
}
"#,
        );

        let clones = mdlr_cpd::find_clones(&[ft], 20);
        assert!(
            !clones.is_empty(),
            "should detect self-clone within single Rust file"
        );
        assert_eq!(clones[0].file_a, clones[0].file_b);
    }

    /// Three files share the same validation logic.
    #[test]
    fn triplicate_validation_across_files() {
        let tmp = tempfile::tempdir().unwrap();

        let make_validator = |name: &str| -> String {
            format!(
                r#"
fn validate_{name}(data: &InputData) -> ValidationResult {{
    let mut errors = Vec::new();
    if data.name.is_empty() {{
        errors.push("name is required".to_string());
    }}
    if !data.email.contains('@') {{
        errors.push("valid email is required".to_string());
    }}
    if let Some(age) = data.age {{
        if age < 0 {{
            errors.push("age must be non-negative".to_string());
        }}
    }}
    if errors.is_empty() {{
        ValidationResult {{ valid: true, errors: vec![] }}
    }} else {{
        ValidationResult {{ valid: false, errors }}
    }}
}}
"#,
                name = name
            )
        };

        let a = tokenize_file(
            tmp.path(),
            "validate_user.rs",
            &make_validator("user"),
        );
        let b = tokenize_file(
            tmp.path(),
            "validate_admin.rs",
            &make_validator("admin"),
        );
        let c = tokenize_file(
            tmp.path(),
            "validate_guest.rs",
            &make_validator("guest"),
        );

        let clones = mdlr_cpd::find_clones(&[a, b, c], 20);
        assert!(
            clones.len() >= 3,
            "three identical validators should produce ≥3 clone pairs, got {}",
            clones.len()
        );
    }

    /// Full metrics pipeline: two duplicated files + one unique file.
    #[test]
    fn metrics_pipeline_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();

        let a = tokenize_file(
            tmp.path(),
            "fetch_a.rs",
            r#"
fn fetch_and_process(url: &str, client: &Client) -> Vec<Item> {
    let response = client.get(url).send().expect("request failed");
    let data: ApiResponse = response.json().expect("invalid json");
    let filtered: Vec<_> = data.items.into_iter().filter(|i| i.active).collect();
    filtered.into_iter().map(|item| Item {
        id: item.id,
        name: item.name,
        score: item.value * 1.5,
    }).collect()
}
"#,
        );

        let b = tokenize_file(
            tmp.path(),
            "fetch_b.rs",
            r#"
fn load_and_transform(endpoint: &str, client: &Client) -> Vec<Item> {
    let response = client.get(endpoint).send().expect("request failed");
    let data: ApiResponse = response.json().expect("invalid json");
    let filtered: Vec<_> = data.items.into_iter().filter(|i| i.active).collect();
    filtered.into_iter().map(|item| Item {
        id: item.id,
        name: item.name,
        score: item.value * 1.5,
    }).collect()
}
"#,
        );

        let c = tokenize_file(
            tmp.path(),
            "config.rs",
            r#"
pub const PORT: u16 = 3000;
pub const HOST: &str = "localhost";
pub const DEBUG: bool = true;
pub const MAX_RETRIES: u32 = 5;
"#,
        );

        let all = vec![a, b, c];
        let clones = mdlr_cpd::find_clones(&all, 20);
        let metrics = mdlr_cpd::compute_duplication(&clones, &all, None);

        assert!(metrics.clone_count > 0, "should detect clones");

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
