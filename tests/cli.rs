//! End-to-end: the binary on the bundled demo and on a generated project.

use includecost::json::{self, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_includecost"))
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().expect("binary runs")
}

fn demo_db() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo/compile_commands.json")
}

fn header<'a>(report: &'a Value, path: &str) -> &'a Value {
    report
        .get("headers")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|h| h.get("path").and_then(Value::as_str) == Some(path))
        .unwrap_or_else(|| panic!("{path} not in report"))
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap()
}

#[test]
fn demo_report_separates_exclusive_from_inclusive() {
    let out = run(&[
        "analyze",
        demo_db().to_str().unwrap(),
        "--json",
        "--top",
        "0",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json::parse(&String::from_utf8(out.stdout).unwrap()).expect("valid JSON");
    assert_eq!(num(&report, "translation_units"), 3.0);

    // app.h reaches log.h, socket.h and json.h, but main.cpp includes log.h
    // itself and server.cpp includes socket.h itself: only json.h is carried
    // by app.h alone.
    let app = header(&report, "include/app/app.h");
    assert!(num(app, "inclusive_bytes") > 1.8 * num(app, "exclusive_bytes"));
    let carries = app.get("carries").and_then(Value::as_array).unwrap();
    assert_eq!(
        carries[0].get("path").and_then(Value::as_str),
        Some("include/json/json.h")
    );

    let parser = header(&report, "include/json/detail/parser.h");
    assert_eq!(
        num(parser, "exclusive_bytes"),
        num(parser, "inclusive_bytes")
    );
    let win = header(&report, "include/net/platform_win.h");
    assert_eq!(win.get("conditional"), Some(&Value::Bool(true)));

    let unresolved = report.get("unresolved").and_then(Value::as_array).unwrap();
    assert!(unresolved
        .iter()
        .any(|u| u.get("spelling").and_then(Value::as_str) == Some("vector")));
}

#[test]
fn text_report_respects_top() {
    let out = run(&["analyze", demo_db().to_str().unwrap(), "--top", "2"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("3 translation units, 9 headers"), "{text}");
    let ranks: Vec<&str> = text
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("rank"))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|w| w.parse::<u32>().is_ok())
        .collect();
    assert_eq!(ranks, ["1", "2"], "{text}");
    assert!(text.contains("... 7 more"), "{text}");
}

#[test]
fn generate_then_analyze_round_trip() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("gen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = run(&[
        "generate",
        dir.to_str().unwrap(),
        "--headers",
        "60",
        "--tus",
        "9",
        "--seed",
        "3",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let db = dir.join("compile_commands.json");
    assert!(db.is_file());

    let out = run(&[
        "analyze",
        db.to_str().unwrap(),
        "--json",
        "--top",
        "5",
        "--jobs",
        "2",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json::parse(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(num(&report, "translation_units"), 9.0);
    let headers = report.get("headers").and_then(Value::as_array).unwrap();
    assert_eq!(headers.len(), 5);
    let exclusive: Vec<f64> = headers.iter().map(|h| num(h, "exclusive_bytes")).collect();
    assert!(exclusive.windows(2).all(|w| w[0] >= w[1]));
    assert!(exclusive[0] > 0.0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn errors_have_distinct_exit_codes() {
    let missing = run(&["analyze", "/definitely/not/here/compile_commands.json"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));

    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("compile_commands.json");
    std::fs::write(&bad, "[{\"directory\": 5}]").unwrap();
    let malformed = run(&["analyze", bad.to_str().unwrap()]);
    assert_eq!(malformed.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("missing \"directory\""));

    assert_eq!(run(&[]).status.code(), Some(2));
    assert_eq!(run(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(run(&["analyze"]).status.code(), Some(2));
    assert_eq!(
        run(&["analyze", "x.json", "--top", "many"]).status.code(),
        Some(2)
    );
    assert_eq!(
        run(&["analyze", "x.json", "--bogus"]).status.code(),
        Some(2)
    );
    assert!(run(&["--help"]).status.success());
}
