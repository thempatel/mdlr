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

/// Run the extractor on a temp directory with a single TS file and return units keyed by id.
fn extract(source: &str) -> HashMap<String, Unit> {
    extract_file("src/test.ts", source)
}

fn extract_file(rel_path: &str, source: &str) -> HashMap<String, Unit> {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path();

    let file_path = root.join(rel_path);
    std::fs::create_dir_all(file_path.parent().unwrap()).expect("mkdir");
    std::fs::write(&file_path, source).expect("write source");

    let output_dir = root.join("output");
    std::fs::create_dir_all(&output_dir).expect("mkdir output");

    mdlr_extract_ts::extract(root, &output_dir, Some(1))
        .expect("run extractor");

    let json_files = find_json_files(&output_dir);
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

// ---- Unit extraction tests ----

#[test]
fn function_declaration() {
    let units = extract(
        r#"
function greet(name: string): string {
    return "hello " + name;
}
"#,
    );

    let f = &units["src/test.ts::greet"];
    assert_eq!(f.kind, "Function");
    assert_eq!(f.params, 1);
}

#[test]
fn arrow_function_const() {
    let units = extract(
        r#"
const add = (a: number, b: number) => a + b;
"#,
    );

    let f = &units["src/test.ts::add"];
    assert_eq!(f.kind, "Function");
    assert_eq!(f.params, 2);
}

#[test]
fn function_expression_const() {
    let units = extract(
        r#"
const multiply = function(a: number, b: number) {
    return a * b;
};
"#,
    );

    let f = &units["src/test.ts::multiply"];
    assert_eq!(f.kind, "Function");
    assert_eq!(f.params, 2);
}

#[test]
fn class_and_methods() {
    let units = extract(
        r#"
class Foo {
    bar(x: number): number {
        return x * 2;
    }
}
"#,
    );

    let s = &units["src/test.ts::Foo"];
    assert_eq!(s.kind, "Struct");

    let m = &units["src/test.ts::Foo::bar"];
    assert_eq!(m.kind, "Method");
    assert_eq!(m.params, 1);
    assert_eq!(m.parent.as_deref(), Some("src/test.ts::Foo"));
}

#[test]
fn constructor() {
    let units = extract(
        r#"
class Widget {
    name: string;
    constructor(name: string) {
        this.name = name;
    }
}
"#,
    );

    let c = &units["src/test.ts::Widget::constructor"];
    assert_eq!(c.kind, "Method");
    assert_eq!(c.params, 1);
    assert_eq!(c.parent.as_deref(), Some("src/test.ts::Widget"));
    assert!(c.writes.contains(&"name".to_string()));
}

#[test]
fn getter_setter() {
    let units = extract(
        r#"
class Box {
    private _value: number = 0;

    get value(): number {
        return this._value;
    }

    set value(v: number) {
        this._value = v;
    }
}
"#,
    );

    let getter = &units["src/test.ts::Box::get_value"];
    assert_eq!(getter.kind, "Method");
    assert_eq!(getter.params, 0);

    let setter = &units["src/test.ts::Box::set_value"];
    assert_eq!(setter.kind, "Method");
    assert_eq!(setter.params, 1);
}

#[test]
fn export_default_function() {
    let units = extract(
        r#"
export default function handler() {
    console.log("hello");
}
"#,
    );

    let f = &units["src/test.ts::handler"];
    assert_eq!(f.kind, "Function");
    assert!(f.calls.contains(&"console.log".to_string()));
}

#[test]
fn export_default_arrow() {
    let units = extract(
        r#"
export default () => {
    return 42;
};
"#,
    );

    let f = &units["src/test.ts::default"];
    assert_eq!(f.kind, "Function");
}

#[test]
fn nested_named_function() {
    let units = extract(
        r#"
function outer() {
    function inner() {
        return 42;
    }
    return inner();
}
"#,
    );

    assert!(units.contains_key("src/test.ts::outer"));
    assert!(units.contains_key("src/test.ts::outer::inner"));
}

// ---- Branch counting tests ----

#[test]
fn branches_if() {
    let units = extract(
        r#"
function branchy(x: number): number {
    if (x > 0) {
        return 1;
    } else if (x < 0) {
        return -1;
    } else {
        return 0;
    }
}
"#,
    );

    let f = &units["src/test.ts::branchy"];
    // if (+1) + else-if (+1) = 2
    assert_eq!(f.branches, 2, "if/else-if should count 2 branches");
}

#[test]
fn branches_switch() {
    let units = extract(
        r#"
function switchy(x: number): string {
    switch (x) {
        case 1: return "one";
        case 2: return "two";
        case 3: return "three";
        default: return "other";
    }
}
"#,
    );

    let f = &units["src/test.ts::switchy"];
    // 4 cases - 1 = 3
    assert_eq!(f.branches, 3, "switch with 4 cases should count 3 branches");
}

#[test]
fn branches_loops() {
    let units = extract(
        r#"
function loopy(n: number): number {
    let sum = 0;
    for (let i = 0; i < n; i++) {
        sum += i;
    }
    return sum;
}
"#,
    );

    let f = &units["src/test.ts::loopy"];
    assert_eq!(f.branches, 1, "for loop should count 1 branch");
}

#[test]
fn branches_logical_operators() {
    let units = extract(
        r#"
function logic(a: boolean, b: boolean, c: boolean): boolean {
    return a && b || c;
}
"#,
    );

    let f = &units["src/test.ts::logic"];
    // && (+1) + || (+1) = 2
    assert_eq!(f.branches, 2, "&& and || should each count as a branch");
}

#[test]
fn branches_ternary() {
    let units = extract(
        r#"
const decide = (x: number) => x > 0 ? "yes" : "no";
"#,
    );

    let f = &units["src/test.ts::decide"];
    assert_eq!(f.branches, 1, "ternary should count 1 branch");
}

#[test]
fn branches_zero_for_simple_fn() {
    let units = extract(
        r#"
function simple(x: number): number {
    return x + 1;
}
"#,
    );

    let f = &units["src/test.ts::simple"];
    assert_eq!(f.branches, 0, "simple function should have 0 branches");
}

// ---- Scope measurement tests ----

#[test]
fn scope_if_body() {
    let units = extract(
        r#"
function scoped(x: number): number {
    if (x > 0) {
        const a = 1;
        const b = 2;
        const c = 3;
        return a + b + c;
    } else {
        return 0;
    }
}
"#,
    );

    let f = &units["src/test.ts::scoped"];
    assert!(
        f.max_scope_lines >= 4,
        "max_scope should be at least 4, got {}",
        f.max_scope_lines
    );
}

// ---- Call extraction tests ----

#[test]
fn calls_function() {
    let units = extract(
        r#"
function helper(): number { return 42; }

function caller(): number {
    return helper();
}
"#,
    );

    let f = &units["src/test.ts::caller"];
    assert!(
        f.calls.contains(&"helper".to_string()),
        "should call helper, got {:?}",
        f.calls
    );
}

#[test]
fn calls_method() {
    let units = extract(
        r#"
function doStuff() {
    console.log("hello");
    const arr = [1, 2, 3];
    arr.push(4);
}
"#,
    );

    let f = &units["src/test.ts::doStuff"];
    assert!(
        f.calls.contains(&"console.log".to_string()),
        "should call console.log, got {:?}",
        f.calls
    );
    assert!(
        f.calls.contains(&"arr.push".to_string()),
        "should call arr.push, got {:?}",
        f.calls
    );
}

#[test]
fn calls_new_expr() {
    let units = extract(
        r#"
function create() {
    return new Map();
}
"#,
    );

    let f = &units["src/test.ts::create"];
    assert!(
        f.calls.contains(&"Map".to_string()),
        "should call Map (new), got {:?}",
        f.calls
    );
}

#[test]
fn calls_no_calls() {
    let units = extract(
        r#"
function pure(x: number): number {
    return x * 2;
}
"#,
    );

    let f = &units["src/test.ts::pure"];
    assert!(f.calls.is_empty(), "should have no calls, got {:?}", f.calls);
}

// ---- Field access tests ----

#[test]
fn field_read() {
    let units = extract(
        r#"
class Point {
    x: number;
    y: number;

    constructor(x: number, y: number) {
        this.x = x;
        this.y = y;
    }

    sum(): number {
        return this.x + this.y;
    }
}
"#,
    );

    let f = &units["src/test.ts::Point::sum"];
    assert!(f.reads.contains(&"x".to_string()), "should read x");
    assert!(f.reads.contains(&"y".to_string()), "should read y");
    assert!(f.writes.is_empty(), "should have no writes");
}

#[test]
fn field_write() {
    let units = extract(
        r#"
class Counter {
    count: number = 0;

    increment() {
        this.count += 1;
    }
}
"#,
    );

    let f = &units["src/test.ts::Counter::increment"];
    assert!(
        f.writes.contains(&"count".to_string()),
        "should write count, got {:?}",
        f.writes
    );
}

#[test]
fn field_this_method_not_read() {
    let units = extract(
        r#"
class Logger {
    count: number = 0;

    log() {
        this.count += 1;
        this.flush();
    }

    flush() {}
}
"#,
    );

    let f = &units["src/test.ts::Logger::log"];
    assert!(f.writes.contains(&"count".to_string()));
    assert!(f.calls.contains(&"this.flush".to_string()));
    // flush should NOT appear as a field read
    assert!(
        !f.reads.contains(&"flush".to_string()),
        "this.method() should not be a field read, got {:?}",
        f.reads
    );
}

// ---- Parameter counting ----

#[test]
fn params_function() {
    let units = extract(
        r#"
function threeParams(a: number, b: number, c: number): number {
    return a + b + c;
}
"#,
    );

    let f = &units["src/test.ts::threeParams"];
    assert_eq!(f.params, 3);
}

#[test]
fn params_no_params() {
    let units = extract(
        r#"
function noParams(): void {}
"#,
    );

    let f = &units["src/test.ts::noParams"];
    assert_eq!(f.params, 0);
}

// ---- JS file support ----

#[test]
fn javascript_file() {
    let units = extract_file(
        "src/app.js",
        r#"
function hello(name) {
    console.log("hello " + name);
}

class App {
    run() {
        hello("world");
    }
}
"#,
    );

    assert!(units.contains_key("src/app.js::hello"));
    assert!(units.contains_key("src/app.js::App"));
    assert!(units.contains_key("src/app.js::App::run"));
}

// ---- Parameter field access tests ----

#[test]
fn param_field_read() {
    let units = extract(
        r#"
interface Point { x: number; y: number; }

function distance(p: Point): number {
    return p.x * p.x + p.y * p.y;
}
"#,
    );

    let f = &units["src/test.ts::distance"];
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
interface Point { x: number; y: number; }

function reset(p: Point) {
    p.x = 0;
}
"#,
    );

    let f = &units["src/test.ts::reset"];
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
function run_it(s: string) {
    s.trim();
}
"#,
    );

    let f = &units["src/test.ts::run_it"];
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
class Point {
    x: number;
    y: number;

