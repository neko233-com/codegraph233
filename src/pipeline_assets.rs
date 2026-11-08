//! Art and audio pipeline frontends: Unity AudioMixer, Wwise work units and
//! banks, FMOD projects/banks, glTF/GLB, Aseprite, FBX headers and Unity
//! Package Manager manifests.
//!
//! These are inventory/dependency parsers, not importers: they expose the
//! object graph and metadata needed for navigation, not decoded payloads.

use crate::model::{Asset, AssetEdge, FileParse};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn base_name(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn ext_of(rel: &str) -> String {
    Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn new_res(rel: &str, root: &str, lang: &str) -> FileParse {
    FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        ..Default::default()
    }
}

fn push_file(res: &mut FileParse, rel: &str, root: &str, kind: &str, name: &str, meta: HashMap<String, String>) -> String {
    let key = file_key(root, rel);
    res.assets.push(Asset {
        key: key.clone(),
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 0,
        meta,
    });
    key
}

fn push_child(
    res: &mut FileParse,
    rel: &str,
    root: &str,
    kind: &str,
    name: &str,
    meta: HashMap<String, String>,
) -> String {
    let key = format!("asset:{root}:{rel}#{kind}:{name}");
    res.assets.push(Asset {
        key: key.clone(),
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 0,
        meta,
    });
    let src = file_key(root, rel);
    res.asset_edges.push(AssetEdge {
        src,
        dst: key.clone(),
        kind: "contains".into(),
        detail: kind.to_string(),
        root: root.to_string(),
        line: 0,
    });
    key
}

fn edge(res: &mut FileParse, root: &str, src: &str, dst: &str, kind: &str, detail: &str) {
    res.asset_edges.push(AssetEdge {
        src: src.to_string(),
        dst: dst.to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
        root: root.to_string(),
        line: 0,
    });
}

fn add_meta(meta: &mut HashMap<String, String>, key: &str, value: impl ToString) {
    meta.insert(key.to_string(), value.to_string());
}

fn err_res(rel: &str, root: &str, lang: &str, message: &str) -> FileParse {
    let mut res = new_res(rel, root, lang);
    res.errors.push(message.to_string());
    res
}

/// Resolve a relative URI found inside `rel` to a workspace-relative path.
fn resolve_uri(rel: &str, uri: &str) -> Option<String> {
    if uri.is_empty() || uri.starts_with("data:") || uri.contains("://") || uri.starts_with('/') {
        return None;
    }
    let uri = uri.replace("%20", " ");
    let parent = Path::new(rel).parent()?;
    let joined = parent.join(uri);
    let mut out: Vec<String> = Vec::new();
    for part in joined.components() {
        match part.as_os_str().to_string_lossy().as_ref() {
            "." | "" => {}
            ".." => {
                out.pop();
            }
            other => out.push(other.replace('\\', "/")),
        }
    }
    Some(out.join("/"))
}

// ------------------------------------------------------------------ Unity mixer

pub fn parse_mixer(rel: &str, root: &str, text: &str) -> FileParse {
    struct Doc {
        class: String,
        name: String,
        parent: i64,
        effect: String,
    }
    let doc_re = Regex::new(r"(?m)^--- !u!(\d+) &(-?\d+)").unwrap();
    let name_re = Regex::new(r"(?m)^\s*m_Name:\s*(.+?)\s*$").unwrap();
    let parent_re = Regex::new(r"m_ParentGroup:\s*\{[^}\n]*fileID:\s*(-?\d+)").unwrap();
    let effect_re = Regex::new(r"(?m)^\s*m_EffectName:\s*(.+?)\s*$").unwrap();

    let mut res = new_res(rel, root, "unity_mixer");
    let positions: Vec<(usize, usize)> = doc_re
        .captures_iter(text)
        .map(|c| (c.get(0).unwrap().start(), c.get(0).unwrap().end()))
        .collect();
    let mut docs: Vec<Doc> = Vec::new();
    for (i, (_, end)) in positions.iter().enumerate() {
        let stop = positions.get(i + 1).map(|p| p.0).unwrap_or(text.len());
        let body = &text[*end..stop];
        let class = body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
            .trim_end_matches(':')
            .to_string();
        let name = name_re
            .captures(body)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let parent = parent_re
            .captures(body)
            .and_then(|c| c[1].parse::<i64>().ok())
            .unwrap_or(0);
        let effect = effect_re
            .captures(body)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        docs.push(Doc {
            class,
            name,
            parent,
            effect,
        });
    }

    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", text.len());
    add_meta(&mut meta, "documents", docs.len());
    push_file(&mut res, rel, root, "audio_mixer", &base_name(rel), meta);

    let mut id_to_key: HashMap<i64, String> = HashMap::new();
    let mut id_by_key: HashMap<String, i64> = HashMap::new();
    let mut ids: Vec<i64> = Vec::new();
    for c in doc_re.captures_iter(text) {
        ids.push(c[2].parse::<i64>().unwrap_or(0));
    }

    let mut groups = 0;
    let mut snapshots = 0;
    let mut effects = 0;
    for (i, doc) in docs.iter().enumerate() {
        let id = ids.get(i).copied().unwrap_or(0);
        let (kind, name) = match doc.class.as_str() {
            "AudioMixerGroupController" => {
                groups += 1;
                ("audio_mixer_group", doc.name.clone())
            }
            "AudioMixerSnapshotController" => {
                snapshots += 1;
                ("audio_mixer_snapshot", doc.name.clone())
            }
            "AudioMixerEffectController" => {
                effects += 1;
                let name = if doc.effect.is_empty() {
                    doc.name.clone()
                } else {
                    format!("{} ({})", doc.name, doc.effect)
                };
                ("audio_mixer_effect", name)
            }
            "AudioMixerController" => {
                // The mixer root is the file-level asset, not a child document.
                id_to_key.insert(id, file_key(root, rel));
                continue;
            }
            _ => continue,
        };
        let mut m = HashMap::new();
        add_meta(&mut m, "class", &doc.class);
        if !doc.effect.is_empty() {
            add_meta(&mut m, "effect", &doc.effect);
        }
        let key = push_child(&mut res, rel, root, kind, &name, m);
        id_to_key.insert(id, key.clone());
        id_by_key.insert(key, doc.parent);
    }
    // Parent routing edges (group -> child group).
    for (key, parent) in &id_by_key {
        if *parent != 0 {
            if let Some(parent_key) = id_to_key.get(parent) {
                edge(&mut res, root, key, parent_key, "routes_to", "m_ParentGroup");
            }
        }
    }
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "groups", groups);
        add_meta(&mut a.meta, "snapshots", snapshots);
        add_meta(&mut a.meta, "effects", effects);
    }
    res
}

