use crate::config::{self, RawConfig};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Detect engine from ProjectSettings/ProjectVersion.txt (Tuanjie uses a `t`
/// patch level, Unity uses f/a/b/p).
pub fn detect_strategy(dir: &Path) -> (String, String) {
    let vf = dir.join("ProjectSettings").join("ProjectVersion.txt");
    if let Ok(text) = fs::read_to_string(&vf) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix("m_EditorVersion:") {
                let v = v.trim().to_string();
                let is_tj = v.contains('t');
                let engine = if is_tj {
                    format!("Tuanjie {v}")
                } else {
                    format!("Unity {v}")
                };
                return (if is_tj { "tuanjie".into() } else { "unity".into() }, engine);
            }
        }
    }
    if dir.join("Content").is_dir() {
        return ("ue".into(), "Unreal Engine".into());
    }
    ("generic".into(), String::new())
}

pub fn init(dir: &Path, backend: Option<&str>, profile: &str) -> Result<PathBuf, String> {
    let path = dir.join(config::CONFIG_NAME);
    let (strategy, _engine) = detect_strategy(dir);
    let mut roots = vec![config::Root {
        key: "client".into(),
        path: ".".into(),
        role: "frontend".into(),
    }];
    if let Some(b) = backend {
        if !b.is_empty() {
            roots.push(config::Root {
                key: "server".into(),
                path: b.into(),
                role: "backend".into(),
            });
        }
    }
    let raw = RawConfig {
        version: 1,
        strategy: strategy.clone(),
        profile: profile.to_string(),
        roots,
        languages: vec![if strategy == "generic" { "auto".into() } else { "csharp".into() }],
        exclude: Vec::new(),
        config_dirs: Vec::new(),
        features: Default::default(),
        guid_map: Vec::new(),
        jvm_release: crate::config::jvm_release(),
        c_annotations: Vec::new(),
        extra_extensions: Default::default(),
        link_rules: Vec::new(),
        hot_days: None,
    };
    let text = serde_json::to_string_pretty(&raw).map_err(|e| e.to_string())?;
    fs::write(&path, format!("{text}\n")).map_err(|e| e.to_string())?;

    // gitignore
    let gi = dir.join(".gitignore");
    let existing = fs::read_to_string(&gi).unwrap_or_default();
    if !existing.contains(".codegraph233/") {
        let mut s = existing;
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str("\n# codegraph233 本地索引（勿提交）\n/.codegraph233/\n");
        let _ = fs::write(&gi, s);
    }
    Ok(path)
}

fn home() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn client_path(client: &str, dir: &Path, global: bool) -> Option<PathBuf> {
    let base = if global {
        if client == "vscode" {
            let app = std::env::var("APPDATA").unwrap_or_default();
            PathBuf::from(app).join("Code").join("User")
        } else {
            home()
        }
    } else {
        dir.to_path_buf()
    };
    let p = match client {
        "codex" => base.join(".codex").join("config.toml"),
        "cursor" => base.join(".cursor").join("mcp.json"),
        "vscode" => base.join(".vscode").join("mcp.json"),
        "air" => base.join(".air").join("mcp.json"),
        "cowork" | "codely" => base.join(".codely-cli").join("settings.json"),
        "workbuddy" => base.join(".workbuddy").join("mcp.json"),
        "project" if !global => base.join(".mcp.json"),
        _ => return None,
    };
    Some(p)
}

fn server_entry(args: &[&str]) -> Value {
    json!({ "type": "stdio", "command": "codegraph233", "args": args })
}

fn strip_toml_section(text: &str, name: &str) -> String {
    let prefix = format!("[mcp_servers.{name}");
    let mut out: Vec<&str> = Vec::new();
    let mut skip = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('[') {
            skip = t.starts_with(&prefix);
            if skip {
                continue;
            }
        }
        if !skip {
            out.push(line);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn write_json_config(path: &Path, client: &str, entry: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut root: Map<String, Value> = if let Ok(text) = fs::read_to_string(path) {
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        Map::new()
    };
    let key = if client == "vscode" { "servers" } else { "mcpServers" };
    let mut section = root
        .get(key)
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    section.remove("game-project-graph");
    section.insert("codegraph233".into(), entry.clone());
    root.insert(key.into(), Value::Object(section));
    let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|e| e.to_string())
}

fn write_codex_config(path: &Path, args: &[&str]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let text = strip_toml_section(&existing, "codegraph233");
    let mut block = String::from("\n[mcp_servers.codegraph233]\ncommand = \"codegraph233\"\nargs = [");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            block.push_str(", ");
        }
        block.push_str(&format!("\"{}\"", a.replace('\\', "\\\\")));
    }
    block.push_str("]\n");
    fs::write(path, format!("{text}{block}")).map_err(|e| e.to_string())
}

/// Install MCP entries.  scope: "global" (default) | "project" | "both".
pub fn install_mcp(
    dir: &Path,
    clients: &str,
    scope: &str,
    clean_project: bool,
) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let project_args = ["serve", "--root", dir.to_str().unwrap_or(".")];
    let global_args = ["serve"];
    let entry_project = server_entry(&project_args);
    let _ = &entry_project;

    for raw in clients.split(',') {
        let client = raw.trim().to_ascii_lowercase();
        if client.is_empty() {
            continue;
        }
        if scope == "global" || scope == "both" {
            if let Some(p) = client_path(&client, dir, true) {
                if client == "codex" {
                    write_codex_config(&p, &global_args)?;
                } else {
                    write_json_config(&p, &client, &server_entry(&global_args))?;
                }
                done.push(format!("{} (global)", p.display()));
            }
        }
        if scope == "project" || scope == "both" {
            if let Some(p) = client_path(&client, dir, false) {
                if client == "codex" {
                    write_codex_config(&p, &project_args)?;
                } else {
                    write_json_config(&p, &client, &server_entry(&project_args))?;
                }
                done.push(format!("{} (project)", p.display()));
            }
        }
        if clean_project {
            if let Some(p) = client_path(&client, dir, false) {
                if client == "codex" {
                    if let Ok(text) = fs::read_to_string(&p) {
                        let stripped = strip_toml_section(&text, "codegraph233");
                        let _ = fs::write(&p, stripped);
                    }
                } else if let Ok(text) = fs::read_to_string(&p) {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&text) {
                        if let Some(obj) = v.as_object_mut() {
                            let key = if client == "vscode" { "servers" } else { "mcpServers" };
                            if let Some(sec) = obj.get_mut(key).and_then(|s| s.as_object_mut()) {
                                sec.remove("codegraph233");
                            }
                        }
                        let _ = fs::write(&p, format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default()));
                    }
                }
            }
        }
    }
    Ok(done)
}
