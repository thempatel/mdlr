use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct FileCacheEntry {
    units: Vec<Unit>,
}

#[derive(Debug, Deserialize)]
struct Unit {
    id: String,
    kind: String,
    reads: Vec<String>,
    writes: Vec<String>,
    calls: Vec<String>,
    params: usize,
    branches: usize,
    max_scope_lines: usize,
    #[serde(default)]
    parent: Option<String>,
}

/// Run the extractor on a temp project and return units keyed by id.
fn extract(lib_rs: &str) -> HashMap<String, Unit> {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "test_crate"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("write Cargo.toml");

    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(root.join("src/lib.rs"), lib_rs).expect("write lib.rs");

    let output_dir = root.join("output");
    std::fs::create_dir_all(&output_dir).expect("mkdir output");

    mdlr_extract_rust::extract(
        &root.join("Cargo.toml"),
        &output_dir,
        None,
        &[],
        root,
    )
    .expect("run extractor");

    let json_files: Vec<PathBuf> = find_json_files(&output_dir);
    assert!(
        !json_files.is_empty(),
        "no JSON output files in {}",
        output_dir.display()
    );

    let mut units = HashMap::new();
    for json_file in &json_files {
        let content = std::fs::read_to_string(json_file)
            .unwrap_or_else(|e| panic!("read {}: {e}", json_file.display()));
        let entry: FileCacheEntry = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("parse {}: {e}", json_file.display()));
        for unit in entry.units {
            units.insert(unit.id.clone(), unit);
        }
    }

    units
}

fn find_files_by_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(find_files_by_ext(&path, ext));
            } else if path.extension().is_some_and(|e| e == ext) {
                results.push(path);
            }
        }
    }
    results
}

fn find_json_files(dir: &Path) -> Vec<PathBuf> {
    find_files_by_ext(dir, "json")
}

fn find_token_files(dir: &Path) -> Vec<PathBuf> {
    find_files_by_ext(dir, "tokens")
}

// ---- Branch counting tests ----

#[test]
fn branches_if_else() {
    let units = extract(
        r#"
pub fn branchy(x: i32) -> i32 {
    if x > 0 {
        1
    } else if x < 0 {
        -1
    } else {
        0
    }
}
"#,
    );

    let f = &units["test_crate::branchy"];
    // if (+1) + else-if (+1) = 2 branches
    assert_eq!(f.branches, 2, "if/else-if should count 2 branches");
}

#[test]
fn branches_match() {
    let units = extract(
        r#"
pub fn matchy(x: i32) -> &'static str {
    match x {
        1 => "one",
        2 => "two",
        3 => "three",
        _ => "other",
    }
}
"#,
    );

    let f = &units["test_crate::matchy"];
    // 4 arms - 1 = 3 branches
    assert_eq!(f.branches, 3, "match with 4 arms should count 3 branches");
}

#[test]
fn branches_loop() {
    let units = extract(
        r#"
pub fn loopy(n: i32) -> i32 {
    let mut sum = 0;
    for i in 0..n {
        sum += i;
    }
    sum
}
"#,
    );

    let f = &units["test_crate::loopy"];
    // for loop desugars to Loop in HIR → +1
    // The for-loop desugar also includes an If and a Match, but those
    // are desugared (not MatchSource::Normal), so only the loop counts.
    assert!(
        f.branches >= 1,
        "for loop should count at least 1 branch, got {}",
        f.branches
    );
}

#[test]
fn branches_short_circuit() {
    let units = extract(
        r#"
pub fn logic(a: bool, b: bool, c: bool) -> bool {
    a && b || c
}
"#,
    );

    let f = &units["test_crate::logic"];
    // && (+1) + || (+1) = 2
    assert_eq!(f.branches, 2, "&& and || should each count as a branch");
}

#[test]
fn branches_zero_for_simple_fn() {
    let units = extract(
        r#"
pub fn simple(x: i32) -> i32 {
    x + 1
}
"#,
    );

    let f = &units["test_crate::simple"];
    assert_eq!(f.branches, 0, "simple function should have 0 branches");
}

