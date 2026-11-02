//! Model Context Protocol server over stdio (`difflux mcp`).
//!
//! Lets AI agents compare files, folders and in-memory text and get the same
//! structured reports as `difflux diff --json`. All tools are read-only.
//!
//! Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout; logs go to stderr.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use difflux_app::{Comparison, compare_paths, compare_texts};
use difflux_core::{CancelToken, CompareOptions, ContentMode, Progress, TextOptions, WhitespaceMode};
use serde_json::{Value, json};

use crate::output::{self, Limits, Output, Report};

/// Protocol revisions this server understands, newest first.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "difflux compares files, folders and text. Use compare_paths for anything on disk \
(absolute paths; files and folders are detected automatically) and compare_text for content you already hold. \
Results are JSON in the difflux/1 schema: for files, 'hunks' list removed/added lines with 1-based line numbers; \
for folders, 'entries' list every difference with a status and the basis of the verdict. \
'equivalent' means the bytes differ only in ignored whitespace, case or line endings.";

/// Serves requests until stdin closes.
pub fn serve(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&line) {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// Handles one JSON-RPC message; returns `None` for notifications.
#[must_use]
pub fn handle_line(line: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(&Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = msg.get("id").cloned()?; // notifications carry no id and get no reply
    let method = msg.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == requested).unwrap_or(&PROTOCOL_VERSIONS[1]);
            result(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "difflux", "title": "difflux", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => result(&id, json!({})),
        "tools/list" => result(&id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call_tool(name, &args) {
                Ok(v) => result(&id, v),
                Err(ToolError::UnknownTool) => error(&id, -32602, &format!("unknown tool: {name}")),
                Err(ToolError::Invalid(m)) => error(&id, -32602, &m),
            }
        }
        _ => error(&id, -32601, &format!("method not found: {method}")),
    })
}

#[allow(clippy::needless_pass_by_value)] // reads better at call sites: result(&id, json!({...}))
fn result(id: &Value, value: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": value })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rule_properties() -> Value {
    json!({
        "whitespace": { "type": "string", "enum": ["exact", "trailing", "change", "all"], "default": "exact",
            "description": "exact: every byte counts; trailing: ignore trailing spaces; change: ignore amount of whitespace; all: ignore all whitespace" },
        "ignore_case": { "type": "boolean", "default": false },
        "strict_line_endings": { "type": "boolean", "default": false, "description": "Treat CRLF and LF as different" },
        "context_lines": { "type": "integer", "minimum": 0, "default": 3 },
        "max_lines": { "type": "integer", "minimum": 1, "default": 2000, "description": "Cap on diff lines returned" },
    })
}

fn tools() -> Value {
    let mut paths_props = rule_properties();
    if let Some(map) = paths_props.as_object_mut() {
        map.insert("left".into(), json!({ "type": "string", "description": "Absolute path of the left file or folder" }));
        map.insert("right".into(), json!({ "type": "string", "description": "Absolute path of the right file or folder" }));
        map.insert("content".into(), json!({ "type": "string", "enum": ["smart", "exact", "metadata"], "default": "smart",
            "description": "Folders only. smart: equal size+mtime counts as identical, else bytes are read; exact: always read bytes; metadata: never read" }));
        map.insert("exclude".into(), json!({ "type": "array", "items": { "type": "string" }, "description": "Folders only. Glob patterns to skip, e.g. [\"target\", \"*.log\"]" }));
        map.insert(
            "include_identical".into(),
            json!({ "type": "boolean", "default": false, "description": "Folders only. Also list identical entries" }),
        );
        map.insert(
            "max_entries".into(),
            json!({ "type": "integer", "minimum": 1, "default": 1000, "description": "Folders only. Cap on entries returned" }),
        );
    }
    let mut text_props = rule_properties();
    if let Some(map) = text_props.as_object_mut() {
        map.insert("left".into(), json!({ "type": "string", "description": "Left text" }));
        map.insert("right".into(), json!({ "type": "string", "description": "Right text" }));
    }
    let annotations = |title: &str| json!({ "title": title, "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false });
    json!([
        {
            "name": "compare_paths",
            "title": "Compare files or folders",
            "description": "Compare two files or two folders on disk. Returns a difflux/1 JSON report: for files, a status and unified-diff style hunks; for folders, every entry that differs with its status (different, left_only, right_only, type_mismatch, error) and the basis of the verdict.",
            "inputSchema": { "type": "object", "properties": paths_props, "required": ["left", "right"] },
            "annotations": annotations("Compare files or folders"),
        },
        {
            "name": "compare_text",
            "title": "Compare two texts",
            "description": "Compare two pieces of text you already have in context (no files needed). Returns a difflux/1 JSON report with unified-diff style hunks.",
            "inputSchema": { "type": "object", "properties": text_props, "required": ["left", "right"] },
            "annotations": annotations("Compare two texts"),
        }
    ])
}

enum ToolError {
    UnknownTool,
    Invalid(String),
}

fn call_tool(name: &str, args: &Value) -> Result<Value, ToolError> {
    let text_opts = text_options(args)?;
    let limits = Limits {
        context: u32_arg(args, "context_lines", 3)?,
        max_lines: usize_arg(args, "max_lines", 2000)?,
        max_entries: usize_arg(args, "max_entries", 1000)?,
        include_identical: args.get("include_identical").and_then(Value::as_bool).unwrap_or(false),
    };
    let cancel = CancelToken::new();
    let report = match name {
        "compare_text" => {
            let left = str_arg(args, "left")?.to_owned();
            let right = str_arg(args, "right")?.to_owned();
            match compare_texts(left, right, &text_opts, &cancel) {
                Ok(c) => Report::Files(output::files_report(&c, limits)),
                Err(e) => Report::Error(output::ErrorReport { message: e.to_string() }),
            }
        }
        "compare_paths" => {
            let left = PathBuf::from(str_arg(args, "left")?);
            let right = PathBuf::from(str_arg(args, "right")?);
            let mut opts = CompareOptions { text: text_opts, ..CompareOptions::default() };
            opts.folder.content = match args.get("content").and_then(Value::as_str).unwrap_or("smart") {
                "smart" => ContentMode::Smart,
                "exact" => ContentMode::Exact,
                "metadata" => ContentMode::Metadata,
                other => return Err(ToolError::Invalid(format!("content must be smart, exact or metadata, not {other:?}"))),
            };
            if let Some(list) = args.get("exclude").and_then(Value::as_array) {
                opts.folder.excludes = list.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            match compare_paths(&left, &right, &opts, &cancel, &Progress::new()) {
                Ok(Comparison::Files(f)) => Report::Files(output::files_report(&f, limits)),
                Ok(Comparison::Folders(d)) => Report::Folders(output::folders_report(&d, &opts.folder, limits)),
                Err(e) => Report::Error(output::ErrorReport { message: e.to_string() }),
            }
        }
        _ => return Err(ToolError::UnknownTool),
    };
    let is_error = matches!(report, Report::Error(_));
    let structured = serde_json::to_value(Output::new(report)).map_err(|e| ToolError::Invalid(e.to_string()))?;
    Ok(json!({
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": is_error,
    }))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| ToolError::Invalid(format!("missing string argument {key:?}")))
}

fn usize_arg(args: &Value, key: &str, default: usize) -> Result<usize, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| ToolError::Invalid(format!("{key:?} must be a non-negative integer"))),
    }
}

