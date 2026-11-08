//! Engine-specific asset frontends.
//!
//! - Unreal Engine 5: `.uproject`, `.uplugin`, INI configuration and the
//!   header of cooked/editor `.uasset`/`.umap` packages.
//! - Godot: text `.tscn`/`.tres` resources and `project.godot`.
//!
//! These parsers are deliberately static: they never execute project files and
//! never decompress Unreal packages. Unreal package support is header-level
//! (magic, legacy/UE4/UE5 version, folder, name table sample); the prototype
//! tables are left to a future binary graph frontend.

use crate::model::{Asset, AssetEdge, FileParse};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const UE_MAGIC: u32 = 0xC1832A9E;

fn stem(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn empty(rel: &str, root: &str, lang: &str) -> FileParse {
    FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        ..Default::default()
    }
}

fn asset(rel: &str, root: &str, kind: &str, name: &str, meta: HashMap<String, String>) -> Asset {
    Asset {
        key: format!("asset:{root}:{rel}"),
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 0,
        meta,
    }
}

fn edge(res: &mut FileParse, root: &str, src: &str, dst: String, kind: &str, detail: &str) {
    res.asset_edges.push(AssetEdge {
        src: src.to_string(),
        dst,
        kind: kind.to_string(),
        detail: detail.to_string(),
        root: root.to_string(),
        line: 0,
    });
}

// --------------------------------------------------------------- Unreal text

pub fn parse_text(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    match lang {
        "ue_project" | "ue_plugin" => parse_engine_project(rel, root, text),
        "ue_config" => parse_ini(rel, root, text),
        "godot_scene" => parse_godot_scene(rel, root, text),
        "godot_project" => parse_godot_project(rel, root, text),
        _ => {
            let mut res = empty(rel, root, lang);
            res.errors.push(format!("no engine text frontend for {lang}"));
            res
        }
    }
}

fn parse_engine_project(rel: &str, root: &str, text: &str) -> FileParse {
    let plugin = rel.to_ascii_lowercase().ends_with(".uplugin");
    let lang = if plugin { "ue_plugin" } else { "ue_project" };
    let mut res = empty(rel, root, lang);
    let file_key = format!("asset:{root}:{rel}");
    let mut meta = HashMap::new();
    meta.insert("bytes".to_string(), text.len().to_string());
    let mut modules: Vec<String> = Vec::new();
    let mut plugins: Vec<String> = Vec::new();
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => {
            for (key, target) in [
                ("EngineAssociation", "engine"),
                ("Category", "category"),
                ("Description", "description"),
                ("VersionName", "version"),
                ("FriendlyName", "friendly_name"),
                ("Version", "version"),
                ("CreatedBy", "created_by"),
            ] {
                if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
                    meta.insert(target.to_string(), s.to_string());
                }
            }
            if let Some(list) = value.get("Modules").and_then(|v| v.as_array()) {
                for module in list {
                    if let Some(name) = module.get("Name").and_then(|v| v.as_str()) {
                        modules.push(name.to_string());
                        edge(&mut res, root, &file_key, format!("ue_module:{name}"), "contains_module", "module");
                    }
                }
            }
            if let Some(list) = value.get("Plugins").and_then(|v| v.as_array()) {
                for plugin in list {
                    let Some(name) = plugin.get("Name").and_then(|v| v.as_str()) else { continue };
                    let enabled = plugin
                        .get("Enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if enabled {
                        plugins.push(name.to_string());
                        edge(&mut res, root, &file_key, format!("ue_plugin:{name}"), "uses_plugin", "plugin");
                    }
                }
            }
            if !meta.contains_key("engine") {
                if let Some(engine) = value.get("EngineVersion").and_then(|v| v.as_str()) {
                    meta.insert("engine".into(), engine.to_string());
                }
            }
        }
        Err(error) => res.errors.push(format!("invalid unreal json: {error}")),
    }
    meta.insert("modules".into(), modules.join(","));
    meta.insert("module_count".into(), modules.len().to_string());
    meta.insert("plugins".into(), plugins.join(","));
    meta.insert("plugin_count".into(), plugins.len().to_string());
    res.assets.push(asset(rel, root, lang, &stem(rel), meta));
    res
}