// ------------------------------------------------------------------------ Wwise

pub fn parse_wwise_workunit(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = new_res(rel, root, "wwise_workunit");
    let obj_re = Regex::new(
        r#"<(Event|SwitchGroup|StateGroup|GameParameter|SoundBank|AuxBus|AudioDevice|Effect|ExternalSource)\b[^>]*\bName="([^"]+)""#,
    )
    .unwrap();
    let wem_re = Regex::new(r#"(?i)([A-Za-z0-9_\-./\\]+\.wem)"#).unwrap();
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", text.len());
    push_file(&mut res, rel, root, "wwise_workunit", &base_name(rel), meta);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for c in obj_re.captures_iter(text) {
        let tag = c.get(1).unwrap().as_str();
        let name = c[2].to_string();
        let kind = match tag {
            "Event" => "wwise_event",
            "SwitchGroup" => "wwise_switch_group",
            "StateGroup" => "wwise_state_group",
            "GameParameter" => "wwise_rtpc",
            "SoundBank" => "wwise_bank",
            "AuxBus" => "wwise_aux_bus",
            "AudioDevice" => "wwise_audio_device",
            "Effect" => "wwise_effect",
            "ExternalSource" => "wwise_external_source",
            _ => "wwise_object",
        };
        *counts.entry(kind).or_insert(0) += 1;
        let mut m = HashMap::new();
        add_meta(&mut m, "tag", tag);
        push_child(&mut res, rel, root, kind, &name, m);
    }
    let file = file_key(root, rel);
    let mut seen: HashMap<String, bool> = HashMap::new();
    for c in wem_re.captures_iter(text) {
        let name = c[1].trim_start_matches(['/', '\\']).replace('\\', "/");
        if seen.insert(name.clone(), true).is_some() {
            continue;
        }
        let dst = format!("wwise_media:{name}");
        edge(&mut res, root, &file, &dst, "references", "wem");
    }
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "objects", counts.values().sum::<usize>());
        for (k, v) in counts {
            add_meta(&mut a.meta, k, v);
        }
    }
    res
}

pub fn parse_wwise_project(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = new_res(rel, root, "wwise_project");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", text.len());
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        for key in ["name", "version", "platforms", "rootPaths", "soundbanksPath"] {
            if let Some(x) = v.get(key) {
                let s = x.to_string();
                add_meta(&mut meta, key, s.trim_matches('"'));
            }
        }
    } else {
        res.errors.push("Wwise project JSON is invalid".into());
    }
    push_file(&mut res, rel, root, "wwise_project", &base_name(rel), meta);
    res
}

fn u32le(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn u16le(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([data[at], data[at + 1]])
}

pub fn parse_wwise_bank(rel: &str, root: &str, data: &[u8]) -> FileParse {
    if data.len() < 12 || &data[0..4] != b"BKHD" {
        return err_res(rel, root, "wwise_bank", "not a Wwise soundbank (missing BKHD chunk)");
    }
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    add_meta(&mut meta, "chunk_size", u32le(data, 4));
    add_meta(&mut meta, "bank_version", u32le(data, 8));
    if data.len() >= 16 {
        add_meta(&mut meta, "bank_id", u32le(data, 12));
    }
    if data.len() >= 20 {
        add_meta(&mut meta, "language_id", u32le(data, 16));
    }
    let mut res = new_res(rel, root, "wwise_bank");
    push_file(&mut res, rel, root, "wwise_bank", &base_name(rel), meta);
    res
}

pub fn parse_wem(rel: &str, root: &str, data: &[u8]) -> FileParse {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return err_res(rel, root, "wwise_media", "not a RIFF/WAVE media file");
    }
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32le(data, pos + 4) as usize;
        if id == b"fmt " && pos + 24 <= data.len() {
            let codec = u16le(data, pos + 8);
            let channels = u16le(data, pos + 10);
            let sample_rate = u32le(data, pos + 12);
            let byte_rate = u32le(data, pos + 16);
            add_meta(&mut meta, "codec", codec_name(codec));
            add_meta(&mut meta, "channels", channels);
            add_meta(&mut meta, "sample_rate", sample_rate);
            if byte_rate > 0 {
                if let Some(data_pos) = find_chunk(data, b"data") {
                    let data_size = u32le(data, data_pos + 4) as usize;
                    add_meta(&mut meta, "duration_sec", format!("{:.2}", data_size as f64 / byte_rate as f64));
                }
            }
        }
        pos += 8 + size + (size & 1);
    }
    let mut res = new_res(rel, root, "wwise_media");
    push_file(&mut res, rel, root, "audio", &base_name(rel), meta);
    res
}

