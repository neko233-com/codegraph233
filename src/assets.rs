//! Game asset layer: prefab/scene YAML, ShaderLab, media headers, config tables,
//! plaintext .meta and editor-exported guid maps.

use crate::model::{Asset, AssetEdge, FileParse, GuidRow};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub fn asset_kind(rel: &str) -> &'static str {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "exr" | "tif" | "tiff" | "bmp" | "gif"
        | "webp" | "hdr" | "dds" => "image",
        "wav" | "mp3" | "ogg" | "oga" | "m4a" | "aac" | "flac" | "aif" | "aiff" | "wma" => "audio",
        "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" => "video",
        "ttf" | "otf" | "ttc" => "font",
        "atlas" => "spine_atlas",
        "skel" => "spine_data",
        "mat" => "material",
        "anim" => "animation_clip",
        "controller" => "animator_controller",
        "mixer" => "audio_mixer",
        "playable" => "timeline",
        "asmdef" => "assembly_def",
        "spriteatlas" => "sprite_atlas",
        "shader" | "hlsl" | "cginc" | "compute" => "shader",
        "asset" => "scriptable_asset",
        "json" => "json_asset",
        "meta" => "meta",
        _ => "asset",
    }
}

pub fn is_asset_ext(rel: &str) -> bool {    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "exr" | "tif" | "tiff" | "bmp" | "gif"
            | "webp" | "hdr" | "dds" | "wav" | "mp3" | "ogg" | "oga" | "m4a" | "aac" | "flac"
            | "aif" | "aiff" | "wma" | "mp4" | "mov" | "webm" | "mkv" | "avi" | "m4v" | "ttf"
            | "otf" | "ttc" | "atlas" | "skel" | "mat" | "anim" | "controller" | "mixer"
            | "playable" | "asmdef" | "spriteatlas" | "meta"
    )
}

/// Extensions parsed as reference-bearing YAML/JSON instead of opaque media.
pub fn is_structured_asset(rel: &str) -> bool {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mat" | "anim" | "controller" | "playable" | "mixer" | "asset" | "spriteatlas" | "asmdef"
    )
}