// ---- Scope measurement tests ----

#[test]
fn scope_if_body() {
    let units = extract(
        r#"
pub fn scoped(x: i32) -> i32 {
    if x > 0 {
        let a = 1;
        let b = 2;
        let c = 3;
        a + b + c
    } else {
        0
    }
}
"#,
    );

    let f = &units["test_crate::scoped"];
    // The if-body spans 5 lines (from `{` to `}`), the else is 1 line
    assert!(
        f.max_scope_lines >= 4,
        "max_scope should be at least 4 for the if body, got {}",
        f.max_scope_lines
    );
}

// ---- Call extraction tests ----

#[test]
fn calls_function_call() {
    let units = extract(
        r#"
fn helper() -> i32 { 42 }

pub fn caller() -> i32 {
    helper()
}
"#,
    );

    let f = &units["test_crate::caller"];
    assert!(
        f.calls.iter().any(|c| c.contains("helper")),
        "caller should call helper, got {:?}",
        f.calls
    );
}

#[test]
fn calls_method_call() {
    let units = extract(
        r#"
pub fn stringy() -> String {
    let s = String::from("hello");
    s.to_uppercase()
}
"#,
    );

    let f = &units["test_crate::stringy"];
    assert!(
        f.calls.iter().any(|c| c.contains("to_uppercase")),
        "should detect to_uppercase call, got {:?}",
        f.calls
    );
}

#[test]
fn calls_no_calls() {
    let units = extract(
        r#"
pub fn pure(x: i32) -> i32 {
    x * 2
}
"#,
    );

    let f = &units["test_crate::pure"];
    assert!(
        f.calls.is_empty(),
        "pure function should have no calls, got {:?}",
        f.calls
    );
}

// ---- Field access tests ----

#[test]
fn field_read() {
    let units = extract(
        r#"
pub struct Point { pub x: f64, pub y: f64 }

impl Point {
    pub fn sum(&self) -> f64 {
        self.x + self.y
    }
}
"#,
    );

    let f = &units["test_crate::Point::sum"];
    assert!(
        f.reads.contains(&"x".to_string()),
        "should read x, got {:?}",
        f.reads
    );
    assert!(
        f.reads.contains(&"y".to_string()),
        "should read y, got {:?}",
        f.reads
    );
    assert!(f.writes.is_empty(), "should have no writes, got {:?}", f.writes);
}