    constructor(x: number, y: number) {
        this.x = x;
        this.y = y;
    }

    add(other: Point) {
        this.x += other.x;
        this.y += other.y;
    }
}
"#,
    );

    let f = &units["src/test.ts::Point::add"];
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

// ---- CPD token extraction tests (real files on disk) ----

/// Run the extractor and return the output directory path and token files.
fn extract_with_tokens(source: &str) -> (PathBuf, Vec<PathBuf>) {
    extract_files_with_tokens(&[("src/test.ts", source)])
}

/// Run the extractor on multiple files and return output dir + token files.
fn extract_files_with_tokens(
    files: &[(&str, &str)],
) -> (PathBuf, Vec<PathBuf>) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let root = tmp.path().to_path_buf();

    for (rel_path, source) in files {
        let file_path = root.join(rel_path);
        std::fs::create_dir_all(file_path.parent().unwrap()).expect("mkdir");
        std::fs::write(&file_path, source).expect("write source");
    }

    let output_dir = root.join("output");
    std::fs::create_dir_all(&output_dir).expect("mkdir output");

    mdlr_extract_ts::extract(&root, &output_dir, Some(1))
        .expect("run extractor");

    let token_files = find_token_files(&output_dir);
    std::mem::forget(tmp);
    (output_dir, token_files)
}

