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

pub fn is_asset_ext(rel: &str) -> bool {
    let ext = Path::new(rel)
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
        }
        _ => {}
    }
    res.assets.push(asset_node(rel, root, asset_kind(rel), &base_name(rel), meta));
    res
}

fn find(h: &[u8], pat: &[u8]) -> Option<usize> {
    h.windows(pat.len()).position(|w| w == pat)
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