fn parse_ini(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = empty(rel, root, "ue_config");
    let file_key = format!("asset:{root}:{rel}");
    let mut sections: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut keys = 0usize;
    let mut refs: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        // Section names and values both reference script/game paths.
        for (idx, _) in line.match_indices('/') {
            let rest = &line[idx..];
            if !(rest.starts_with("/Game/") || rest.starts_with("/Script/")) {
                continue;
            }
            let end = rest
                .char_indices()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')))
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            let path = rest[..end].trim_end_matches('.');
            if path.len() > 6 && !refs.iter().any(|r| r == path) {
                refs.push(path.to_string());
            }
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                section = line[1..end].trim().to_string();
                if !section.is_empty() && !sections.contains(&section) {
                    sections.push(section.clone());
                }
            }
            continue;
        }
        if line.contains('=') && !section.is_empty() {
            keys += 1;
        }
    }
    for path in &refs {
        if let Some(game) = path.strip_prefix("/Game/") {
            edge(&mut res, root, &file_key, format!("uegame:{game}"), "ue_reference", "game");
        } else if let Some(script) = path.strip_prefix("/Script/") {
            edge(&mut res, root, &file_key, format!("uescript:{script}"), "ue_reference", "script");
        }
    }
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    meta.insert("sections".into(), sections.join(","));
    meta.insert("section_count".into(), sections.len().to_string());
    meta.insert("keys".into(), keys.to_string());
    meta.insert("references".into(), refs.len().to_string());
    res.assets.push(asset(rel, root, "ue_config", &stem(rel), meta));
    res
}

// ------------------------------------------------------------ Unreal package