fn find_chunk(data: &[u8], id: &[u8; 4]) -> Option<usize> {
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let size = u32le(data, pos + 4) as usize;
        if &data[pos..pos + 4] == id {
            return Some(pos);
        }
        pos += 8 + size + (size & 1);
    }
    None
}

fn codec_name(codec: u16) -> &'static str {
    match codec {
        0x0001 => "pcm",
        0x0003 => "ieee_float",
        0x0006 => "alaw",
        0x0007 => "mulaw",
        0x0016 => "g723",
        0xFFFF => "vorbis",
        0xFFFE => "extensible",
        _ => "unknown",
    }
}

// ------------------------------------------------------------------------- FMOD

pub fn parse_fmod_project(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = new_res(rel, root, "fmod_project");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", text.len());
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return err_res(rel, root, "fmod_project", "FMOD Studio project JSON is invalid"),
    };
    push_file(&mut res, rel, root, "fmod_project", &base_name(rel), meta);
    let mut objects: Vec<(String, String)> = Vec::new();
    if let Some(o) = v.as_object() {
        for (key, category) in [
            ("events", "fmod_event"),
            ("buses", "fmod_bus"),
            ("vcas", "fmod_vca"),
            ("banks", "fmod_bank"),
        ] {
            if let Some(arr) = o.get(key).and_then(|x| x.as_array()) {
                for item in arr {
                    fmod_walk(item, category, &mut objects);
                }
            }
        }
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (kind, name) in &objects {
        *counts.entry(kind.clone()).or_insert(0) += 1;
        let mut m = HashMap::new();
        add_meta(&mut m, "category", kind);
        push_child(&mut res, rel, root, kind, name, m);
    }
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "objects", objects.len());
        for (k, v) in counts {
            add_meta(&mut a.meta, &k, v);
        }
    }
    res
}

fn fmod_walk(v: &Value, category: &str, out: &mut Vec<(String, String)>) {
    if let Some(o) = v.as_object() {
        if let Some(name) = o.get("name").and_then(|x| x.as_str()) {
            if !name.is_empty() {
                out.push((category.to_string(), name.to_string()));
            }
        }
        for key in ["children", "events"] {
            if let Some(arr) = o.get(key).and_then(|x| x.as_array()) {
                let child_category = if key == "events" { "fmod_event" } else { category };
                for item in arr {
                    fmod_walk(item, child_category, out);
                }
            }
        }
    }
}

pub fn parse_fmod_strings(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = new_res(rel, root, "fmod_strings");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    push_file(&mut res, rel, root, "fmod_strings", &base_name(rel), meta);
    let mut count = 0usize;
    let mut seen: HashMap<String, bool> = HashMap::new();
    for part in data.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        let name = String::from_utf8_lossy(part).trim().trim_matches('\u{0}').to_string();
        if name.len() < 3 || !name.contains('/') {
            continue;
        }
        if seen.insert(name.clone(), true).is_some() {
            continue;
        }
        count += 1;
        let mut m = HashMap::new();
        add_meta(&mut m, "path", &name);
        push_child(&mut res, rel, root, "fmod_event", &name, m);
        if count >= 5000 {
            break;
        }
    }
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "events", count);
    }
    res
}

pub fn parse_fmod_bank(rel: &str, root: &str, data: &[u8]) -> FileParse {
    if data.len() < 28 || &data[0..4] != b"FSB5" {
        return err_res(rel, root, "fmod_bank", "not an FSB5 bank (missing FSB5 magic)");
    }
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    add_meta(&mut meta, "fsb_version", u32le(data, 4));
    add_meta(&mut meta, "samples", u32le(data, 8));
    add_meta(&mut meta, "sample_headers_size", u32le(data, 12));
    add_meta(&mut meta, "name_table_size", u32le(data, 16));
    add_meta(&mut meta, "data_size", u32le(data, 20));
    add_meta(&mut meta, "mode", u32le(data, 24));
    let mut res = new_res(rel, root, "fmod_bank");
    push_file(&mut res, rel, root, "fmod_bank", &base_name(rel), meta);
    res
}

// ------------------------------------------------------------------------- glTF

pub fn parse_gltf(rel: &str, root: &str, text: &str) -> FileParse {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return err_res(rel, root, "gltf", &format!("glTF JSON is invalid: {e}")),
    };
    gltf_core(rel, root, &v, "gltf_scene", text.len(), "gltf")
}

pub fn parse_glb(rel: &str, root: &str, data: &[u8]) -> FileParse {
    if data.len() < 12 || &data[0..4] != b"glTF" {
        return err_res(rel, root, "gltf", "not a GLB container (missing glTF magic)");
    }
    let version = u32le(data, 4);
    if version != 2 {
        return err_res(rel, root, "gltf", &format!("unsupported GLB version {version}"));
    }
    let mut pos = 12usize;
    let mut json: Option<&[u8]> = None;
    while pos + 8 <= data.len() {
        let len = u32le(data, pos) as usize;
        let kind = u32le(data, pos + 4);
        let start = pos + 8;
        let end = (start + len).min(data.len());
        if kind == 0x4E4F_534A {
            json = Some(&data[start..end]);
            break;
        }
        pos = start + len;
    }
    let Some(mut chunk) = json else {
        return err_res(rel, root, "gltf", "GLB has no JSON chunk");
    };
    while chunk.last() == Some(&b' ') || chunk.last() == Some(&0) {
        chunk = &chunk[..chunk.len() - 1];
    }
    let v: Value = match serde_json::from_slice(chunk) {
        Ok(v) => v,
        Err(e) => return err_res(rel, root, "gltf", &format!("GLB JSON chunk is invalid: {e}")),
    };
    gltf_core(rel, root, &v, "glb_scene", data.len(), "glb")
}

