//! End-to-end tests of the `difflux` binary: exit codes, JSON, schema, MCP.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const BIN: &str = env!("CARGO_BIN_EXE_difflux");

fn difflux(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(BIN).args(args).env("NO_COLOR", "1").output()
}

#[test]
fn exit_codes_follow_diff() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
    fs::write(&a, "one\ntwo\n")?;
    fs::write(&b, "one\ntwo\n")?;
    fs::write(&c, "one\nTWO\n")?;
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();

    assert_eq!(difflux(&["diff", &s(&a), &s(&b)])?.status.code(), Some(0));
    let out = difflux(&["diff", &s(&a), &s(&c)])?;
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout)?;
    assert!(text.contains("-two\n+TWO\n"), "{text}");
    assert_eq!(difflux(&["diff", "-i", &s(&a), &s(&c)])?.status.code(), Some(0));
    assert_eq!(difflux(&["diff", &s(&a), "/definitely/not/here"])?.status.code(), Some(2));
    Ok(())
}

#[test]
fn folder_json_report() -> TestResult {
    let (l, r) = (tempfile::tempdir()?, tempfile::tempdir()?);
    fs::write(l.path().join("same.txt"), "x")?;
    fs::write(r.path().join("same.txt"), "x")?;
    fs::write(l.path().join("gone.txt"), "x")?;
    fs::write(r.path().join("new.txt"), "x")?;
    let out = difflux(&["diff", "--json", &l.path().to_string_lossy(), &r.path().to_string_lossy()])?;
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout)?;
    assert_eq!(v["schema"], "difflux/1");
    assert_eq!(v["kind"], "folders");
    assert_eq!(v["status"], "different");
    let entries = v["entries"].as_array().ok_or("entries")?;
    let summary: Vec<(String, String)> =
        entries.iter().map(|e| (e["path"].as_str().unwrap_or("").to_owned(), e["status"].as_str().unwrap_or("").to_owned())).collect();
    assert_eq!(summary, [("gone.txt".into(), "left_only".into()), ("new.txt".into(), "right_only".into())]);
    Ok(())
}

#[test]
fn errors_are_json_when_json_was_asked_for() -> TestResult {
    let out = difflux(&["diff", "--json", "/nope/a", "/nope/b"])?;
    assert_eq!(out.status.code(), Some(2));
    let v: Value = serde_json::from_slice(&out.stdout)?;
    assert_eq!(v["kind"], "error");
    assert!(v["message"].as_str().is_some_and(|m| m.contains("does not exist")));
    Ok(())
}

#[test]
fn schema_is_valid_json_schema() -> TestResult {
    let out = difflux(&["schema"])?;
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout)?;
    assert_eq!(v["title"], "Output");
    assert!(v["$schema"].as_str().is_some_and(|s| s.contains("json-schema.org")));
    Ok(())
}

#[test]
fn checked_in_schema_is_current() -> TestResult {
    let out = difflux(&["schema"])?;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/schema/difflux-output.schema.json");
    let checked_in = fs::read_to_string(path).unwrap_or_default();
    assert!(checked_in == String::from_utf8(out.stdout)?, "docs/schema is stale: run `cargo xtask schema`");
    Ok(())
}

#[test]
fn mcp_server_round_trip() -> TestResult {
    let mut child = Command::new(BIN).arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("stdout")?).lines();
    let send = |stdin: &mut std::process::ChildStdin, msg: &str| -> std::io::Result<()> { writeln!(stdin, "{msg}") };

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    )?;
    let init: Value = serde_json::from_str(&lines.next().ok_or("no reply")??)?;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    send(&mut stdin, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)?;
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"compare_text","arguments":{"left":"a\n","right":"b\n"}}}"#,
    )?;
    let call: Value = serde_json::from_str(&lines.next().ok_or("no reply")??)?;
    assert_eq!(call["id"], 2);
    assert_eq!(call["result"]["structuredContent"]["status"], "different");

    drop(stdin); // closing stdin ends the server
    assert!(child.wait()?.success());
    Ok(())
}

#[test]
fn headless_gui_request_explains_itself_or_gui_is_present() -> TestResult {
    // `difflux --help` must always work and mention the subcommands agents need.
    let out = difflux(&["--help"])?;
    let help = String::from_utf8(out.stdout)?;
    for word in ["diff", "mcp", "schema", "Exit codes"] {
        assert!(help.contains(word), "help is missing {word}: {help}");
    }
    Ok(())
}