fn u32_arg(args: &Value, key: &str, default: u32) -> Result<u32, ToolError> {
    let n = usize_arg(args, key, default as usize)?;
    u32::try_from(n).map_err(|_| ToolError::Invalid(format!("{key:?} is too large")))
}

fn text_options(args: &Value) -> Result<TextOptions, ToolError> {
    let whitespace = match args.get("whitespace").and_then(Value::as_str).unwrap_or("exact") {
        "exact" => WhitespaceMode::Exact,
        "trailing" => WhitespaceMode::Trailing,
        "change" => WhitespaceMode::Change,
        "all" => WhitespaceMode::All,
        other => return Err(ToolError::Invalid(format!("whitespace must be exact, trailing, change or all, not {other:?}"))),
    };
    Ok(TextOptions {
        whitespace,
        ignore_case: args.get("ignore_case").and_then(Value::as_bool).unwrap_or(false),
        ignore_line_endings: !args.get("strict_line_endings").and_then(Value::as_bool).unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(line: &str) -> Value {
        handle_line(line).unwrap_or(Value::Null)
    }

    #[test]
    fn negotiates_protocol_version() {
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#);
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[1]);
        assert_eq!(r["result"]["serverInfo"]["name"], "difflux");
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn lists_read_only_tools() {
        let r = call(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().cloned().unwrap_or_default();
        assert_eq!(tools.len(), 2);
        assert!(tools.iter().all(|t| t["annotations"]["readOnlyHint"] == true));
    }

    #[test]
    fn compare_text_returns_structured_hunks() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"compare_text","arguments":{"left":"a\nb\n","right":"a\nc\n"}}}"#,
        );
        let s = &r["result"]["structuredContent"];
        assert_eq!(s["schema"], "difflux/1");
        assert_eq!(s["kind"], "files");
        assert_eq!(s["status"], "different");
        assert_eq!(s["hunks"][0]["lines"][1], json!({"op": "removed", "left": 2, "text": "b"}));
        assert_eq!(s["hunks"][0]["lines"][2], json!({"op": "added", "right": 2, "text": "c"}));
        assert_eq!(r["result"]["isError"], false);
    }

    #[test]
    fn bad_arguments_are_reported() {
        let r = call(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"compare_text","arguments":{"left":"a"}}}"#);
        assert_eq!(r["error"]["code"], -32602);
        let r = call(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"compare_paths","arguments":{"left":"/nope/a","right":"/nope/b"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["structuredContent"]["kind"], "error");
    }

    #[test]
    fn unknown_methods_and_garbage() {
        assert_eq!(call(r#"{"jsonrpc":"2.0","id":6,"method":"resources/list"}"#)["error"]["code"], -32601);
        assert_eq!(call("{not json")["error"]["code"], -32700);
    }
}