fn gltf_core(rel: &str, root: &str, v: &Value, kind: &str, bytes: usize, format: &str) -> FileParse {
    let mut res = new_res(rel, root, "gltf");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", bytes);
    add_meta(&mut meta, "format", format);
    if let Some(a) = v.get("asset") {
        if let Some(x) = a.get("version") {
            add_meta(&mut meta, "version", x.to_string().trim_matches('"'));
        }
        if let Some(x) = a.get("generator").and_then(|x| x.as_str()) {
            add_meta(&mut meta, "generator", x);
        }
    }
    let arr = |key: &str| v.get(key).and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let meshes = arr("meshes");
    let materials = arr("materials");
    let textures = arr("textures");
    let images = arr("images");
    let animations = arr("animations");
    let skins = arr("skins");
    let nodes = arr("nodes");
    add_meta(&mut meta, "meshes", meshes.len());
    add_meta(&mut meta, "materials", materials.len());
    add_meta(&mut meta, "textures", textures.len());
    add_meta(&mut meta, "images", images.len());
    add_meta(&mut meta, "animations", animations.len());
    add_meta(&mut meta, "skins", skins.len());
    add_meta(&mut meta, "nodes", nodes.len());
    push_file(&mut res, rel, root, kind, &base_name(rel), meta);
    let file = file_key(root, rel);

    let names = |list: &[Value], prefix: &str| -> Vec<String> {
        list.iter()
            .enumerate()
            .map(|(i, x)| {
                x.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| format!("{prefix}{i}"))
            })
            .collect()
    };
    let mesh_names = names(&meshes, "mesh");
    let material_names = names(&materials, "material");
    let texture_names = names(&textures, "texture");
    let image_names = names(&images, "image");
    let animation_names = names(&animations, "animation");
    let skin_names = names(&skins, "skin");
    let node_names = names(&nodes, "node");

    let mut mesh_keys: Vec<String> = Vec::new();
    for (i, m) in meshes.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(prims) = m.get("primitives").and_then(|x| x.as_array()) {
            add_meta(&mut mm, "primitives", prims.len());
        }
        mesh_keys.push(push_child(&mut res, rel, root, "gltf_mesh", &mesh_names[i], mm));
    }
    let mut material_keys: Vec<String> = Vec::new();
    for (i, m) in materials.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(x) = m.get("alphaMode") {
            add_meta(&mut mm, "alpha_mode", x.to_string().trim_matches('"'));
        }
        material_keys.push(push_child(&mut res, rel, root, "gltf_material", &material_names[i], mm));
    }
    let mut texture_keys: Vec<String> = Vec::new();
    for (i, t) in textures.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(src) = t.get("source").and_then(|x| x.as_u64()) {
            add_meta(&mut mm, "source", src);
        }
        texture_keys.push(push_child(&mut res, rel, root, "gltf_texture", &texture_names[i], mm));
    }
    let mut image_keys: Vec<String> = Vec::new();
    for (i, image) in images.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(uri) = image.get("uri").and_then(|x| x.as_str()) {
            add_meta(&mut mm, "uri", uri);
            if let Some(target) = resolve_uri(rel, uri) {
                let dst = format!("asset:{root}:{target}");
                edge(&mut res, root, &file, &dst, "references", "image_uri");
            }
        }
        image_keys.push(push_child(&mut res, rel, root, "gltf_image", &image_names[i], mm));
    }
    let mut animation_keys: Vec<String> = Vec::new();
    for (i, a) in animations.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(ch) = a.get("channels").and_then(|x| x.as_array()) {
            add_meta(&mut mm, "channels", ch.len());
        }
        animation_keys.push(push_child(&mut res, rel, root, "gltf_animation", &animation_names[i], mm));
    }
    let mut skin_keys: Vec<String> = Vec::new();
    for (i, s) in skins.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(j) = s.get("joints").and_then(|x| x.as_array()) {
            add_meta(&mut mm, "joints", j.len());
        }
        skin_keys.push(push_child(&mut res, rel, root, "gltf_skin", &skin_names[i], mm));
    }
    let mut node_keys: Vec<String> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let mut mm = HashMap::new();
        if let Some(m) = n.get("mesh").and_then(|x| x.as_u64()) {
            add_meta(&mut mm, "mesh", m);
        }
        if let Some(c) = n.get("children").and_then(|x| x.as_array()) {
            add_meta(&mut mm, "children", c.len());
        }
        node_keys.push(push_child(&mut res, rel, root, "gltf_node", &node_names[i], mm));
    }

    // mesh primitive -> material
    for (i, m) in meshes.iter().enumerate() {
        if let Some(prims) = m.get("primitives").and_then(|x| x.as_array()) {
            for p in prims {
                if let Some(mi) = p.get("material").and_then(|x| x.as_u64()) {
                    if let Some(dst) = material_keys.get(mi as usize) {
                        edge(&mut res, root, &mesh_keys[i], dst, "uses_material", "primitive");
                    }
                }
            }
        }
    }
    // material -> texture
    for (i, m) in materials.iter().enumerate() {
        let mut refs: Vec<u64> = Vec::new();
        if let Some(o) = m.as_object() {
            for (_, val) in o {
                collect_texture_indices(val, &mut refs);
            }
        }
        refs.sort_unstable();
        refs.dedup();
        for ti in refs {
            if let Some(dst) = texture_keys.get(ti as usize) {
                edge(&mut res, root, &material_keys[i], dst, "uses_texture", "material");
            }
        }
    }
    // texture -> image
    for (i, t) in textures.iter().enumerate() {
        if let Some(src) = t.get("source").and_then(|x| x.as_u64()) {
            if let Some(dst) = image_keys.get(src as usize) {
                edge(&mut res, root, &texture_keys[i], dst, "uses_image", "source");
            }
        }
    }
    // node -> mesh/skin/children
    for (i, n) in nodes.iter().enumerate() {
        if let Some(m) = n.get("mesh").and_then(|x| x.as_u64()) {
            if let Some(dst) = mesh_keys.get(m as usize) {
                edge(&mut res, root, &node_keys[i], dst, "uses_mesh", "node");
            }
        }
        if let Some(s) = n.get("skin").and_then(|x| x.as_u64()) {
            if let Some(dst) = skin_keys.get(s as usize) {
                edge(&mut res, root, &node_keys[i], dst, "uses_skin", "node");
            }
        }
        if let Some(ch) = n.get("children").and_then(|x| x.as_array()) {
            for c in ch {
                if let Some(ci) = c.as_u64() {
                    if let Some(dst) = node_keys.get(ci as usize) {
                        edge(&mut res, root, &node_keys[i], dst, "contains_node", "children");
                    }
                }
            }
        }
    }
    // animation channel targets
    for (i, a) in animations.iter().enumerate() {
        if let Some(channels) = a.get("channels").and_then(|x| x.as_array()) {
            for c in channels {
                if let Some(ni) = c
                    .get("target")
                    .and_then(|x| x.get("node"))
                    .and_then(|x| x.as_u64())
                {
                    if let Some(dst) = node_keys.get(ni as usize) {
                        edge(&mut res, root, &animation_keys[i], dst, "targets_node", "channel");
                    }
                }
            }
        }
    }
    // external buffers
    if let Some(buffers) = v.get("buffers").and_then(|x| x.as_array()) {
        for b in buffers {
            if let Some(uri) = b.get("uri").and_then(|x| x.as_str()) {
                if let Some(target) = resolve_uri(rel, uri) {
                    let dst = format!("asset:{root}:{target}");
                    edge(&mut res, root, &file, &dst, "references", "buffer_uri");
                }
            }
        }
    }
    res
}

