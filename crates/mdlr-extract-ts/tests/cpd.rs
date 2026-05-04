//! End-to-end CPD tests with real TypeScript/JavaScript source files.
//!
//! Each test writes real source code to disk, tokenizes it with the real
//! TS tokenizer, serializes to binary `.tokens` files, loads them back,
//! and runs the full clone-detection + metrics pipeline.
//!
//! The source code is written so a human reader can verify by eye whether
//! two blocks should or should not be detected as duplicates.

// The tokenizer is pub(crate), so these tests drive the public `extract`
// entry point of the library — the same path production uses.

use std::path::Path;

/// Write source files to a temp dir, run the extractor, load `.tokens` files,
/// and return the deserialized token streams.
fn tokenize_files(files: &[(&str, &str)]) -> Vec<mdlr_cpd::FileTokens> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    for (rel_path, source) in files {
        let p = root.join(rel_path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, source).unwrap();
    }

    let output = root.join(".mdlr-cache");
    std::fs::create_dir_all(&output).unwrap();

    mdlr_extract_ts::extract(root, &output, Some(1)).expect("run extractor");

    let mut tokens = Vec::new();
    load_tokens_recursive(&output, &mut tokens);

    // Keep temp dir alive until we're done reading
    std::mem::forget(tmp);
    tokens
}