fn rd_u32(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn rd_i32(data: &[u8], pos: usize) -> Option<i32> {
    rd_u32(data, pos).map(|v| v as i32)
}

/// Unreal `FString`: i32 length including the terminator; negative length means
/// UTF-16LE code units.
fn read_fstring(data: &[u8], pos: usize) -> Option<(String, usize)> {
    let n = rd_i32(data, pos)?;
    if n == 0 {
        return Some((String::new(), pos + 4));
    }
    if n > 0 {
        let len = n as usize;
        let start = pos + 4;
        let end = start.checked_add(len)?;
        if len == 0 || end > data.len() {
            return None;
        }
        let body = &data[start..end - 1];
        if data[end - 1] != 0 {
            return None;
        }
        Some((String::from_utf8_lossy(body).into_owned(), end))
    } else {
        let units = n.unsigned_abs() as usize;
        let start = pos + 4;
        let end = start.checked_add(units.checked_mul(2)?)?;
        if units == 0 || end > data.len() {
            return None;
        }
        let mut text = String::new();
        for i in 0..units.saturating_sub(1) {
            let at = start + i * 2;
            let unit = u16::from_le_bytes([data[at], data[at + 1]]);
            text.push(char::from_u32(unit as u32).unwrap_or('\u{fffd}'));
        }
        Some((text, end))
    }
}

fn parse_name_table(data: &[u8], start: usize, count: usize, hash_gap: usize) -> Option<Vec<String>> {
    let mut pos = start;
    let mut names = Vec::new();
    for _ in 0..count {
        let (name, next) = read_fstring(data, pos)?;
        if next <= pos {
            return None;
        }
        names.push(name);
        pos = next + hash_gap;
        if pos > data.len() {
            return None;
        }
    }
    Some(names)
}

pub fn parse_uasset(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = empty(rel, root, "ue_asset");
    let kind = if rel.to_ascii_lowercase().ends_with(".umap") { "ue_map" } else { "ue_asset" };
    let mut meta = HashMap::new();
    meta.insert("format".into(), "unreal-package".to_string());
    meta.insert("bytes".into(), data.len().to_string());
    if data.len() >= 8 && rd_u32(data, 0) == Some(UE_MAGIC) {
        if data.len() < 16 {
            res.errors.push("unreal package header is truncated".into());
        }
        meta.insert("magic".into(), "0xC1832A9E".into());
        let legacy = rd_i32(data, 4).unwrap_or(0);
        meta.insert("legacy_version".into(), legacy.to_string());
        let mut pos = 8usize;
        if legacy != 0 {
            if let Some(v) = rd_i32(data, pos) {
                meta.insert("ue3_version".into(), v.to_string());
                pos += 4;
            }
            if legacy <= -2 {
                if let Some(v) = rd_i32(data, pos) {
                    meta.insert("ue4_version".into(), v.to_string());
                    pos += 4;
                }
            }
            if legacy <= -8 {
                if let Some(v) = rd_i32(data, pos) {
                    meta.insert("ue5_version".into(), v.to_string());
                    pos += 4;
                }
            }
            if let Some(v) = rd_i32(data, pos) {
                meta.insert("licensee_version".into(), v.to_string());
                pos += 4;
            }
            if legacy <= -4 {
                if let Some(count) = rd_i32(data, pos) {
                    pos += 4;
                    if !(0..=4096).contains(&count) {
                        res.errors.push(format!("implausible custom version count {count}"));
                        pos = data.len();
                    } else {
                        pos = pos.saturating_add((count as usize).saturating_mul(20));
                    }
                }
            }
            if pos < data.len() {
                if let Some(total) = rd_i32(data, pos) {
                    meta.insert("header_size".into(), total.to_string());
                    pos += 4;
                }
                if let Some((folder, next)) = read_fstring(data, pos) {
                    if !folder.is_empty() {
                        meta.insert("package_folder".into(), folder);
                    }
                    pos = next;
                }
                if let Some(flags) = rd_u32(data, pos) {
                    meta.insert("package_flags".into(), format!("{flags:#x}"));
                    pos += 4;
                }
                let name_count = rd_i32(data, pos).unwrap_or(-1);
                let name_offset = rd_i32(data, pos + 4).unwrap_or(-1);
                if name_count > 0 && name_offset > 0 {
                    let count = (name_count as usize).min(4096);
                    meta.insert("name_count".into(), name_count.to_string());
                    let start = name_offset as usize;
                    let plain = parse_name_table(data, start, count, 0);
                    let hashed = parse_name_table(data, start, count, 4);
                    let names = match (plain, hashed) {
                        (Some(a), Some(b)) => if b.len() > a.len() { b } else { a },
                        (Some(a), None) => a,
                        (None, Some(b)) => b,
                        (None, None) => Vec::new(),
                    };
                    if names.is_empty() {
                        res.errors.push("unreal name table not decodable (header-only view)".into());
                    } else {
                        meta.insert("names_parsed".into(), names.len().to_string());
                        let sample: Vec<&str> = names.iter().take(16).map(|s| s.as_str()).collect();
                        meta.insert("names_sample".into(), sample.join(","));
                    }
                }
            }
        }
    } else {
        res.errors.push("not an Unreal package (bad magic or truncated file)".into());
    }
    res.assets.push(asset(rel, root, kind, &stem(rel), meta));
    res
}

// ------------------------------------------------------------------- Godot

fn parse_attrs(text: &str) -> Vec<(String, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-' | b'/')) {
            i += 1;
        }
        if i == key_start {
            i += 1;
            continue;
        }
        let key = text[key_start..i].to_string();
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            let quote = bytes[i];
            i += 1;
            let val_start = i;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push((key, text[val_start..i].to_string()));
            i += 1;
        } else {
            let val_start = i;
            while i < bytes.len() && !(bytes[i] as char).is_ascii_whitespace() {
                i += 1;
            }
            out.push((key, text[val_start..i].to_string()));
        }
    }
    out
}