fn collect_texture_indices(v: &Value, out: &mut Vec<u64>) {
    match v {
        Value::Object(o) => {
            for (k, val) in o {
                if k.ends_with("Texture") {
                    if let Some(i) = val.get("index").and_then(|x| x.as_u64()) {
                        out.push(i);
                    }
                }
                collect_texture_indices(val, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_texture_indices(x, out);
            }
        }
        _ => {}
    }
}

// --------------------------------------------------------------------- Aseprite

pub fn parse_aseprite(rel: &str, root: &str, data: &[u8]) -> FileParse {
    if data.len() < 128 {
        return err_res(rel, root, "aseprite", "Aseprite header is truncated");
    }
    if u16le(data, 4) != 0xA5E0 {
        return err_res(rel, root, "aseprite", "not an Aseprite file (bad 0xA5E0 magic)");
    }
    let frames = u16le(data, 6);
    let width = u16le(data, 8);
    let height = u16le(data, 10);
    let depth = u16le(data, 12);
    let mut res = new_res(rel, root, "aseprite");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    add_meta(&mut meta, "frames", frames);
    add_meta(&mut meta, "width", width);
    add_meta(&mut meta, "height", height);
    add_meta(&mut meta, "color_depth", depth);
    push_file(&mut res, rel, root, "aseprite", &base_name(rel), meta);

    let mut layers: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut pos = 128usize;
    for _ in 0..frames {
        if pos + 16 > data.len() {
            res.errors.push("Aseprite frame header is truncated".into());
            break;
        }
        let frame_size = u32le(data, pos) as usize;
        let magic = u16le(data, pos + 4);
        if magic != 0xF1FA {
            res.errors.push(format!("Aseprite frame magic mismatch at {pos}"));
            break;
        }
        let old_chunks = u16le(data, pos + 6) as usize;
        let new_chunks = if old_chunks == 0xFFFF {
            0
        } else {
            old_chunks
        };
        let chunk_count = if pos + 20 <= data.len() {
            u32le(data, pos + 12) as usize
        } else {
            new_chunks
        };
        let mut cpos = pos + 16;
        for _ in 0..chunk_count {
            if cpos + 6 > data.len() {
                break;
            }
            let chunk_size = u32le(data, cpos) as usize;
            let chunk_type = u16le(data, cpos + 4);
            let payload = cpos + 6;
            if payload + chunk_size.saturating_sub(6) > data.len() {
                break;
            }
            match chunk_type {
                0x2004 if payload + 18 <= data.len() => {
                    let name_len = u16le(data, payload + 16) as usize;
                    if payload + 18 + name_len <= data.len() {
                        let name = String::from_utf8_lossy(&data[payload + 18..payload + 18 + name_len]).to_string();
                        if !name.is_empty() {
                            layers.push(name);
                        }
                    }
                }
                0x2018 if payload + 2 <= data.len() => {
                    let count = u16le(data, payload) as usize;
                    let mut tpos = payload + 2;
                    for _ in 0..count {
                        if tpos + 19 > data.len() {
                            break;
                        }
                        let name_len = u16le(data, tpos + 17) as usize;
                        if tpos + 19 + name_len <= data.len() {
                            let name = String::from_utf8_lossy(&data[tpos + 19..tpos + 19 + name_len]).to_string();
                            if !name.is_empty() {
                                tags.push(name);
                            }
                            tpos += 19 + name_len;
                        } else {
                            break;
                        }
                    }
                }
                _ => {}
            }
            cpos += chunk_size;
        }
        pos += frame_size;
    }
    let mut seen = HashMap::new();
    for name in layers {
        if seen.insert(format!("layer:{name}"), true).is_some() {
            continue;
        }
        let mut m = HashMap::new();
        add_meta(&mut m, "role", "layer");
        push_child(&mut res, rel, root, "aseprite_layer", &name, m);
    }
    for name in tags {
        if seen.insert(format!("tag:{name}"), true).is_some() {
            continue;
        }
        let mut m = HashMap::new();
        add_meta(&mut m, "role", "tag");
        push_child(&mut res, rel, root, "aseprite_tag", &name, m);
    }
    let layers = count_kind(&res.assets, "aseprite_layer");
    let tags = count_kind(&res.assets, "aseprite_tag");
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "layers", layers);
        add_meta(&mut a.meta, "tags", tags);
    }
    res
}