fn base_name(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn asset_node(rel: &str, root: &str, kind: &str, name: &str, meta: HashMap<String, String>) -> Asset {
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

// ---------------------------------------------------------------- prefab/scene

pub fn parse_prefab(rel: &str, root: &str, text: &str) -> FileParse {
    #[derive(Default)]
    struct Doc {
        class_id: String,
        file_id: String,
        name: String,
        game_obj: String,
        father: String,
        script: String,
        source_prefab: String,
        guids: Vec<String>,
    }
    let doc_re = Regex::new(r"^--- !u!(\d+) &(-?\d+)").unwrap();
    let name_re = Regex::new(r"^\s*m_Name:\s*(.+?)\s*$").unwrap();
    let guid_re = Regex::new(r"guid:\s*([0-9a-fA-F]{32})").unwrap();
    let script_re = Regex::new(r"m_Script:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})").unwrap();
    let src_re = Regex::new(r"m_SourcePrefab:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})").unwrap();
    let go_re = Regex::new(r"m_GameObject:\s*\{fileID:\s*(-?\d+)").unwrap();
    let father_re = Regex::new(r"m_Father:\s*\{fileID:\s*(-?\d+)").unwrap();

    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "prefab".into(),
        ..Default::default()
    };
    let file_key = format!("asset:{root}:{rel}");
    let mut file_meta = HashMap::new();
    file_meta.insert("bytes".to_string(), text.len().to_string());
    res.assets.push(asset_node(
        rel,
        root,
        if rel.ends_with(".unity") { "scene" } else { "prefab" },
        &base_name(rel),
        file_meta,
    ));

    let mut docs: Vec<Doc> = Vec::new();
    let mut cur: Option<Doc> = None;
    let mut cur_line = 0i32;
    for (idx, line) in text.lines().enumerate() {
        if let Some(c) = doc_re.captures(line) {
            if let Some(d) = cur.take() {
                docs.push(d);
            }
            cur = Some(Doc {
                class_id: c[1].to_string(),
                file_id: c[2].to_string(),
                ..Default::default()
            });
            cur_line = idx as i32 + 1;
            continue;
        }
        let Some(d) = cur.as_mut() else { continue };
        if let Some(c) = name_re.captures(line) {
            if d.name.is_empty() {
                d.name = c[1].trim().trim_matches('"').to_string();
            }
        }
        if let Some(c) = script_re.captures(line) {
            d.script = c[1].to_string();
        } else if let Some(c) = src_re.captures(line) {
            d.source_prefab = c[1].to_string();
        } else if let Some(c) = guid_re.captures(line) {
            d.guids.push(c[1].to_string());
        }
        if let Some(c) = go_re.captures(line) {
            d.game_obj = c[1].to_string();
        }
        if let Some(c) = father_re.captures(line) {
            d.father = c[1].to_string();
        }
    }
    if let Some(d) = cur.take() {
        docs.push(d);
    }
    let _ = cur_line;

    let mut obj_key: HashMap<String, String> = HashMap::new();
    for d in &docs {
        if d.class_id == "1" {
            let key = format!("{file_key}#{}", d.file_id);
            obj_key.insert(d.file_id.clone(), key.clone());
            res.assets.push(Asset {
                key: key.clone(),
                root: root.to_string(),
                kind: if rel.ends_with(".unity") { "scene_object".into() } else { "prefab_object".into() },
                path: rel.to_string(),
                name: d.name.clone(),
                unity_id: d.file_id.clone(),
                line: 0,
                meta: HashMap::new(),
            });
            res.asset_edges.push(AssetEdge {
                src: file_key.clone(),
                dst: key,
                kind: "contains".into(),
                detail: "gameobject".into(),
                root: root.to_string(),
                line: 0,
            });
        }
    }
    // parent links (Transform.m_Father -> parent GameObject)
    let by_id: HashMap<&str, &Doc> = docs.iter().map(|d| (d.file_id.as_str(), d)).collect();
    for d in &docs {
        if d.father.is_empty() {
            continue;
        }
        let parent_obj = by_id
            .get(d.father.as_str())
            .map(|p| {
                if p.class_id == "1" {
                    p.file_id.clone()
                } else {
                    p.game_obj.clone()
                }
            })
            .unwrap_or_default();
        let child_obj = if d.class_id == "1" { d.file_id.clone() } else { d.game_obj.clone() };
        if let (Some(pk), Some(ck)) = (obj_key.get(&parent_obj), obj_key.get(&child_obj)) {
            if pk != ck {
                res.asset_edges.push(AssetEdge {
                    src: pk.clone(),
                    dst: ck.clone(),
                    kind: "contains".into(),
                    detail: "child".into(),
                    root: root.to_string(),
                    line: 0,
                });
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    for d in &docs {
        let owner = if d.class_id == "1" {
            obj_key.get(&d.file_id).cloned().unwrap_or(file_key.clone())
        } else if !d.game_obj.is_empty() {
            obj_key.get(&d.game_obj).cloned().unwrap_or(file_key.clone())
        } else {
            file_key.clone()
        };
        let mut add = |gid: &str, kind: &str, detail: &str| {
            if gid.is_empty() {
                return;
            }
            let k = format!("{owner}|{gid}|{kind}");
            if !seen.insert(k) {
                return;
            }
            let dst = format!("guid:{gid}");
            res.assets.push(Asset {
                key: dst.clone(),
                root: root.to_string(),
                kind: "guid_ref".into(),
                path: String::new(),
                name: gid.to_string(),
                unity_id: gid.to_string(),
                line: 0,
                meta: HashMap::new(),
            });
            res.asset_edges.push(AssetEdge {
                src: owner.clone(),
                dst,
                kind: kind.into(),
                detail: detail.into(),
                root: root.to_string(),
                line: 0,
            });
        };
        if !d.script.is_empty() {
            add(&d.script, "uses_script", "m_Script");
        }
        if !d.source_prefab.is_empty() {
            add(&d.source_prefab, "instantiates", "m_SourcePrefab");
        } else {
            for g in &d.guids {
                add(g, "references", "guid");
            }
        }
    }
    res
}

// -------------------------------------------------------------------- shader

pub fn parse_shader(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "shader".into(),
        ..Default::default()
    };
    let shader_re = Regex::new(r#"(?m)^\s*Shader\s+"([^"]+)""#).unwrap();
    let prop_re = Regex::new(r#"(?m)^\s*(_?[A-Za-z][A-Za-z0-9_]*)\s*\(\s*"([^"]*)""#).unwrap();
    let inc_re = Regex::new(r#"(?m)^\s*#include\s+"([^"]+)""#).unwrap();
    let pragma_re = Regex::new(r"(?m)^\s*#pragma\s+(?:multi_compile|shader_feature)[^\n]*").unwrap();
    let fn_re = Regex::new(
        r"(?m)^\s*(?:inline\s+)?(?:half|float|fixed|double|void|int|uint|bool|vector|real)[0-9]?(?:x[0-9]+)?\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )
    .unwrap();

    let name = shader_re
        .captures(text)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| base_name(rel));
    let shader_line = shader_re
        .find(text)
        .map(|m| text[..m.start()].matches('\n').count() as i32 + 1)
        .unwrap_or(1);
    let file_key = format!("asset:{root}:{rel}");
    let mut meta = HashMap::new();
    meta.insert("bytes".to_string(), text.len().to_string());
    let mut shader_asset = asset_node(rel, root, "shader", &name, meta);
    shader_asset.line = shader_line;
    res.assets.push(shader_asset);
    let mut props = HashSet::new();
    for c in prop_re.captures_iter(text) {
        let p = c[1].to_string();
        if !props.insert(p.clone()) {
            continue;
        }
        let at = c.get(0).map(|m| m.start()).unwrap_or(0);
        res.assets.push(Asset {
            key: format!("{file_key}#prop:{p}"),
            root: root.to_string(),
            kind: "shader_property".into(),
            path: rel.to_string(),
            name: p,
            unity_id: String::new(),
            line: text[..at].matches('\n').count() as i32 + 1,
            meta: HashMap::new(),
        });
    }
    for c in inc_re.captures_iter(text) {
        res.asset_edges.push(AssetEdge {
            src: file_key.clone(),
            dst: format!("include:{}", &c[1]),
            kind: "includes".into(),
            detail: c[1].to_string(),
            root: root.to_string(),
            line: 0,
        });
    }
    let mut kws: Vec<String> = Vec::new();
    for m in pragma_re.find_iter(text) {
        for w in m.as_str().split_whitespace() {
            if w == "_" || w.starts_with('#') || w.is_empty() {
                continue;
            }
            if w == "multi_compile"
                || w.starts_with("multi_compile_")
                || w == "shader_feature"
                || w.starts_with("shader_feature_")
            {
                continue;
            }
            if !kws.iter().any(|k| k == w) {
                kws.push(w.to_string());
            }
        }
    }
    if let Some(a) = res.assets.first_mut() {
        a.meta.insert("keywords".into(), kws.join(","));
    }
    for c in fn_re.captures_iter(text) {
        let f = c[1].to_string();
        if f == "main" {
            continue;
        }
        let at = c.get(0).map(|m| m.start()).unwrap_or(0);
        let line = text[..at].matches('\n').count() as i32 + 1;
        res.symbols.push(crate::model::Symbol {
            key: crate::model::symbol_key(&format!("{name}.{f}"), "shader_func", line),
            lang: "shader".into(),
            kind: "shader_func".into(),
            name: f.clone(),
            qualname: format!("{name}.{f}"),
            namespace: name.clone(),
            start_line: line,
            end_line: line,
            signature: f,
            modifiers: String::new(),
            param_count: -1,
            container: String::new(),
            module: rel.to_string(),
        });
    }
    res
}

// --------------------------------------------------------------------- media

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

pub fn parse_media(rel: &str, root: &str, abs: &Path) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "asset".into(),
        ..Default::default()
    };
    let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
    let mut meta = HashMap::new();
    meta.insert("bytes".to_string(), size.to_string());
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut head = vec![0u8; 8192];
    let n = fs::File::open(abs)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .unwrap_or(0);
    head.truncate(n);
    match ext.as_str() {
        "png" if head.len() >= 24 => {
            meta.insert("width".into(), be32(&head, 16).to_string());
            meta.insert("height".into(), be32(&head, 20).to_string());
        }
        "jpg" | "jpeg" => {
            if let Some((w, h)) = jpeg_size(&head) {
                meta.insert("width".into(), w.to_string());
                meta.insert("height".into(), h.to_string());
            }
        }
        "wav" if head.len() >= 44 => {
            let sr = le32(&head, 24);
            let ch = u16::from_le_bytes([head[22], head[23]]);
            meta.insert("sample_rate".into(), sr.to_string());
            meta.insert("channels".into(), ch.to_string());
        }
        "mp3" if head.len() > 4 => {
            if let Some(br) = mp3_bitrate(&head) {
                meta.insert("bitrate_kbps".into(), br.to_string());
                meta.insert("duration_sec".into(), format!("{:.2}", size as f64 * 8.0 / (br as f64 * 1000.0)));
            }
        }
        "mp4" | "mov" | "m4v" => {
            if let Some(pos) = find(&head, b"mvhd") {
                if pos + 24 <= head.len() {
                    let ts = be32(&head, pos + 16);
                    let dur = be32(&head, pos + 20);
                    if ts > 0 {
                        meta.insert("duration_sec".into(), format!("{:.2}", dur as f64 / ts as f64));
                    }
                }
            }
        }
        "ttf" | "otf" | "ttc" => {
            if head.len() >= 12 {
                let num = u16::from_be_bytes([head[4], head[5]]) as usize;
                for i in 0..num {
                    let rec = 12 + i * 16;
                    if rec + 16 > head.len() {
                        break;
                    }
                    if &head[rec..rec + 4] == b"maxp" {
                        let off = be32(&head, rec + 8) as usize;
                        if off + 6 <= head.len() {
                            meta.insert("glyphs".into(), u16::from_be_bytes([head[off + 4], head[off + 5]]).to_string());
                        }
                    }
                }
            }
            if let Some(name) = ttf_name(abs) {
                meta.insert("font_name".into(), name);
            }
        }
        "atlas" => {
            if let Ok(text) = fs::read_to_string(abs) {
                let mut pages = 0usize;
                let mut regions = 0usize;
                for line in text.lines() {
                    let t = line.trim();
                    if t.is_empty() || line.starts_with(' ') || line.starts_with('\t') {
                        continue;
                    }
                    if t.ends_with(".png") || t.ends_with(".jpg") || t.ends_with(".webp") {
                        pages += 1;
                    } else if !t.contains(':') {
                        regions += 1;
                    }
                    if let Some(v) = t.strip_prefix("size:") {
                        meta.insert("page_size".into(), v.trim().to_string());
                    }
                }
                meta.insert("pages".into(), pages.to_string());
                meta.insert("regions".into(), regions.to_string());
            }
        }
        "skel" => {
            if let Ok(text) = fs::read_to_string(abs) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(s) = v.get("skeleton").and_then(|x| x.get("spine")) {
                        meta.insert("spine_version".into(), s.to_string().trim_matches('"').to_string());
                    }
                    for (key, label) in [("bones", "bones"), ("slots", "slots"), ("skins", "skins")] {
                        if let Some(a) = v.get(key).and_then(|x| x.as_array()) {
                            meta.insert(label.into(), a.len().to_string());
                        }
                    }
                    if let Some(o) = v.get("animations").and_then(|x| x.as_object()) {
                        meta.insert("animations".into(), o.len().to_string());
                    }
                }
            } else if !head.is_empty() {
                let probe = String::from_utf8_lossy(&head[..head.len().min(128)]);
                if let Some(v) = Regex::new(r"\d+\.\d+\.\d+").ok().and_then(|r| r.find(&probe)) {
                    meta.insert("spine_version".into(), v.as_str().to_string());
                }
            }
        }
        _ => {}
    }
    res.assets.push(asset_node(rel, root, asset_kind(rel), &base_name(rel), meta));
    res
}

fn find(h: &[u8], pat: &[u8]) -> Option<usize> {
    h.windows(pat.len()).position(|w| w == pat)
}

/// Unity YAML assets that are mostly reference tables: materials, animation
/// clips, animator controllers, timelines, mixers, sprite atlases,
/// ScriptableObjects (including TMP font assets) and asmdef JSON.
pub fn parse_structured_asset(rel: &str, root: &str, abs: &Path) -> FileParse {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "asmdef" {
        return parse_asmdef(rel, root, abs);
    }
    let text = match fs::read_to_string(abs) {
        Ok(t) => t,
        Err(_) => return parse_media(rel, root, abs),
    };
    if !text.contains("--- !u!") && !text.starts_with("%YAML") {
        return parse_media(rel, root, abs);
    }

    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "asset".into(),
        ..Default::default()
    };
    let mut kind = asset_kind(rel);
    let mut meta = HashMap::new();
    meta.insert("bytes".to_string(), text.len().to_string());

    // TMP font asset: serialized character/glyph tables and atlas size.
    if text.contains("m_AtlasPopulationMode") || text.contains("TMP_FontAsset") {
        kind = "tmp_font";
        if let Some(pm) = capture_one(r"(?m)^\s*m_AtlasPopulationMode:\s*(-?\d+)", &text) {
            meta.insert("atlas_population_mode".into(), pm);
        }
        if let Some(w) = capture_one(r"(?m)^\s*m_AtlasWidth:\s*(\d+)", &text) {
            meta.insert("atlas_width".into(), w);
        }
        if let Some(h) = capture_one(r"(?m)^\s*m_AtlasHeight:\s*(\d+)", &text) {
            meta.insert("atlas_height".into(), h);
        }
        meta.insert("characters".into(), count_matches(&text, r"(?m)^\s*-?\s*m_Unicode:"));
        meta.insert("glyphs".into(), count_matches(&text, r"(?m)^\s*-?\s*m_Index:\s*\d+"));
        meta.insert(
            "source_font_guid".into(),
            capture_one(
                r"(?m)^\s*m_SourceFontFile_EditorRef:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})",
                &text,
            )
            .unwrap_or_default(),
        );
    } else if kind == "material" {
        meta.insert(
            "shader_guid".into(),
            capture_one(r"(?m)^\s*m_Shader:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})", &text)
                .unwrap_or_default(),
        );
        meta.insert(
            "texture_slots".into(),
            count_matches(&text, r"(?m)^\s{4,6}[A-Za-z_][A-Za-z0-9_.]*:\s*$"),
        );
    } else if kind == "animation_clip" {
        if let Some(v) = capture_one(r"(?m)^\s*m_SampleRate:\s*([0-9.]+)", &text) {
            meta.insert("sample_rate".into(), v);
        }
        if let Some(v) = capture_one(r"(?m)^\s*m_StopTime:\s*([0-9.]+)", &text) {
            meta.insert("duration_sec".into(), v);
        }
        meta.insert("events".into(), count_matches(&text, r"(?m)^\s*m_FunctionName:"));
    } else if kind == "sprite_atlas" {
        meta.insert(
            "packed_sprites".into(),
            count_matches(&text, r"(?m)^\s*m_PackedSprites:"),
        );
    }

    res.assets
        .push(asset_node(rel, root, kind, &base_name(rel), meta));
    let file_key = format!("asset:{root}:{rel}");

    // Every one-line Unity reference (`field: {fileID: N, guid: G, type: T}`).
    let re = Regex::new(
        r#"(?m)^\s*([A-Za-z_][A-Za-z0-9_.]*):\s*\{[^}\n]*guid:\s*([0-9a-fA-F]{32})"#,
    )
    .unwrap();
    let mut seen: HashSet<String> = HashSet::new();
    for c in re.captures_iter(&text) {
        let field = c[1].to_string();
        let guid = c[2].to_ascii_lowercase();
        let (kind, detail) = match field.as_str() {
            "m_Script" => ("uses_script", "m_Script"),
            "m_Shader" => ("uses_shader", "m_Shader"),
            "m_SourceFontFile_EditorRef" | "m_SourceFontFile" => ("uses_font", "m_SourceFontFile"),
            _ => ("references", ""),
        };
        let dedup = format!("{guid}|{kind}|{field}");
        if !seen.insert(dedup) {
            continue;
        }
        let dst = format!("guid:{guid}");
        res.assets.push(Asset {
            key: dst.clone(),
            root: root.to_string(),
            kind: "guid_ref".into(),
            path: String::new(),
            name: guid.clone(),
            unity_id: guid,
            line: 0,
            meta: HashMap::new(),
        });
        res.asset_edges.push(AssetEdge {
            src: file_key.clone(),
            dst,
            kind: kind.into(),
            detail: if detail.is_empty() { field } else { detail.into() },
            root: root.to_string(),
            line: 0,
        });
    }
    res
}

fn parse_asmdef(rel: &str, root: &str, abs: &Path) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "asset".into(),
        ..Default::default()
    };
    let mut meta = HashMap::new();
    let text = fs::read_to_string(abs).unwrap_or_default();
    meta.insert("bytes".into(), text.len().to_string());
    let mut refs: Vec<String> = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
            meta.insert("assembly".into(), name.to_string());
        }
        if let Some(auto) = v.get("autoReferenced").and_then(|x| x.as_bool()) {
            meta.insert("auto_referenced".into(), auto.to_string());
        }
        for key in ["references", "precompiledReferences"] {
            if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
                for r in arr {
                    if let Some(s) = r.as_str() {
                        refs.push(s.to_string());
                    }
                }
            }
        }
        if let Some(arr) = v.get("includePlatforms").and_then(|x| x.as_array()) {
            let mut p: Vec<String> = Vec::new();
            for r in arr {
                if let Some(s) = r.as_str() {
                    p.push(s.to_string());
                }
            }
            meta.insert("platforms".into(), p.join(","));
        }
    }
    meta.insert("references".into(), refs.join(";"));
    res.assets
        .push(asset_node(rel, root, "assembly_def", &base_name(rel), meta));
    res
}