#[test]
fn field_write() {
    let units = extract(
        r#"
pub struct Counter { pub count: i32 }

impl Counter {
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
"#,
    );

    let f = &units["test_crate::Counter::increment"];
    assert!(
        f.writes.contains(&"count".to_string()),
        "should write count, got {:?}",
        f.writes
    );
}

#[test]
fn field_read_and_write() {
    let units = extract(
        r#"
pub struct Acc { pub total: i32, pub count: i32 }

impl Acc {
    pub fn add(&mut self, value: i32) {
        self.total += value;
        self.count += 1;
    }

    pub fn average(&self) -> f64 {
        self.total as f64 / self.count as f64
    }
}
"#,
    );

    let add = &units["test_crate::Acc::add"];
    assert!(
        add.writes.contains(&"total".to_string()),
        "add should write total"
    );
    assert!(
        add.writes.contains(&"count".to_string()),
        "add should write count"
    );

    let avg = &units["test_crate::Acc::average"];
    assert!(
        avg.reads.contains(&"total".to_string()),
        "average should read total"
    );
    assert!(
        avg.reads.contains(&"count".to_string()),
        "average should read count"
    );
    assert!(avg.writes.is_empty(), "average should have no writes");
}

// ---- Parameter counting tests ----

#[test]
fn params_function() {
    let units = extract(
        r#"
pub fn three_params(a: i32, b: i32, c: i32) -> i32 {
    a + b + c
}
"#,
    );

    let f = &units["test_crate::three_params"];
    assert_eq!(f.params, 3, "should count 3 params");
}

#[test]
fn params_method_excludes_self() {
    let units = extract(
        r#"
pub struct Foo;

impl Foo {
    pub fn bar(&self, x: i32) -> i32 { x }
}
"#,
    );

    let f = &units["test_crate::Foo::bar"];
    assert_eq!(
        f.params, 1,
        "method params should exclude self, got {}",
        f.params
    );
}

// ---- Struct extraction tests ----

#[test]
fn struct_extracted() {
    let units = extract(
        r#"
pub struct Widget { pub name: String }
"#,
    );

    let s = &units["test_crate::Widget"];
    assert_eq!(s.kind, "Struct");
}

// ---- Parent tracking tests ----

#[test]
fn method_has_parent() {
    let units = extract(
        r#"
pub struct Dog { pub name: String }

impl Dog {
    pub fn bark(&self) -> &str { "woof" }
}
"#,
    );

    let method = &units["test_crate::Dog::bark"];
    assert_eq!(method.kind, "Method");
    assert!(
        method.parent.as_ref().is_some_and(|p| p.contains("Dog")),
        "method should have Dog as parent, got {:?}",
        method.parent
    );
}

// ---- Combined / complex tests ----

#[test]
fn complex_function() {
    let units = extract(
        r#"
pub fn complex(items: &[i32]) -> i32 {
    let mut result = 0;
    for item in items {
        if *item > 0 {
            result += item;
        } else if *item < -10 {
            result -= item;
        }
    }
    match result {
        0 => -1,
        x if x > 100 => 100,
        x => x,
    }
}
"#,
    );

    let f = &units["test_crate::complex"];
    // for-loop (+1 from Loop), if (+1), else-if (+1), match (3 arms - 1 = 2) = 5+
    assert!(
        f.branches >= 4,
        "complex function should have at least 4 branches, got {}",
        f.branches
    );
}

// ---- Parameter field access tests ----

#[test]
fn param_field_read() {
    let units = extract(
        r#"
pub struct Point { pub x: f64, pub y: f64 }

pub fn distance(p: &Point) -> f64 {
    p.x * p.x + p.y * p.y
}
"#,
    );

    let f = &units["test_crate::distance"];
    assert!(
        f.reads.contains(&"p.x".to_string()),
        "should read p.x, got {:?}",
        f.reads
    );
    assert!(
        f.reads.contains(&"p.y".to_string()),
        "should read p.y, got {:?}",
        f.reads
    );
    assert!(f.writes.is_empty(), "should have no writes, got {:?}", f.writes);
}

#[test]
fn param_field_write() {
    let units = extract(
        r#"
pub struct Point { pub x: f64, pub y: f64 }

pub fn reset(p: &mut Point) {
    p.x = 0.0;
}
"#,
    );

    let f = &units["test_crate::reset"];
    assert!(
        f.writes.contains(&"p.x".to_string()),
        "should write p.x, got {:?}",
        f.writes
    );
}

#[test]
fn param_method_call_not_read() {
    let units = extract(
        r#"
pub fn run_it(s: String) {
    s.len();
}
"#,
    );

    let f = &units["test_crate::run_it"];
    // s.len() is a method call, not a field read
    assert!(
        !f.reads.iter().any(|r| r.starts_with("s.")),
        "param.method() should not be a field read, got {:?}",
        f.reads
    );
}

#[test]
fn method_with_param_field_access() {
    let units = extract(
        r#"
pub struct Point { pub x: f64, pub y: f64 }

impl Point {
    pub fn add(&mut self, other: &Point) {
        self.x += other.x;
        self.y += other.y;
    }
}
"#,
    );

    let f = &units["test_crate::Point::add"];
    // self fields (bare names)
    assert!(f.writes.contains(&"x".to_string()), "should write x");
    assert!(f.writes.contains(&"y".to_string()), "should write y");
    // param fields (prefixed)
    assert!(
        f.reads.contains(&"other.x".to_string()),
        "should read other.x, got {:?}",
        f.reads
    );
    assert!(
        f.reads.contains(&"other.y".to_string()),
        "should read other.y, got {:?}",
        f.reads
    );
}

#[test]
fn closure_branches_counted() {
    let units = extract(
        r#"
pub fn with_closure(items: &[i32]) -> Vec<i32> {
    items.iter().filter(|x| {
        if **x > 0 {
            true
        } else {
            false
        }
    }).copied().collect()
}
"#,
    );

    let f = &units["test_crate::with_closure"];
    // The if inside the closure counts as a branch
    assert!(
        f.branches >= 1,
        "branches inside closures should be counted, got {}",
        f.branches
    );
}

// ---- CPD token extraction tests (real files on disk) ----

/// Run the extractor on real Rust source on disk and return token files produced.
fn extract_with_tokens(lib_rs: &str) -> (PathBuf, Vec<PathBuf>) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path().to_path_buf();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "test_crate"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("write Cargo.toml");

    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(root.join("src/lib.rs"), lib_rs).expect("write lib.rs");

    let output_dir = root.join("output");
    std::fs::create_dir_all(&output_dir).expect("mkdir output");

    mdlr_extract_rust::extract(
        &root.join("Cargo.toml"),
        &output_dir,
        None,
        &[],
        &root,
    )
    .expect("run extractor");

    let token_files = find_token_files(&output_dir);
    // Leak the tempdir so files survive for the caller
    std::mem::forget(tmp);
    (output_dir, token_files)
}