fn count_kind(assets: &[Asset], kind: &str) -> usize {
    assets.iter().filter(|a| a.kind == kind).count()
}

// ------------------------------------------------------------------------- FBX

pub fn parse_fbx(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", data.len());
    let mut res = new_res(rel, root, "fbx");
    if data.starts_with(b"Kaydara FBX Binary  \x00") {
        if data.len() < 27 {
            return err_res(rel, root, "fbx", "FBX binary header is truncated");
        }
        add_meta(&mut meta, "format", "binary");
        add_meta(&mut meta, "version", u32le(data, 23));
    } else if data.starts_with(b"; FBX") {
        let text = String::from_utf8_lossy(&data[..data.len().min(512)]);
        let first = text.lines().next().unwrap_or("");
        if let Some(v) = Regex::new(r"FBX ([0-9.]+)").ok().and_then(|r| r.captures(first)) {
            add_meta(&mut meta, "version", &v[1]);
        }
        add_meta(&mut meta, "format", "ascii");
    } else {
        return err_res(rel, root, "fbx", "not an FBX file (missing Kaydara header or ASCII banner)");
    }
    push_file(&mut res, rel, root, "fbx", &base_name(rel), meta);
    res
}

// ------------------------------------------------------------------ UPM manifest

pub fn parse_upm(rel: &str, root: &str, text: &str, lock: bool) -> FileParse {
    let mut res = new_res(rel, root, "upm_manifest");
    let mut meta = HashMap::new();
    add_meta(&mut meta, "bytes", text.len());
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return err_res(rel, root, "upm_manifest", &format!("package manifest JSON is invalid: {e}")),
    };
    push_file(
        &mut res,
        rel,
        root,
        if lock { "upm_lock" } else { "upm_manifest" },
        &base_name(rel),
        meta,
    );
    let deps = v.get("dependencies").and_then(|x| x.as_object()).cloned().unwrap_or_default();
    for (name, info) in deps {
        let mut m = HashMap::new();
        if lock {
            if let Some(x) = info.get("version") {
                add_meta(&mut m, "version", x.to_string().trim_matches('"'));
            }
            if let Some(x) = info.get("source") {
                add_meta(&mut m, "source", x.to_string().trim_matches('"'));
            }
            if let Some(x) = info.get("depth") {
                add_meta(&mut m, "depth", x);
            }
        } else {
            add_meta(&mut m, "version", info.to_string().trim_matches('"'));
        }
        push_child(&mut res, rel, root, "upm_package", &name, m);
    }
    let packages = count_kind(&res.assets, "upm_package");
    if let Some(a) = res.assets.first_mut() {
        add_meta(&mut a.meta, "packages", packages);
    }
    res
}