#[test]
fn tokens_file_produced_for_ts() {
    let (output_dir, token_files) = extract_with_tokens(
        r#"
function greet(name: string): string {
    return "hello " + name;
}
"#,
    );

    let json_files = find_json_files(&output_dir);
    assert!(!json_files.is_empty(), "should produce JSON files");
    assert!(!token_files.is_empty(), "should produce .tokens files");
}

#[test]
fn tokens_file_deserializes_with_correct_normalization() {
    let (_output_dir, token_files) = extract_with_tokens(
        r#"
function add(a: number, b: number): number {
    return a + b;
}
"#,
    );

    assert!(!token_files.is_empty());
    let data = std::fs::read(&token_files[0]).expect("read token file");
    let file_tokens =
        mdlr_cpd::binary::deserialize(&data).expect("deserialize tokens");

    assert!(!file_tokens.tokens.is_empty());

    let values: Vec<&str> =
        file_tokens.tokens.iter().map(|t| t.value.as_str()).collect();
    assert!(
        values.contains(&"function"),
        "should contain keyword 'function', got {:?}",
        values
    );
    assert!(
        values.contains(&"$ID"),
        "should normalize identifiers to $ID, got {:?}",
        values
    );
    assert!(
        values.contains(&"return"),
        "should contain keyword 'return', got {:?}",
        values
    );
}