fn godot_res_key(root: &str, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("res://") {
        format!("asset:{root}:{rest}")
    } else if let Some(uid) = path.strip_prefix("uid://") {
        format!("godot_uid:{uid}")
    } else {
        format!("godot_path:{path}")
    }
}

pub fn parse_godot_scene(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = empty(rel, root, "godot_scene");
    let resource = rel.to_ascii_lowercase().ends_with(".tres");
    let kind = if resource { "godot_resource" } else { "godot_scene" };
    let file_key = format!("asset:{root}:{rel}");
    let name = stem(rel);
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    meta.insert("format".into(), if resource { "gd_resource" } else { "gd_scene" }.to_string());
    let mut ext: HashMap<String, (String, String)> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut node_count = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let Some(end) = line.find(']') else { continue };
        let inner = &line[1..end];
        let name_end = inner
            .char_indices()
            .find(|(_, c)| c.is_ascii_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(inner.len());
        let section = &inner[..name_end];
        let attrs = parse_attrs(&inner[name_end..]);
        let get = |key: &str| -> String {
            attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        match section {
            "gd_scene" | "gd_resource" => {
                if let Some(v) = attrs.iter().find(|(k, _)| k == "format").map(|(_, v)| v.clone()) {
                    meta.insert("format_version".into(), v);
                }
                meta.insert("load_steps".into(), get("load_steps"));
            }
            "ext_resource" => {
                let id = get("id");
                let typ = get("type");
                let path = get("path");
                let uid = get("uid");
                let target = if !path.is_empty() { path.clone() } else { uid.clone() };
                if !id.is_empty() {
                    ext.insert(id, (typ.clone(), target.clone()));
                }
                if !target.is_empty() {
                    let key = format!("{file_key}|ext|{target}");
                    if seen.insert(key) {
                        edge(
                            &mut res,
                            root,
                            &file_key,
                            godot_res_key(root, &target),
                            "references",
                            if typ.is_empty() { "ext_resource" } else { &typ },
                        );
                    }
                }
            }
            "node" => {
                node_count += 1;
                let node = get("name");
                let typ = get("type");
                let mut node_meta = HashMap::new();
                node_meta.insert("type".into(), typ.clone());
                node_meta.insert("parent".into(), get("parent"));
                if !node.is_empty() {
                    let key = format!("{file_key}#node:{node_count}:{node}");
                    res.assets.push(Asset {
                        key: key.clone(),
                        root: root.to_string(),
                        kind: "godot_node".into(),
                        path: rel.to_string(),
                        name: node.clone(),
                        unity_id: String::new(),
                        line: 0,
                        meta: node_meta,
                    });
                    edge(&mut res, root, &file_key, key, "contains", "node");
                }
                let instance = get("instance");
                if let Some(rest) = instance.strip_prefix("ExtResource(") {
                    let id = rest.trim_end_matches(')').trim_matches('"').trim();
                    if let Some((typ, path)) = ext.get(id) {
                        edge(
                            &mut res,
                            root,
                            &file_key,
                            godot_res_key(root, path),
                            "instantiates",
                            if typ.is_empty() { "instance" } else { typ },
                        );
                    }
                }
            }
            "sub_resource" => {
                let id = get("id");
                let typ = get("type");
                if !id.is_empty() {
                    let mut sub_meta = HashMap::new();
                    sub_meta.insert("type".into(), typ);
                    res.assets.push(Asset {
                        key: format!("{file_key}#sub:{id}"),
                        root: root.to_string(),
                        kind: "godot_sub_resource".into(),
                        path: rel.to_string(),
                        name: id,
                        unity_id: String::new(),
                        line: 0,
                        meta: sub_meta,
                    });
                }
            }
            _ => {}
        }
    }
    meta.insert("ext_resources".into(), ext.len().to_string());
    meta.insert("nodes".into(), node_count.to_string());
    res.assets.insert(0, asset(rel, root, kind, &name, meta));
    res
}

pub fn parse_godot_project(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = empty(rel, root, "godot_project");
    let file_key = format!("asset:{root}:{rel}");
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    let mut section = String::new();
    let mut sections: Vec<String> = Vec::new();
    let mut autoloads: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                section = line[1..end].trim().to_string();
                if !section.is_empty() && !sections.contains(&section) {
                    sections.push(section.clone());
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match (section.as_str(), key) {
            ("application", "config/name") => {
                meta.insert("name".into(), value.to_string());
            }
            ("application", "run/main_scene") => {
                let scene = value.trim_start_matches("res://");
                meta.insert("main_scene".into(), scene.to_string());
                edge(&mut res, root, &file_key, godot_res_key(root, value), "main_scene", "scene");
            }
            ("autoload", _) => {
                let path = value.trim_start_matches('*');
                if path.starts_with("res://") {
                    autoloads.push(format!("{key}={path}"));
                    edge(&mut res, root, &file_key, godot_res_key(root, path), "autoload", key);
                }
            }
            _ => {}
        }
    }
    meta.insert("sections".into(), sections.join(","));
    meta.insert("autoloads".into(), autoloads.join(";"));
    res.assets.push(asset(rel, root, "godot_project", "project", meta));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uproject_json_emits_project_modules_and_plugins() {
        let text = r#"{
          "FileVersion": 3,
          "EngineAssociation": "5.4",
          "Category": "",
          "Modules": [{"Name": "MyGame", "Type": "Runtime", "LoadingPhase": "Default"}],
          "Plugins": [{"Name": "EnhancedInput", "Enabled": true}, {"Name": "OldThing", "Enabled": false}]
        }"#;
        let fp = parse_text("MyGame.uproject", "app", "ue_project", text);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let project = &fp.assets[0];
        assert_eq!(project.kind, "ue_project");
        assert_eq!(project.meta.get("engine").map(|s| s.as_str()), Some("5.4"));
        assert_eq!(project.meta.get("modules").map(|s| s.as_str()), Some("MyGame"));
        let dsts: Vec<&str> = fp.asset_edges.iter().map(|e| e.dst.as_str()).collect();
        assert!(dsts.contains(&"ue_module:MyGame"));
        assert!(dsts.contains(&"ue_plugin:EnhancedInput"));
        assert!(!dsts.contains(&"ue_plugin:OldThing"), "disabled plugins are skipped");
    }

    #[test]
    fn uplugin_json_reports_dependencies() {
        let text = r#"{"FileVersion":3,"Version":1,"VersionName":"1.0","FriendlyName":"Combat",
            "Modules":[{"Name":"Combat"}],"Plugins":[{"Name":"GameplayAbilities"}]}"#;
        let fp = parse_text("Combat.uplugin", "app", "ue_plugin", text);
        assert_eq!(fp.assets[0].kind, "ue_plugin");
        assert_eq!(fp.assets[0].meta.get("friendly_name").map(|s| s.as_str()), Some("Combat"));
        assert!(fp.asset_edges.iter().any(|e| e.dst == "ue_plugin:GameplayAbilities"));
    }

    #[test]
    fn ini_collects_sections_and_game_paths() {
        let text = "[/Script/EngineSettings.GameMapsSettings]\nGameDefaultMap=/Game/Maps/Main.Main\n\
                    EditorStartupMap=/Game/Maps/Editor.Editor\n\n[CoreRedirects]\n+ClassRedirects=(OldName=\"A\",NewName=\"B\")\n";
        let fp = parse_text("Config/DefaultEngine.ini", "app", "ue_config", text);
        assert!(fp.errors.is_empty());
        let cfg = &fp.assets[0];
        assert_eq!(cfg.kind, "ue_config");
        assert_eq!(cfg.meta.get("keys").map(|s| s.as_str()), Some("3"));
        assert!(cfg.meta.get("sections").unwrap().contains("GameMapsSettings"));
        let dsts: Vec<&str> = fp.asset_edges.iter().map(|e| e.dst.as_str()).collect();
        assert!(dsts.contains(&"uegame:Maps/Main.Main"));
        assert!(dsts.contains(&"uescript:EngineSettings.GameMapsSettings"));
    }

    fn fstring(s: &str, out: &mut Vec<u8>) {
        out.extend_from_slice(&((s.len() + 1) as i32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }

    fn synthetic_uasset() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&UE_MAGIC.to_le_bytes());
        b.extend_from_slice(&(-8i32).to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&522i32.to_le_bytes());
        b.extend_from_slice(&1009i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes()); // custom versions
        b.extend_from_slice(&0i32.to_le_bytes()); // total header size
        b.extend_from_slice(&0i32.to_le_bytes()); // folder FString ""
        b.extend_from_slice(&0u32.to_le_bytes()); // package flags
        b.extend_from_slice(&3i32.to_le_bytes()); // name count
        let offset = b.len() as i32 + 4;
        b.extend_from_slice(&offset.to_le_bytes());
        assert_eq!(offset as usize, b.len(), "offset check");
        fstring("Player", &mut b);
        fstring("Default__Player", &mut b);
        fstring("Health", &mut b);
        b
    }

    #[test]
    fn uasset_header_reports_versions_and_name_sample() {
        let data = synthetic_uasset();
        let fp = parse_uasset("Content/Hero.uasset", "app", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let meta = &fp.assets[0].meta;
        assert_eq!(meta.get("ue5_version").map(|s| s.as_str()), Some("1009"));
        assert_eq!(meta.get("name_count").map(|s| s.as_str()), Some("3"));
        assert_eq!(meta.get("names_sample").map(|s| s.as_str()), Some("Player,Default__Player,Health"));
    }

    #[test]
    fn malformed_uasset_is_reported_not_panicking() {
        let fp = parse_uasset("Content/Bad.uasset", "app", b"not a package");
        assert!(!fp.errors.is_empty());
        assert_eq!(fp.assets[0].kind, "ue_asset");
    }

    #[test]
    fn godot_scene_links_scripts_and_instances() {
        let text = r#"[gd_scene load_steps=3 format=3]

[ext_resource type="Script" path="res://src/player.gd" id="1_abc"]
[ext_resource type="PackedScene" path="res://scenes/Enemy.tscn" id="2_def"]

[sub_resource type="CircleShape2D" id="Circle_1"]

[node name="Player" type="CharacterBody2D"]
script = ExtResource("1_abc")

[node name="Enemy" parent="." instance=ExtResource("2_def")]
"#;
        let fp = parse_text("scenes/Main.tscn", "app", "godot_scene", text);
        assert!(fp.errors.is_empty());
        let dsts: Vec<&str> = fp.asset_edges.iter().map(|e| e.dst.as_str()).collect();
        assert!(dsts.contains(&"asset:app:src/player.gd"));
        assert!(dsts.contains(&"asset:app:scenes/Enemy.tscn"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "instantiates"));
        let kinds: Vec<&str> = fp.assets.iter().map(|a| a.kind.as_str()).collect();
        assert!(kinds.contains(&"godot_scene"));
        assert!(kinds.contains(&"godot_node"));
        assert!(kinds.contains(&"godot_sub_resource"));
    }

    #[test]
    fn godot_project_records_autoloads_and_main_scene() {
        let text = "[application]\n\nconfig/name=\"My Game\"\nrun/main_scene=\"res://scenes/Main.tscn\"\n\n[autoload]\n\nPlayer=\"*res://src/player.gd\"\n";
        let fp = parse_text("project.godot", "app", "godot_project", text);
        assert_eq!(fp.assets[0].meta.get("name").map(|s| s.as_str()), Some("My Game"));
        let dsts: Vec<&str> = fp.asset_edges.iter().map(|e| e.dst.as_str()).collect();
        assert!(dsts.contains(&"asset:app:scenes/Main.tscn"));
        assert!(dsts.contains(&"asset:app:src/player.gd"));
    }
}