// ------------------------------------------------------------------- dispatch

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    let ext = ext_of(rel);
    match lang {
        "unity_mixer" => parse_mixer(rel, root, &String::from_utf8_lossy(data)),
        "wwise_workunit" => parse_wwise_workunit(rel, root, &String::from_utf8_lossy(data)),
        "wwise_project" => parse_wwise_project(rel, root, &String::from_utf8_lossy(data)),
        "wwise_bank" => parse_wwise_bank(rel, root, data),
        "wwise_media" => parse_wem(rel, root, data),
        "fmod_project" => parse_fmod_project(rel, root, &String::from_utf8_lossy(data)),
        "fmod_strings" => parse_fmod_strings(rel, root, data),
        "fmod_bank" => parse_fmod_bank(rel, root, data),
        "gltf" => {
            if ext == "glb" {
                parse_glb(rel, root, data)
            } else {
                parse_gltf(rel, root, &String::from_utf8_lossy(data))
            }
        }
        "aseprite" => parse_aseprite(rel, root, data),
        "fbx" => parse_fbx(rel, root, data),
        "upm_manifest" => {
            let lock = rel.to_ascii_lowercase().ends_with("packages-lock.json");
            parse_upm(rel, root, &String::from_utf8_lossy(data), lock)
        }
        _ => err_res(rel, root, lang, "no pipeline frontend for this language"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_asset(fp: &FileParse, kind: &str) -> bool {
        fp.assets.iter().any(|a| a.kind == kind)
    }

    fn has_edge(fp: &FileParse, kind: &str) -> bool {
        fp.asset_edges.iter().any(|e| e.kind == kind)
    }

    #[test]
    fn unity_mixer_extracts_groups_snapshots_and_effects() {
        let text = r#"%YAML 1.1
--- !u!243 &-123
AudioMixerController:
  m_Name: MainMixer
--- !u!243 &1
AudioMixerGroupController:
  m_Name: Master
  m_ParentGroup: {fileID: 0}
--- !u!243 &2
AudioMixerGroupController:
  m_Name: SFX
  m_ParentGroup: {fileID: 1}
--- !u!245 &3
AudioMixerSnapshotController:
  m_Name: Quiet
--- !u!242 &4
AudioMixerEffectController:
  m_Name: LowPass
  m_EffectName: AudioLowPassFilter
"#;
        let fp = parse_mixer("Assets/Main.mixer", "app", text);
        assert!(fp.errors.is_empty());
        assert!(has_asset(&fp, "audio_mixer_group"));
        assert!(has_asset(&fp, "audio_mixer_snapshot"));
        assert!(has_asset(&fp, "audio_mixer_effect"));
        assert!(has_edge(&fp, "routes_to"));
    }

    #[test]
    fn wwise_workunit_and_bank_extract_objects() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<WwiseDocument Version="0">
 <Event Name="Play_Hit"><Action Name="Play" Type="Play"><ReferenceFile Name="..\GeneratedSoundBanks\Windows\hits.wem"/></Action></Event>
 <SwitchGroup Name="Surface"><Switch Name="Stone"/></SwitchGroup>
 <SoundBank Name="Main"><IncludedEvents><Event Name="Play_Hit"/></IncludedEvents></SoundBank>
</WwiseDocument>"#;
        let fp = parse_wwise_workunit("Wwise/Events.wwu", "app", xml);
        assert!(fp.errors.is_empty());
        assert!(has_asset(&fp, "wwise_event"));
        assert!(has_asset(&fp, "wwise_switch_group"));
        assert!(has_asset(&fp, "wwise_bank"));
        assert!(has_edge(&fp, "references"));

        let mut b = Vec::new();
        b.extend_from_slice(b"BKHD");
        b.extend_from_slice(&12u32.to_le_bytes());
        b.extend_from_slice(&0x8Au32.to_le_bytes());
        b.extend_from_slice(&7u32.to_le_bytes());
        let fp = parse_wwise_bank("Wwise/Main.bnk", "app", &b);
        assert!(fp.errors.is_empty());
        assert_eq!(fp.assets[0].meta.get("bank_id").map(String::as_str), Some("7"));
    }

    #[test]
    fn fmod_project_strings_and_fsb5_are_indexed() {
        let json = r#"{"events":[{"name":"Ambience","children":[{"name":"Play_Wind"}]}],"buses":[{"name":"Master"}],"vcas":[{"name":"Music"}],"banks":[{"name":"Desktop"}]}"#;
        let fp = parse_fmod_project("FMOD/Game.fspro", "app", json);
        assert!(has_asset(&fp, "fmod_event"));
        assert!(has_asset(&fp, "fmod_bus"));
        assert!(has_asset(&fp, "fmod_vca"));
        assert!(has_asset(&fp, "fmod_bank"));

        let strings = b"event:/Ambience\n\0event:/SFX/Hit\0\0";
        let fp = parse_fmod_strings("FMOD/Desktop.strings", "app", strings);
        assert_eq!(count_kind(&fp.assets, "fmod_event"), 2);

        let mut bank = Vec::new();
        bank.extend_from_slice(b"FSB5");
        bank.extend_from_slice(&1u32.to_le_bytes());
        bank.extend_from_slice(&4u32.to_le_bytes());
        bank.extend_from_slice(&0u32.to_le_bytes());
        bank.extend_from_slice(&0u32.to_le_bytes());
        bank.extend_from_slice(&64u32.to_le_bytes());
        bank.extend_from_slice(&0u32.to_le_bytes());
        let fp = parse_fmod_bank("FMOD/Desktop.bank", "app", &bank);
        assert!(fp.errors.is_empty());
        assert_eq!(fp.assets[0].meta.get("samples").map(String::as_str), Some("4"));
    }

    #[test]
    fn gltf_extracts_meshes_materials_and_external_images() {
        let json = r#"{
          "asset": {"version": "2.0", "generator": "Blender"},
          "scenes": [{"nodes": [0]}],
          "nodes": [{"name": "Root", "mesh": 0, "children": [1]}, {"name": "Child"}],
          "meshes": [{"name": "Cube", "primitives": [{"material": 0}]}],
          "materials": [{"name": "Glow", "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
          "textures": [{"source": 0}],
          "images": [{"name": "GlowTex", "uri": "tex/glow.png"}],
          "animations": [{"name": "Idle", "channels": [{"target": {"node": 0}}]}],
          "buffers": [{"uri": "data/cube.bin"}]
        }"#;
        let fp = parse_gltf("Art/Hero.gltf", "app", json);
        assert!(fp.errors.is_empty());
        assert!(has_asset(&fp, "gltf_mesh"));
        assert!(has_asset(&fp, "gltf_material"));
        assert!(has_asset(&fp, "gltf_texture"));
        assert!(has_asset(&fp, "gltf_image"));
        assert!(has_asset(&fp, "gltf_animation"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "uses_material"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "uses_texture"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "references" && e.dst.ends_with("tex/glow.png")));
    }

    #[test]
    fn glb_json_chunk_is_parsed() {
        let json = br#"{"asset":{"version":"2.0"},"meshes":[{"name":"Cube"}],"materials":[{"name":"Glow"}]}"#;
        let mut chunk = json.to_vec();
        while !chunk.len().is_multiple_of(4) {
            chunk.push(b' ');
        }
        let total = 12 + 8 + chunk.len();
        let mut data = Vec::new();
        data.extend_from_slice(b"glTF");
        data.extend_from_slice(&2u32.to_le_bytes());
        data.extend_from_slice(&(total as u32).to_le_bytes());
        data.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        data.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        data.extend_from_slice(&chunk);
        let fp = parse_glb("Art/Hero.glb", "app", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "glb_scene"));
        assert!(has_asset(&fp, "gltf_mesh"));
    }

    #[test]
    fn aseprite_extracts_layers_and_tags() {
        let mut d = vec![0u8; 128];
        d[4..6].copy_from_slice(&0xA5E0u16.to_le_bytes());
        d[6..8].copy_from_slice(&1u16.to_le_bytes());
        d[8..10].copy_from_slice(&32u16.to_le_bytes());
        d[10..12].copy_from_slice(&32u16.to_le_bytes());
        d[12..14].copy_from_slice(&32u16.to_le_bytes());
        let layer_name = b"Hero";
        let mut layer = Vec::new();
        layer.extend_from_slice(&0u16.to_le_bytes());
        layer.extend_from_slice(&0u16.to_le_bytes());
        layer.extend_from_slice(&0u16.to_le_bytes());
        layer.extend_from_slice(&32u16.to_le_bytes());
        layer.extend_from_slice(&32u16.to_le_bytes());
        layer.extend_from_slice(&0u16.to_le_bytes());
        layer.push(255);
        layer.extend_from_slice(&[0, 0, 0]);
        layer.extend_from_slice(&(layer_name.len() as u16).to_le_bytes());
        layer.extend_from_slice(layer_name);
        let mut layer_chunk = Vec::new();
        layer_chunk.extend_from_slice(&((6 + layer.len()) as u32).to_le_bytes());
        layer_chunk.extend_from_slice(&0x2004u16.to_le_bytes());
        layer_chunk.extend_from_slice(&layer);

        let tag_name = b"Idle";
        let mut tag = Vec::new();
        tag.extend_from_slice(&1u16.to_le_bytes());
        tag.extend_from_slice(&0u16.to_le_bytes());
        tag.extend_from_slice(&0u16.to_le_bytes());
        tag.push(0);
        tag.extend_from_slice(&0u16.to_le_bytes());
        tag.extend_from_slice(&[0u8; 6]);
        tag.extend_from_slice(&[0, 0, 0]);
        tag.push(0);
        tag.extend_from_slice(&(tag_name.len() as u16).to_le_bytes());
        tag.extend_from_slice(tag_name);
        let mut tag_chunk = Vec::new();
        tag_chunk.extend_from_slice(&((6 + tag.len()) as u32).to_le_bytes());
        tag_chunk.extend_from_slice(&0x2018u16.to_le_bytes());
        tag_chunk.extend_from_slice(&tag);

        let chunks = [layer_chunk, tag_chunk].concat();
        let frame_size = 16 + chunks.len();
        let mut frame = Vec::new();
        frame.extend_from_slice(&(frame_size as u32).to_le_bytes());
        frame.extend_from_slice(&0xF1FAu16.to_le_bytes());
        frame.extend_from_slice(&2u16.to_le_bytes());
        frame.extend_from_slice(&100u16.to_le_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&2u32.to_le_bytes());
        frame.extend_from_slice(&chunks);
        d.extend_from_slice(&frame);
        let fp = parse_aseprite("Art/Hero.ase", "app", &d);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "aseprite_layer"));
        assert!(has_asset(&fp, "aseprite_tag"));
    }

    #[test]
    fn fbx_header_and_upm_manifest_are_parsed() {
        let mut fbx = Vec::new();
        fbx.extend_from_slice(b"Kaydara FBX Binary  \x00");
        fbx.extend_from_slice(&[0, 0]);
        fbx.extend_from_slice(&7400u32.to_le_bytes());
        let fp = parse_fbx("Art/Hero.fbx", "app", &fbx);
        assert!(fp.errors.is_empty());
        assert_eq!(fp.assets[0].meta.get("version").map(String::as_str), Some("7400"));

        let manifest = r#"{"dependencies":{"com.unity.render-pipelines.universal":"14.0.11","com.unity.inputsystem":"1.7.0"}}"#;
        let fp = parse_upm("Packages/manifest.json", "app", manifest, false);
        assert!(fp.errors.is_empty());
        assert_eq!(count_kind(&fp.assets, "upm_package"), 2);

        let lock = r#"{"dependencies":{"com.unity.inputsystem":{"version":"1.7.0","source":"registry","depth":0}}}"#;
        let fp = parse_upm("Packages/packages-lock.json", "app", lock, true);
        assert!(has_asset(&fp, "upm_lock"));
        assert_eq!(count_kind(&fp.assets, "upm_package"), 1);
    }
}