fn capture_one(re: &str, text: &str) -> Option<String> {
    Regex::new(re)
        .ok()
        .and_then(|r| r.captures(text))
        .map(|c| c[1].trim().to_string())
}

fn count_matches(text: &str, re: &str) -> String {
    Regex::new(re)
        .map(|r| r.find_iter(text).count())
        .unwrap_or(0)
        .to_string()
}

/// Read the family name (name table, nameID 4) of a TTF/OTF file without
/// loading the whole font.
fn ttf_name(abs: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(abs).ok()?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).ok()?;
    let num = u16::from_be_bytes([head[4], head[5]]) as usize;
    if num == 0 || num > 512 {
        return None;
    }
    let mut dir = vec![0u8; num * 16];
    f.read_exact(&mut dir).ok()?;
    for i in 0..num {
        let rec = &dir[i * 16..i * 16 + 16];
        if &rec[0..4] != b"name" {
            continue;
        }
        let off = be32(rec, 8) as u64;
        let len = be32(rec, 12) as usize;
        if len == 0 || len > 512 * 1024 {
            return None;
        }
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf).ok()?;
        return parse_name_table(&buf);
    }
    None
}

fn parse_name_table(b: &[u8]) -> Option<String> {
    if b.len() < 6 {
        return None;
    }
    let count = u16::from_be_bytes([b[2], b[3]]) as usize;
    let string_off = u16::from_be_bytes([b[4], b[5]]) as usize;
    for i in 0..count {
        let rec = 6 + i * 12;
        if rec + 12 > b.len() {
            break;
        }
        let platform = u16::from_be_bytes([b[rec], b[rec + 1]]);
        let name_id = u16::from_be_bytes([b[rec + 6], b[rec + 7]]);
        if name_id != 4 {
            continue;
        }
        let len = u16::from_be_bytes([b[rec + 8], b[rec + 9]]) as usize;
        let off = string_off + u16::from_be_bytes([b[rec + 10], b[rec + 11]]) as usize;
        if off + len > b.len() {
            continue;
        }
        let raw = &b[off..off + len];
        let name = if platform == 3 || platform == 0 {
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(raw).to_string()
        };
        let name = name.trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xD8 || m == 0xD9 || (0xD0..=0xD7).contains(&m) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
            let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

fn mp3_bitrate(b: &[u8]) -> Option<u32> {
    let mut i = 0usize;
    if b.len() > 10 && &b[..3] == b"ID3" {
        i = 10 + ((b[6] as usize & 0x7F) << 21 | (b[7] as usize & 0x7F) << 14 | (b[8] as usize & 0x7F) << 7 | (b[9] as usize & 0x7F));
    }
    const BR: [u32; 16] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0];
    while i + 4 < b.len() {
        if b[i] != 0xFF || (b[i + 1] & 0xE0) != 0xE0 {
            i += 1;
            continue;
        }
        let idx = ((b[i + 1] >> 4) & 0x0F) as usize;
        if idx == 0 || idx == 15 {
            i += 1;
            continue;
        }
        let br = BR[idx];
        if br > 0 {
            return Some(br);
        }
        i += 1;
    }
    None
}

// ------------------------------------------------------------------- config

pub fn parse_config(rel: &str, root: &str, abs: &Path) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "config".into(),
        ..Default::default()
    };
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut meta = HashMap::new();
    meta.insert("format".into(), ext.clone());
    meta.insert(
        "bytes".into(),
        fs::metadata(abs).map(|m| m.len()).unwrap_or(0).to_string(),
    );
    match ext.as_str() {
        "csv" | "tsv" => {
            let sep = if ext == "tsv" { '\t' } else { ',' };
            if let Ok(text) = fs::read_to_string(abs) {
                let mut lines = text.lines();
                let header: Vec<&str> = lines.next().unwrap_or("").split(sep).collect();
                let rows = lines.count();
                meta.insert("headers".into(), header.join(","));
                meta.insert("columns".into(), header.len().to_string());
                meta.insert("rows".into(), rows.to_string());
            }
        }
        "json" => {
            if let Ok(text) = fs::read_to_string(abs) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    match &v {
                        serde_json::Value::Array(a) => {
                            meta.insert("rows".into(), a.len().to_string());
                            if let Some(serde_json::Value::Object(o)) = a.first() {
                                let keys: Vec<String> = o.keys().cloned().collect();
                                meta.insert("headers".into(), keys.join(","));
                                meta.insert("columns".into(), keys.len().to_string());
                            }
                        }
                        serde_json::Value::Object(o) => {
                            let keys: Vec<String> = o.keys().cloned().collect();
                            meta.insert("headers".into(), keys.join(","));
                        }
                        _ => {}
                    }
                }
            }
        }
        "xlsx" => {
            if let Ok(mut wb) = calamine::open_workbook_auto(abs) {
                use calamine::Reader;
                for sheet in wb.sheet_names().to_vec() {
                    if let Ok(range) = wb.worksheet_range(&sheet) {
                        let mut headers: Vec<String> = Vec::new();
                        let mut iter = range.rows();
                        if let Some(first) = iter.next() {
                            for cell in first {
                                let s = cell.to_string();
                                if !s.is_empty() && s != "Empty" {
                                    headers.push(s);
                                }
                            }
                        }
                        let row_count = range.rows().count();
                        let key = format!("asset:{root}:{rel}#{sheet}");
                        res.assets.push(Asset {
                            key,
                            root: root.to_string(),
                            kind: "config_table".into(),
                            path: rel.to_string(),
                            name: sheet.clone(),
                            unity_id: String::new(),
                            line: 0,
                            meta: {
                                let mut m = meta.clone();
                                m.insert("sheet".into(), sheet);
                                m.insert("headers".into(), headers.join(","));
                                m.insert("columns".into(), headers.len().to_string());
                                m.insert("rows".into(), row_count.saturating_sub(1).to_string());
                                m
                            },
                        });
                    }
                }
                return res;
            }
        }
        _ => {}
    }
    res.assets.push(asset_node(rel, root, "config_table", &base_name(rel), meta));
    res
}