fn load_tokens_recursive(dir: &Path, out: &mut Vec<mdlr_cpd::FileTokens>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            load_tokens_recursive(&path, out);
        } else if path.extension().is_some_and(|e| e == "tokens") {
            let data = std::fs::read(&path).unwrap();
            out.push(mdlr_cpd::binary::deserialize(&data).unwrap());
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two functions that do the same thing with different variable names.
/// After normalization (identifiers → $ID, literals → $LIT) they should
/// produce identical token streams and be detected as clones.
#[test]
fn copy_pasted_function_different_names() {
    let file_a = r#"
function processOrders(orders) {
    const results = [];
    for (const order of orders) {
        if (order.total > 100) {
            results.push({
                id: order.id,
                discount: order.total * 0.1,
                status: "eligible"
            });
        } else {
            results.push({
                id: order.id,
                discount: 0,
                status: "ineligible"
            });
        }
    }
    return results;
}
"#;

    let file_b = r#"
function handlePayments(payments) {
    const output = [];
    for (const payment of payments) {
        if (payment.total > 100) {
            output.push({
                id: payment.id,
                discount: payment.total * 0.1,
                status: "eligible"
            });
        } else {
            output.push({
                id: payment.id,
                discount: 0,
                status: "ineligible"
            });
        }
    }
    return output;
}
"#;

    let all = tokenize_files(&[
        ("src/orders.ts", file_a),
        ("src/payments.ts", file_b),
    ]);
    assert_eq!(all.len(), 2);

    let clones = mdlr_cpd::find_clones(&all, 30);
    assert!(
        !clones.is_empty(),
        "should detect copy-pasted function with renamed variables"
    );

    let metrics = mdlr_cpd::compute_duplication(&clones, &all, None);
    assert!(metrics.max > 50.0, "both files should show high duplication");
}

/// Two files with completely different logic — no structural similarity.
/// Should produce zero clones.
#[test]
fn unrelated_code_no_false_positive() {
    let api_handler = r#"
import express from "express";

export function createRouter() {
    const router = express.Router();

    router.get("/users", async (req, res) => {
        const users = await db.query("SELECT * FROM users");
        res.json({ data: users, count: users.length });
    });

    router.post("/users", async (req, res) => {
        const { name, email } = req.body;
        const user = await db.insert("users", { name, email });
        res.status(201).json(user);
    });

    return router;
}
"#;

    let math_utils = r#"
export function fibonacci(n: number): number {
    if (n <= 1) return n;
    let a = 0, b = 1;
    for (let i = 2; i <= n; i++) {
        const temp = a + b;
        a = b;
        b = temp;
    }
    return b;
}

export function isPrime(n: number): boolean {
    if (n < 2) return false;
    for (let i = 2; i * i <= n; i++) {
        if (n % i === 0) return false;
    }
    return true;
}
"#;

    let all = tokenize_files(&[
        ("src/api.ts", api_handler),
        ("src/math.ts", math_utils),
    ]);

    let clones = mdlr_cpd::find_clones(&all, 30);
    assert!(
        clones.is_empty(),
        "unrelated code should produce no clones, got {} clone(s)",
        clones.len()
    );
}

/// Same file contains two copy-pasted blocks (self-clone).
#[test]
fn self_clone_within_single_file() {
    let source = r#"
// Handler for admin users — copy-pasted from regular users below
function getAdminDashboard(adminId) {
    const user = db.findById(adminId);
    if (!user) {
        throw new Error("not found");
    }
    const stats = computeStats(user.activity);
    const notifications = fetchNotifications(user.id);
    return {
        user: user,
        stats: stats,
        notifications: notifications,
        lastLogin: user.lastLogin
    };
}

function somethingUnrelated() {
    console.log("separator");
}

// Handler for regular users — the original
function getUserDashboard(userId) {
    const user = db.findById(userId);
    if (!user) {
        throw new Error("not found");
    }
    const stats = computeStats(user.activity);
    const notifications = fetchNotifications(user.id);
    return {
        user: user,
        stats: stats,
        notifications: notifications,
        lastLogin: user.lastLogin
    };
}
"#;

    let all = tokenize_files(&[("src/dashboard.ts", source)]);
    assert_eq!(all.len(), 1);

    let clones = mdlr_cpd::find_clones(&all, 20);
    assert!(!clones.is_empty(), "should detect self-clone within single file");
    assert_eq!(clones[0].file_a, clones[0].file_b);
}

/// Three files share the same validation logic. CPD should find clone pairs
/// between all three (≥ 3 pairs: A-B, A-C, B-C).
#[test]
fn triplicate_across_three_files() {
    let make_validator = |entity: &str| {
        format!(
            r#"
function validate{entity}(input) {{
    const errors = [];
    if (!input.name || input.name.length === 0) {{
        errors.push("name is required");
    }}
    if (!input.email || !input.email.includes("@")) {{
        errors.push("valid email is required");
    }}
    if (input.age !== undefined && input.age < 0) {{
        errors.push("age must be non-negative");
    }}
    if (errors.length > 0) {{
        return {{ valid: false, errors: errors }};
    }}
    return {{ valid: true, errors: [] }};
}}
"#,
            entity = entity
        )
    };

    let all = tokenize_files(&[
        ("src/validateUser.ts", &make_validator("User")),
        ("src/validateAdmin.ts", &make_validator("Admin")),
        ("src/validateGuest.ts", &make_validator("Guest")),
    ]);
    assert_eq!(all.len(), 3);

    let clones = mdlr_cpd::find_clones(&all, 20);
    assert!(
        clones.len() >= 3,
        "three identical files should produce ≥3 clone pairs, got {}",
        clones.len()
    );
}

/// Code that is structurally similar at a small scale (e.g., both use
/// if/else and push) but semantically different. Should NOT match at
/// a reasonable min_tokens threshold.
#[test]
fn structurally_similar_but_different_logic() {
    let sorting = r#"
function bubbleSort(arr) {
    const n = arr.length;
    for (let i = 0; i < n - 1; i++) {
        for (let j = 0; j < n - i - 1; j++) {
            if (arr[j] > arr[j + 1]) {
                const temp = arr[j];
                arr[j] = arr[j + 1];
                arr[j + 1] = temp;
            }
        }
    }
    return arr;
}
"#;

    let searching = r#"
function binarySearch(arr, target) {
    let low = 0;
    let high = arr.length - 1;
    while (low <= high) {
        const mid = Math.floor((low + high) / 2);
        if (arr[mid] === target) {
            return mid;
        } else if (arr[mid] < target) {
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    return -1;
}
"#;

    let all = tokenize_files(&[
        ("src/sort.ts", sorting),
        ("src/search.ts", searching),
    ]);

    let clones = mdlr_cpd::find_clones(&all, 30);
    assert!(
        clones.is_empty(),
        "different algorithms should not match, got {} clone(s)",
        clones.len()
    );
}

/// Verify the full metrics pipeline: duplication percentage, clone count,
/// and that the unique file reports 0%.
#[test]
fn metrics_pipeline_end_to_end() {
    let duplicated_a = r#"
export function fetchAndProcess(url, options) {
    const response = fetch(url, options);
    if (!response.ok) {
        throw new Error("request failed: " + response.status);
    }
    const data = response.json();
    const filtered = data.items.filter(item => item.active);
    const mapped = filtered.map(item => ({
        id: item.id,
        name: item.name,
        score: item.value * 1.5
    }));
    return mapped;
}
"#;

    let duplicated_b = r#"
export function loadAndTransform(endpoint, config) {
    const response = fetch(endpoint, config);
    if (!response.ok) {
        throw new Error("request failed: " + response.status);
    }
    const data = response.json();
    const filtered = data.items.filter(item => item.active);
    const mapped = filtered.map(item => ({
        id: item.id,
        name: item.name,
        score: item.value * 1.5
    }));
    return mapped;
}
"#;

    let unique = r#"
export const CONFIG = {
    port: 3000,
    host: "localhost",
    debug: true,
    maxRetries: 5
};
"#;

    let all = tokenize_files(&[
        ("src/fetchA.ts", duplicated_a),
        ("src/fetchB.ts", duplicated_b),
        ("src/config.ts", unique),
    ]);

    let clones = mdlr_cpd::find_clones(&all, 20);
    let metrics = mdlr_cpd::compute_duplication(&clones, &all, None);

    assert!(metrics.clone_count > 0, "should detect clones");

    // The duplicated files should show significant duplication
    let dup_files: Vec<_> = metrics
        .files
        .iter()
        .filter(|f| {
            let name = f.file.to_string_lossy();
            name.contains("fetchA") || name.contains("fetchB")
        })
        .collect();
    for f in &dup_files {
        assert!(
            f.percentage > 30.0,
            "{} should show >30% duplication, got {:.1}%",
            f.file.display(),
            f.percentage
        );
    }

    // Config file should show 0% duplication
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