#[test]
fn tokens_detect_duplicate_ts_functions_across_files() {
    let (_output_dir, token_files) = extract_files_with_tokens(&[
        (
            "src/a.ts",
            r#"
function processItems(items: number[]): number[] {
    const result: number[] = [];
    for (const item of items) {
        if (item > 0) {
            result.push(item * 2);
        } else {
            result.push(0);
        }
    }
    return result;
}
"#,
        ),
        (
            "src/b.ts",
            r#"
function transformEntries(entries: number[]): number[] {
    const output: number[] = [];
    for (const entry of entries) {
        if (entry > 0) {
            output.push(entry * 2);
        } else {
            output.push(0);
        }
    }
    return output;
}
"#,
        ),
    ]);

    assert!(
        token_files.len() >= 2,
        "should produce token files for both TS files, got {}",
        token_files.len()
    );

    // Load all token files
    let mut all_tokens: Vec<mdlr_cpd::FileTokens> = Vec::new();
    for tf in &token_files {
        let data = std::fs::read(tf).expect("read token file");
        let ft = mdlr_cpd::binary::deserialize(&data).expect("deserialize");
        all_tokens.push(ft);
    }

    // These two functions have identical structure after normalization.
    // They should be detected as clones.
    let clones = mdlr_cpd::find_clones(&all_tokens, 15);
    assert!(
        !clones.is_empty(),
        "should detect duplicate functions across files"
    );

    // Verify metrics
    let metrics = mdlr_cpd::compute_duplication(&clones, &all_tokens, None);
    assert!(metrics.clone_count > 0);
    assert!(metrics.max > 0.0);
}

#[test]
fn tokens_comments_stripped_on_disk() {
    let (_output_dir, token_files) = extract_with_tokens(
        r#"
// This is a comment that should be stripped
/* Block comment also stripped */
const x = 42;
"#,
    );

    assert!(!token_files.is_empty());
    let data = std::fs::read(&token_files[0]).expect("read");
    let ft = mdlr_cpd::binary::deserialize(&data).expect("deserialize");

    // No comment tokens should be present
    for token in &ft.tokens {
        assert!(
            !token.value.starts_with("//"),
            "line comment should be stripped: {:?}",
            token.value
        );
        assert!(
            !token.value.starts_with("/*"),
            "block comment should be stripped: {:?}",
            token.value
        );
    }
}

#[test]
fn tokens_ignore_markers_on_disk() {
    let (_output_dir, token_files) = extract_with_tokens(
        r#"
const before = 1;
// mdlr:ignore-start
const ignored = 2;
const alsoIgnored = 3;
// mdlr:ignore-end
const after = 4;
"#,
    );

    assert!(!token_files.is_empty());
    let data = std::fs::read(&token_files[0]).expect("read");
    let ft = mdlr_cpd::binary::deserialize(&data).expect("deserialize");

    let values: Vec<&str> =
        ft.tokens.iter().map(|t| t.value.as_str()).collect();

    // "const" should appear exactly twice (before and after the ignored section)
    let const_count = values.iter().filter(|v| **v == "const").count();
    assert_eq!(
        const_count, 2,
        "should have 2 'const' tokens (ignoring middle section), got {:?}",
        values
    );
}