// --------------------------------------------------------------- guid / meta

pub fn parse_meta(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "asset".into(),
        ..Default::default()
    };
    let re = Regex::new(r"(?m)^guid:\s*([0-9a-fA-F]{32})\s*$").unwrap();
    let Some(c) = re.captures(text) else { return res };
    let guid = c[1].to_ascii_lowercase();
    let target = rel.trim_end_matches(".meta").to_string();
    res.assets.push(Asset {
        key: format!("guid:{guid}"),
        root: root.to_string(),
        kind: "guid_ref".into(),
        path: rel.to_string(),
        name: base_name(&target),
        unity_id: guid.clone(),
        line: 0,
        meta: HashMap::from([
            ("guid".into(), guid.clone()),
            ("linked".into(), "1".into()),
            ("target_path".into(), target.clone()),
        ]),
    });
    res.guid_rows.push(GuidRow {
        guid,
        path: target,
        kind: String::new(),
    });
    res
}

pub fn parse_guid_map_file(path: &Path) -> Result<Vec<GuidRow>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut out = Vec::new();
    if ext == "json" {
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if let Some(arr) = v.as_array() {
            for item in arr {
                let guid = item.get("guid").and_then(|g| g.as_str()).unwrap_or("");
                let p = item.get("path").and_then(|g| g.as_str()).unwrap_or("");
                let ty = item.get("type").and_then(|g| g.as_str()).unwrap_or("");
                if guid.len() == 32 && !p.is_empty() {
                    out.push(GuidRow { guid: guid.to_ascii_lowercase(), path: p.to_string(), kind: ty.to_string() });
                }
            }
        }
        return Ok(out);
    }
    let sep = if ext == "csv" { ',' } else { '\t' };
    for (idx, line) in text.lines().enumerate() {
        let parts: Vec<&str> = line.split(sep).collect();
        if parts.len() < 2 {
            continue;
        }
        let guid = parts[0].trim().to_ascii_lowercase();
        if guid.len() != 32 || (idx == 0 && !guid.chars().all(|c| c.is_ascii_hexdigit())) {
            continue;
        }
        out.push(GuidRow {
            guid,
            path: parts[1].trim().to_string(),
            kind: parts.get(2).map(|s| s.trim().to_string()).unwrap_or_default(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefab_yields_hierarchy_and_script_edges() {
        let text = r#"%YAML 1.1
--- !u!1 &10
GameObject:
  m_Name: Root
--- !u!4 &40
Transform:
  m_GameObject: {fileID: 10}
--- !u!114 &1140
MonoBehaviour:
  m_GameObject: {fileID: 10}
  m_Script: {fileID: 11500000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
--- !u!1 &11
GameObject:
  m_Name: Child
--- !u!4 &41
Transform:
  m_GameObject: {fileID: 11}
  m_Father: {fileID: 40}
"#;
        let res = parse_prefab("Assets/Hero.prefab", "client", text);
        let kinds: Vec<(&str, &str)> = res
            .asset_edges
            .iter()
            .map(|e| (e.kind.as_str(), e.detail.as_str()))
            .collect();
        assert!(kinds.contains(&("contains", "gameobject")), "{kinds:?}");
        assert!(kinds.contains(&("contains", "child")), "nested object edge missing: {kinds:?}");
        assert!(kinds.iter().any(|(k, _)| *k == "uses_script"));
        let parent = res
            .asset_edges
            .iter()
            .find(|e| e.detail == "child")
            .unwrap();
        assert!(parent.src.ends_with("#10"));
        assert!(parent.dst.ends_with("#11"));
        let file = res.assets.iter().find(|a| a.kind == "prefab").unwrap();
        assert_eq!(file.meta.get("bytes").map(String::as_str), Some(text.len().to_string().as_str()));
    }

    #[test]
    fn shader_extracts_properties_keywords_and_functions() {
        let text = r#"Shader "Game/Lit" {
    Properties {
        _MainTex ("Tex", 2D) = "white" {}
        _Tint ("Tint", Color) = (1,1,1,1)
    }
    SubShader {
        Pass {
            CGPROGRAM
            #pragma multi_compile _ LOW HIGH
            #include "UnityCG.cginc"
            float4 tinted(float4 c) { return c; }
            fixed4 frag() : SV_Target { return 0; }
            ENDCG
        }
    }
}
"#;
        let res = parse_shader("Assets/Lit.shader", "client", text);
        let props: Vec<&str> = res
            .assets
            .iter()
            .filter(|a| a.kind == "shader_property")
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(props.len(), 2);
        assert!(props.contains(&"_MainTex"));
        let shader = res.assets.iter().find(|a| a.kind == "shader").unwrap();
        assert_eq!(shader.name, "Game/Lit");
        assert!(shader.meta["keywords"].contains("LOW"));
        assert!(res.symbols.iter().any(|s| s.name == "tinted" && s.start_line > 1));
        assert!(res
            .asset_edges
            .iter()
            .any(|e| e.kind == "includes" && e.dst == "include:UnityCG.cginc"));
    }

    #[test]
    fn csv_config_reports_headers_and_rows() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("multi")
            .join("config")
            .join("monsters.csv");
        let res = parse_config("config/monsters.csv", "client", &path);
        let table = res.assets.iter().find(|a| a.kind == "config_table").unwrap();
        assert_eq!(table.meta["headers"], "id,name,attack,hp,quality");
        assert_eq!(table.meta["rows"], "3");
        assert_eq!(table.meta["columns"], "5");
        assert_eq!(table.meta["format"], "csv");
    }
}

#[cfg(test)]
mod structured_tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cg233-asset-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn material_links_shader_and_textures() {
        let p = tmp("Hero.mat");
        fs::write(
            &p,
            r#"%YAML 1.1
--- !u!21 &2100000
Material:
  m_Name: Hero
  m_Shader: {fileID: 4800000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
  m_SavedProperties:
    m_TexEnvs:
    - _MainTex:
        m_Texture: {fileID: 2800000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}
"#,
        )
        .unwrap();
        let res = parse_structured_asset("Assets/Hero.mat", "client", &p);
        let mat = res.assets.iter().find(|a| a.kind == "material").unwrap();
        assert_eq!(mat.meta["shader_guid"], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(res
            .asset_edges
            .iter()
            .any(|e| e.kind == "uses_shader" && e.dst.ends_with("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")));
        assert!(res
            .asset_edges
            .iter()
            .any(|e| e.kind == "references" && e.dst.ends_with("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")));
        let _ = fs::remove_file(p);
    }

    #[test]
    fn tmp_font_reports_atlas_and_tables() {
        let p = tmp("Font.asset");
        fs::write(
            &p,
            r#"%YAML 1.1
--- !u!114 &11400000
MonoBehaviour:
  m_Name: MainFont
  m_AtlasPopulationMode: 1
  m_AtlasWidth: 1024
  m_AtlasHeight: 512
  m_SourceFontFile_EditorRef: {fileID: 12800000, guid: cccccccccccccccccccccccccccccccc, type: 3}
  m_CharacterTable:
  - m_Unicode: 32
  - m_Unicode: 65
  m_GlyphTable:
  - m_Index: 0
  - m_Index: 1
  - m_Index: 2
"#,
        )
        .unwrap();
        let res = parse_structured_asset("Assets/Font.asset", "client", &p);
        let f = res.assets.iter().find(|a| a.kind == "tmp_font").unwrap();
        assert_eq!(f.meta["atlas_width"], "1024");
        assert_eq!(f.meta["characters"], "2");
        assert_eq!(f.meta["glyphs"], "3");
        assert_eq!(f.meta["source_font_guid"], "cccccccccccccccccccccccccccccccc");
        assert!(res.asset_edges.iter().any(|e| e.kind == "uses_font"));
        let _ = fs::remove_file(p);
    }

    #[test]
    fn asmdef_reports_assembly_and_references() {
        let p = tmp("Game.asmdef");
        fs::write(
            &p,
            r#"{"name":"Game.Runtime","references":["UnityEngine.UI","Game.Shared"],"includePlatforms":["Editor"],"autoReferenced":false}"#,
        )
        .unwrap();
        let res = parse_structured_asset("Assets/Game.asmdef", "client", &p);
        let a = res.assets.iter().find(|a| a.kind == "assembly_def").unwrap();
        assert_eq!(a.meta["assembly"], "Game.Runtime");
        assert_eq!(a.meta["references"], "UnityEngine.UI;Game.Shared");
        assert_eq!(a.meta["platforms"], "Editor");
        assert_eq!(a.meta["auto_referenced"], "false");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn spine_atlas_counts_pages_and_regions() {
        let p = tmp("hero.atlas");
        fs::write(
            &p,
            "hero.png\nsize: 1024,1024\nformat: RGBA8888\nfilter: Linear,Linear\nrepeat: none\nidle\n  rotate: false\nwalk\n  rotate: false\n",
        )
        .unwrap();
        let res = parse_media("Assets/hero.atlas", "client", &p);
        let a = res.assets.iter().find(|x| x.kind == "spine_atlas").unwrap();
        assert_eq!(a.meta["pages"], "1");
        assert_eq!(a.meta["regions"], "2");
        assert_eq!(a.meta["page_size"], "1024,1024");
        let _ = fs::remove_file(p);
    }
}
