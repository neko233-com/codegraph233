use crate::config::Config;
use crate::indexer;
use crate::query::Engine;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn a_str(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
fn a_i64(v: &Value, k: &str, def: i64) -> i64 {
    v.get(k).and_then(|x| x.as_i64()).unwrap_or(def)
}
fn a_bool(v: &Value, k: &str, def: bool) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(def)
}

fn tool(name: &str, desc: &str, props: Value, required: Value) -> Value {
    json!({
        "name": name,
        "description": desc,
        "inputSchema": { "type": "object", "properties": props, "required": required }
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool("cg233_status", "Index status: profile, languages, counts, db path. Call first.", json!({}), json!([])),
        tool("cg233_index", "Build or incrementally update the index (C# front-end in this build).", json!({
            "force": {"type":"boolean","description":"Re-parse every file"},
            "jobs": {"type":"integer","description":"Parallel workers"}
        }), json!([])),
        tool("cg233_find_symbol", "Search declarations by name/qualname substring.", json!({
            "query": {"type":"string"}, "kind": {"type":"string"}, "lang": {"type":"string"},
            "root": {"type":"string"}, "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["query"])),
        tool("cg233_get_symbol", "Read one symbol by id, optionally with source.", json!({
            "id": {"type":"integer"}, "include_source": {"type":"boolean"}, "max_lines": {"type":"integer"}
        }), json!(["id"])),
        tool("cg233_file_outline", "List declarations in a file (path substring).", json!({
            "path": {"type":"string"}, "root": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["path"])),
        tool("cg233_find_referencing_symbols", "Who references/calls this symbol.", json!({
            "id": {"type":"integer"}, "limit": {"type":"integer"}
        }), json!(["id"])),
        tool("cg233_call_graph", "Callers (in) or callees (out), up to N hops.", json!({
            "id": {"type":"integer"}, "direction": {"type":"string"}, "depth": {"type":"integer"},
            "limit": {"type":"integer"}
        }), json!(["id"])),
        tool("cg233_impact", "Change-impact set for a symbol id or file path.", json!({
            "id": {"type":"integer"}, "path": {"type":"string"}, "depth": {"type":"integer"}
        }), json!([])),
        tool("cg233_search_pattern", "Regex search with context across indexed files.", json!({
            "pattern": {"type":"string"}, "glob": {"type":"string"}, "root": {"type":"string"},
            "max_results": {"type":"integer"}, "context_lines": {"type":"integer"}
        }), json!(["pattern"])),
        tool("cg233_asset_search", "Search prefab/scene/shader/config assets by name/path/kind.", json!({
            "name": {"type":"string"}, "kind": {"type":"string"}, "root": {"type":"string"},
            "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_asset_info", "Asset metadata plus in/out references.", json!({
            "key": {"type":"string"}, "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_asset_refs", "Asset relationships (out/in) for a key.", json!({
            "key": {"type":"string"}, "direction": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["key"])),
        tool("cg233_guid_lookup", "Resolve a guid or asset path and list its references.", json!({
            "guid": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["guid"])),
        tool("cg233_project_health", "Counts, unresolved refs, guid coverage, db size.", json!({}), json!([])),
    ]
}

fn call(cfg: &Config, name: &str, args: &Value) -> Result<Value, String> {
    if name == "cg233_index" {
        let opts = indexer::Options {
            force: a_bool(args, "force", false),
            jobs: a_i64(args, "jobs", 0) as usize,
            quiet: true,
            fast: true,
        };
        let stats = indexer::run(cfg, &opts)?;
        return Ok(json!({
            "files": stats.files, "changed": stats.changed, "removed": stats.removed,
            "symbols": stats.symbols, "refs": stats.refs, "edges": stats.edges,
            "unresolved_refs": stats.unresolved, "elapsed_ms": stats.elapsed_ms,
        }));
    }
    if name == "cg233_status" {
        let st = indexer::status(cfg);
        return Ok(json!({
            "config": cfg.file.display().to_string(),
            "db": st.db, "exists": st.exists, "files": st.files, "symbols": st.symbols,
            "refs": st.refs, "edges": st.edges, "indexed_at": st.indexed_at,
            "profile": cfg.raw.profile, "languages": cfg.languages(),
        }));
    }

    let eng = Engine::open(cfg)?;
    match name {
        "cg233_find_symbol" => {
            let hits = eng.search_symbols(
                &a_str(args, "query"),
                Some(&a_str(args, "kind")),
                Some(&a_str(args, "lang")),
                Some(&a_str(args, "root")),
                Some(&a_str(args, "path")),
                a_i64(args, "limit", 30),
            )?;
            Ok(json!(hits))
        }
        "cg233_get_symbol" => {
            let id = a_i64(args, "id", 0);
            let sym = eng.get_symbol(id)?;
            if a_bool(args, "include_source", true) {
                let root = sym["root"].as_str().unwrap_or("");
                let path = sym["path"].as_str().unwrap_or("");
                let start = sym["start_line"].as_i64().unwrap_or(1) as i32;
                let end = sym["end_line"].as_i64().unwrap_or(start as i64) as i32;
                let src = eng.source(root, path, start, end, a_i64(args, "max_lines", 120) as i32);
                Ok(json!({ "symbol": sym, "source": src }))
            } else {
                Ok(json!({ "symbol": sym }))
            }
        }
        "cg233_file_outline" => Ok(json!(eng.file_outline(
            &a_str(args, "path"),
            Some(&a_str(args, "root")),
            a_i64(args, "limit", 400)
        )?)),
        "cg233_find_referencing_symbols" => Ok(json!(eng.refs_to(
            a_i64(args, "id", 0),
            a_i64(args, "limit", 100)
        )?)),
        "cg233_call_graph" => Ok(json!(eng.related(
            a_i64(args, "id", 0),
            &a_str(args, "direction"),
            a_i64(args, "depth", 1),
            a_i64(args, "limit", 100)
        )?)),
        "cg233_impact" => {
            let id = a_i64(args, "id", 0);
            let path = a_str(args, "path");
            Ok(eng.impact(
                if id > 0 { Some(id) } else { None },
                if path.is_empty() { None } else { Some(path.as_str()) },
                a_i64(args, "depth", 2),
            )?)
        }
        "cg233_search_pattern" => Ok(json!(eng.search_pattern(
            &a_str(args, "pattern"),
            &a_str(args, "glob"),
            &a_str(args, "root"),
            a_i64(args, "max_results", 50),
            a_i64(args, "context_lines", 2)
        )?)),
        "cg233_asset_search" => Ok(json!(eng.asset_search(
            &a_str(args, "name"),
            &a_str(args, "kind"),
            &a_str(args, "root"),
            &a_str(args, "path"),
            a_i64(args, "limit", 50)
        )?)),
        "cg233_asset_info" => Ok(eng.asset_info(
            &a_str(args, "key"),
            &a_str(args, "path"),
            a_i64(args, "limit", 200)
        )?),
        "cg233_asset_refs" => Ok(json!(eng.asset_edges(
            &a_str(args, "key"),
            &a_str(args, "direction"),
            a_i64(args, "limit", 200)
        )?)),
        "cg233_guid_lookup" => Ok(json!(eng.guid_lookup(
            &a_str(args, "guid"),
            a_i64(args, "limit", 100)
        )?)),
        "cg233_project_health" => Ok(eng.health("")?),
        other => Err(format!("unknown tool: {other}")),
    }
}

pub fn serve(cfg: Config) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("codegraph233: bad json: {e}");
                continue;
            }
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_null() {
            continue; // notification
        }
        let resp = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "codegraph233", "version": "0.2.0"},
                    "instructions": "AST GraphRAG (Rust). Call cg233_status first, build with cg233_index, then cg233_find_symbol / cg233_call_graph / cg233_asset_* / cg233_guid_lookup."
                }
            }),
            "ping" => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            "tools/list" => json!({"jsonrpc":"2.0","id":id,"result":{"tools": tools()}}),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match call(&cfg, name, &args) {
                    Ok(v) => {
                        let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into());
                        json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":text}]}})
                    }
                    Err(e) => json!({"jsonrpc":"2.0","id":id,"result":{
                        "content":[{"type":"text","text":format!("ERROR: {e}")}], "isError": true
                    }}),
                }
            }
            _ => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not found: {method}")}}),
        };
        let text = serde_json::to_string(&resp).unwrap_or_default();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }
    Ok(())
}
