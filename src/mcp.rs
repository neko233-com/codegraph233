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
    let mut defs = vec![
        tool("cg233_status", "Index status: profile, languages, counts, db path, schema freshness. Call first.", json!({}), json!([])),
        tool("cg233_index", "Build/update source and ordinary JVM bytecode graphs; preserves unresolved targets and parse diagnostics.", json!({
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
        tool("cg233_overview", "Orientation: languages, symbol kinds, top namespaces, asset kinds, health counts.", json!({}), json!([])),
        tool("cg233_symbol_path", "Namespace -> owning types -> symbol chain for one symbol id.", json!({
            "id": {"type":"integer"}
        }), json!(["id"])),
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
        tool("cg233_asset_tree", "Prefab/scene GameObject hierarchy with per-node script and asset edges.", json!({
            "key": {"type":"string"}, "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_asset_stats", "Asset counts and bytes per kind, plus the largest assets.", json!({
            "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_asset_orphans", "Unreferenced file-level assets, largest first.", json!({
            "kind": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_audio_graph", "Audio pipeline graph: Unity AudioMixer groups/snapshots, Wwise events/banks, FMOD events/buses and routing edges.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_art_graph", "Art pipeline graph: glTF meshes/materials/textures/animations, Aseprite layers/tags, FBX and Unity UPM package dependencies.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_design_graph", "Design/narrative graph: Markdown requirement ids and @code/@asset/@audio/@video/@art/@story/@value markers, Yarn/Ink nodes, subtitles and video headers, with implements/reference edges to code and assets.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_localization_graph", "Localization graph: gettext PO/POT entries, XLIFF units, RESX values and ARB strings with translated/fuzzy/untranslated status, plus subtitle assets and design @text links.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_localization_report", "Localization QA report: untranslated/fuzzy counts per language, placeholder mismatches ({0}, %s), overlong targets and duplicate keys. Aggregated issues only; use cg233_localization_graph for the raw entries.", json!({
            "root": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_audio_events", "Gameplay code to audio event bindings: Wwise PostEvent and FMOD PlayOneShot/createEvent call sites aggregated per event and resolved to wwise_event/fmod_event assets. Token-small by design.", json!({
            "root": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_dependencies", "Dependency graph from npm package.json, Cargo.toml, go.mod, requirements.txt, vcpkg.json, conanfile.txt, NuGet .csproj and Maven pom.xml, aggregated per package with versions and owning manifests.", json!({
            "root": {"type":"string"}, "ecosystem": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_ci_graph", "CI/build pipeline graph: Jenkinsfile stages, GitHub Actions / GitLab CI / Azure Pipelines / CircleCI jobs and steps, Makefile targets, with contains / uses_action / runs / depends_on edges.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_document_graph", "Office/PDF document graph: DOCX headings and markers, PPTX slides, PDF page/text metadata and @code/@asset/[[REQ]] links to source and requirements.", json!({
            "root": {"type":"string"}, "asset": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_heatmap", "Hot/cold file split: hot files were modified within hot_days (default from config, 30), cold files are older; returns counts, bytes and the oldest cold paths so agents can prioritise refreshes.", json!({
            "root": {"type":"string"}, "hot_days": {"type":"integer"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_refresh", "Incremental reindex (mtime/size driven) for change-driven loops; returns changed/removed counters and elapsed_ms. Use force=true only when parsers or config changed.", json!({
            "force": {"type":"boolean"}
        }), json!([])),
        tool("cg233_shader_info", "ShaderLab detail: properties, includes, keywords, HLSL functions, users.", json!({
            "key": {"type":"string"}, "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_config_tables", "Config tables (xlsx sheets, csv/tsv, json) with headers and row counts.", json!({
            "path": {"type":"string"}, "limit": {"type":"integer"}
        }), json!([])),
        tool("cg233_config_readers", "Code sites that mention a config table (load / hot-reload probes).", json!({
            "table": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["table"])),
        tool("cg233_guid_lookup", "Resolve a guid or asset path and list its references.", json!({
            "guid": {"type":"string"}, "limit": {"type":"integer"}
        }), json!(["guid"])),
        tool("cg233_projects", "List registered projects (name, root, indexed, counts) and which is default.", json!({}), json!([])),
        tool("cg233_project_health", "Counts, unresolved refs, guid coverage, db size.", json!({}), json!([])),
    ];
    for d in defs.iter_mut() {
        if d["name"] == "cg233_projects" {
            continue;
        }
        if let Some(props) = d["inputSchema"]["properties"].as_object_mut() {
            props.insert(
                "project".into(),
                json!({"type":"string","description":"Optional project name/root (see cg233_projects); defaults to the current project"}),
            );
        }
    }
    defs
}

fn call(default_cfg: &Config, name: &str, args: &Value) -> Result<Value, String> {
    if name == "cg233_projects" {
        let reg = crate::projects::load();
        let mut rows: Vec<Value> = Vec::new();
        let default_dir = default_cfg.dir.to_string_lossy().to_string();
        for p in &reg.projects {
            let cfg = crate::config::load(std::path::Path::new(&p.root)).ok();
            let st = cfg.as_ref().map(crate::indexer::status);
            rows.push(json!({
                "name": p.name, "root": p.root,
                "default": crate::projects::same_root(&p.root, &default_dir),
                "indexed": st.as_ref().map(|s| s.exists).unwrap_or(false),
                "files": st.as_ref().map(|s| s.files).unwrap_or(0),
                "symbols": st.as_ref().map(|s| s.symbols).unwrap_or(0),
            }));
        }
        if !rows
            .iter()
            .any(|r| r["default"].as_bool().unwrap_or(false))
        {
            let st = crate::indexer::status(default_cfg);
            rows.push(json!({
                "name": std::path::Path::new(&default_dir).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                "root": default_dir, "default": true, "indexed": st.exists,
                "files": st.files, "symbols": st.symbols,
            }));
        }
        return Ok(json!(rows));
    }

    let sel = a_str(args, "project");
    let default_dir = default_cfg.dir.to_string_lossy().to_string();
    let root = crate::projects::resolve(&default_dir, &sel);
    let cfg: Config = if root == default_dir {
        default_cfg.clone()
    } else {
        crate::config::load(std::path::Path::new(&root))
            .map_err(|e| format!("project {root}: {e}"))?
    };
    let cfg = &cfg;

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
            "assets": stats.assets, "unresolved_refs": stats.unresolved, "parse_errors":stats.parse_errors,
            "elapsed_ms": stats.elapsed_ms,
        }));
    }
    if name == "cg233_status" {
        let st = indexer::status(cfg);
        return Ok(json!({
            "config": cfg.file.display().to_string(),
            "db": st.db, "exists": st.exists, "stale": st.stale,
            "reader_version":env!("CARGO_PKG_VERSION"),"parser_version":st.parser_version,"parse_errors":st.parse_errors,
            "files": st.files, "symbols": st.symbols,
            "refs": st.refs, "edges": st.edges, "assets": st.assets,
            "guid_entries": st.guid_entries,
            "indexed_at": st.indexed_at,
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
                  if sym["lang"] == "jvm" && !sym["path"].as_str().unwrap_or("").ends_with(".cg-jvm.jsonl") {
                      return Ok(json!({"symbol":sym,"source":null,"source_unavailable":"Binary JVM input: descriptors/member identities are available; Java source was not recovered."}));
                  }
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
        "cg233_overview" => eng.overview(&a_str(args, "root")),
        "cg233_symbol_path" => eng.symbol_path(a_i64(args, "id", 0)),
        "cg233_asset_tree" => eng.asset_tree(
            &a_str(args, "key"),
            &a_str(args, "path"),
            a_i64(args, "limit", 2000),
        ),
        "cg233_asset_stats" => eng.asset_stats(&a_str(args, "root"), a_i64(args, "limit", 20)),
        "cg233_asset_orphans" => Ok(json!(eng.asset_orphans(
            &a_str(args, "root"),
            &a_str(args, "kind"),
            a_i64(args, "limit", 50)
        )?)),
        "cg233_audio_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.audio_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_art_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.art_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_design_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.design_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_localization_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.localization_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_localization_report" => {
            let root = a_str(args, "root");
            eng.localization_report(
                if root.is_empty() { None } else { Some(root.as_str()) },
                a_i64(args, "limit", 100),
            )
        }
        "cg233_audio_events" => {
            let root = a_str(args, "root");
            eng.audio_events(
                if root.is_empty() { None } else { Some(root.as_str()) },
                a_i64(args, "limit", 100),
            )
        }
        "cg233_dependencies" => {
            let root = a_str(args, "root");
            let eco = a_str(args, "ecosystem");
            eng.dependencies(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if eco.is_empty() { None } else { Some(eco.as_str()) },
                a_i64(args, "limit", 100),
            )
        }
        "cg233_ci_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.ci_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_document_graph" => {
            let root = a_str(args, "root");
            let asset = a_str(args, "asset");
            eng.document_graph(
                if root.is_empty() { None } else { Some(root.as_str()) },
                if asset.is_empty() { None } else { Some(asset.as_str()) },
                a_i64(args, "limit", 300),
            )
        }
        "cg233_heatmap" => {
            let root = a_str(args, "root");
            eng.heatmap(
                if root.is_empty() { None } else { Some(root.as_str()) },
                args.get("hot_days").and_then(|v| v.as_u64()),
                a_i64(args, "limit", 200),
            )
        }
        "cg233_refresh" => eng.refresh(
            args.get("force").and_then(|v| v.as_bool()).unwrap_or(false),
            true,
        ),
        "cg233_shader_info" => eng.shader_info(
            &a_str(args, "key"),
            &a_str(args, "path"),
            a_i64(args, "limit", 100),
        ),        "cg233_config_tables" => Ok(json!(eng.config_tables(
            &a_str(args, "root"),
            &a_str(args, "path"),
            a_i64(args, "limit", 100)
        )?)),
        "cg233_config_readers" => eng.config_readers(
            &a_str(args, "table"),
            &a_str(args, "root"),
            a_i64(args, "limit", 50),
        ),
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
                    "serverInfo": {"name": "codegraph233", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "AST GraphRAG (Rust). cg233_status first, cg233_index to build/update. Code: cg233_overview / cg233_find_symbol / cg233_symbol_path / cg233_call_graph / cg233_impact. Assets: cg233_asset_search / cg233_asset_tree / cg233_asset_refs / cg233_asset_orphans. Pipelines: cg233_audio_graph (mixers/Wwise/FMOD) / cg233_art_graph (glTF/Aseprite/FBX/UPM) / cg233_design_graph (design docs/narrative/code markers) / cg233_localization_graph (PO/XLIFF/RESX/ARB text status) / cg233_localization_report (translations QA) / cg233_audio_events (code→Wwise/FMOD bindings). Data: cg233_config_tables / cg233_config_readers / cg233_dependencies (manifest package graph) / cg233_ci_graph (Jenkins/GitHub/GitLab/Azure/CircleCI/Make) / cg233_document_graph (DOCX/PPTX/PDF legal & design docs). Freshness: cg233_heatmap (hot/cold files) / cg233_refresh (incremental reindex). Physics: cg233_shader_info. guid: cg233_guid_lookup."
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