#[test]
fn tokens_file_produced_alongside_json() {
    let (output_dir, token_files) = extract_with_tokens(
        r#"
pub fn hello() -> &'static str {
    "world"
}
"#,
    );

    let json_files = find_json_files(&output_dir);
    assert!(!json_files.is_empty(), "should produce JSON cache files");
    assert!(
        !token_files.is_empty(),
        "should produce .tokens files alongside JSON"
    );
}

#[test]
fn tokens_file_deserializes_correctly() {
    let (_output_dir, token_files) = extract_with_tokens(
        r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
    );

    assert!(!token_files.is_empty());
    let data = std::fs::read(&token_files[0]).expect("read token file");
    let file_tokens =
        mdlr_cpd::binary::deserialize(&data).expect("deserialize tokens");

    assert!(!file_tokens.tokens.is_empty(), "should have tokens");

    // Verify normalization: identifiers become $ID, literals become $LIT
    let values: Vec<&str> =
        file_tokens.tokens.iter().map(|t| t.value.as_str()).collect();
    assert!(
        values.contains(&"$ID"),
        "should contain normalized identifiers, got {:?}",
        values
    );
    assert!(
        values.contains(&"fn"),
        "should contain keyword 'fn', got {:?}",
        values
    );
}

#[test]
fn tokens_detect_duplicated_functions_on_disk() {
    // Write two functions with identical structure to a real Rust file
    let (_output_dir, token_files) = extract_with_tokens(
        r#"
pub fn process_alpha(items: &[i32]) -> Vec<i32> {
    let mut result = Vec::new();
    for item in items {
        if *item > 0 {
            result.push(*item * 2);
        } else {
            result.push(0);
        }
    }
    result
}

pub fn process_beta(entries: &[i32]) -> Vec<i32> {
    let mut result = Vec::new();
    for entry in entries {
        if *entry > 0 {
            result.push(*entry * 2);
        } else {
            result.push(0);
        }
    }
    result
}
"#,
    );

    assert!(!token_files.is_empty());
    let data = std::fs::read(&token_files[0]).expect("read token file");
    let file_tokens =
        mdlr_cpd::binary::deserialize(&data).expect("deserialize tokens");

    // Since both functions have the same structure after normalization,
    // CPD should find a self-clone within this single file
    let clones = mdlr_cpd::find_clones(&[file_tokens], 15);
    assert!(
        !clones.is_empty(),
        "should detect duplicated function bodies within the same file"
    );
    // The clone should be within the same file (self-clone)
    assert_eq!(
        clones[0].file_a, clones[0].file_b,
        "clone should be within same file"
    );
}
