//! Design-document, narrative and media-sidecar frontends.
//!
//! These inputs sit upstream of code and assets: Markdown design docs carrying
//! stable requirement ids and `@code` / `@asset` / `@audio` / `@video` /
//! `@art` / `@story` / `@value` markers, Yarn Spinner and Ink scripts, subtitle
//! sidecars and WebM/Matroska headers. They are indexed as bounded static
//! navigation data, never executed.

use std::collections::{HashMap, HashSet};

use crate::model::{Asset, AssetEdge, FileParse};

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn child_key(root: &str, rel: &str, kind: &str, name: &str) -> String {
    format!("{}#{kind}:{name}", file_key(root, rel))
}

fn push_asset(
    res: &mut FileParse,
    root: &str,
    key: String,
    kind: &str,
    path: &str,
    name: &str,
    line: i32,
    meta: HashMap<String, String>,
) {
    if res.assets.iter().any(|a| a.key == key) {
        return;
    }
    res.assets.push(Asset {
        key,
        root: root.to_string(),
        kind: kind.to_string(),
        path: path.to_string(),
        name: name.to_string(),
        line,
        meta,
        ..Default::default()
    });
}

#[allow(clippy::too_many_arguments)]
fn add_edge(
    res: &mut FileParse,
    root: &str,
    src: &str,
    dst: &str,
    kind: &str,
    detail: &str,
    line: i32,
) {
    if src.is_empty() || dst.is_empty() || src == dst {
        return;
    }
    res.asset_edges.push(AssetEdge {
        src: src.to_string(),
        dst: dst.to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
        root: root.to_string(),
        line,
    });
}

fn local_name(rel: &str) -> &str {
    rel.rsplit(['/', '\\']).next().unwrap_or(rel)
}

fn stem(rel: &str) -> String {
    let name = local_name(rel);
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    let text = String::from_utf8_lossy(data);
    match lang {
        "design_doc" => parse_design_doc(rel, root, &text),
        "yarn_script" => parse_yarn(rel, root, &text),
        "ink_script" => parse_ink(rel, root, &text),
        "subtitle" => parse_subtitle(rel, root, &text),
        "video" => parse_video(rel, root, data),
        other => FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: other.to_string(),
            errors: vec![format!("unsupported design frontend {other}")],
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------- markdown

fn is_design_ref(key: &str) -> bool {
    matches!(
        key,
        "code" | "asset" | "audio" | "video" | "art" | "story" | "value" | "text" | "link"
    )
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn marker(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start().strip_prefix('@')?;
    let colon = rest.find(':')?;
    let key = rest[..colon].trim();
    if !key
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    Some((key, rest[colon + 1..].trim()))
}

fn split_target(value: &str) -> (&str, &str) {
    match value.find('#') {
        Some(i) => (value[..i].trim(), value[i + 1..].trim()),
        None => (value.trim(), ""),
    }
}

fn parse_design_doc(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "design_doc".into(),
        ..Default::default()
    };
    let doc = file_key(root, rel);
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());

    let lines: Vec<&str> = text.lines().collect();
    let mut first = 0;
    let mut front = HashMap::new();
    if lines.first().map(|l| l.trim() == "---").unwrap_or(false) {
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            for line in &lines[1..=end] {
                if let Some((k, v)) = line.split_once(':') {
                    front.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                }
            }
            first = end + 2;
        }
    }
    let mut title = front.get("title").cloned().unwrap_or_default();
    if title.is_empty() {
        title = lines[first..]
            .iter()
            .find_map(|l| l.trim().strip_prefix("# ").map(|t| t.trim().to_string()))
            .unwrap_or_else(|| stem(rel));
    }
    let doc_id = front.get("id").cloned().unwrap_or_default();
    if !doc_id.is_empty() {
        meta.insert("doc_id".into(), doc_id.clone());
    }

    let mut requirements = 0usize;
    let mut sections = 0usize;
    let mut refs = 0usize;
    let mut doc_links = 0usize;
    let mut links = HashSet::new();
    let mut requirement_keys = HashSet::new();
    let mut src = doc.clone();

    for (i, line) in lines.iter().enumerate().skip(first) {
        let lineno = i as i32 + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            if level <= 6 && trimmed.chars().nth(level) == Some(' ') {
                let heading = trimmed[level..].trim();
                if !heading.is_empty() {
                    let slug = slugify(heading);
                    let key = child_key(root, rel, "design_section", &slug);
                    let mut m = HashMap::new();
                    m.insert("level".into(), level.to_string());
                    push_asset(&mut res, root, key.clone(), "design_section", rel, heading, lineno, m);
                    add_edge(&mut res, root, &doc, &key, "contains", "design_section", lineno);
                    sections += 1;
                }
            }
            continue;
        }
        if let Some((key, value)) = marker(line) {
            if value.is_empty() {
                continue;
            }
            match key {
                "id" => {
                    let id = value.split_whitespace().next().unwrap_or("");
                    if valid_id(id) && requirement_keys.insert(id.to_string()) {
                        let req = format!("design:{root}:{id}");
                        let mut m = HashMap::new();
                        m.insert("source".into(), rel.to_string());
                        push_asset(&mut res, root, req.clone(), "design_requirement", rel, id, lineno, m);
                        add_edge(&mut res, root, &doc, &req, "contains", "design_requirement", lineno);
                        src = req;
                        requirements += 1;
                    }
                }
                _ if is_design_ref(key) => {
                    let (path, fragment) = split_target(value.trim_matches('"'));
                    if path.is_empty() {
                        continue;
                    }
                    let dst = if key == "link" {
                        format!("design:{root}:{path}")
                    } else {
                        format!("asset:{root}:{path}")
                    };
                    let kind = if key == "link" {
                        "design_link".to_string()
                    } else {
                        format!("design_{key}")
                    };
                    add_edge(&mut res, root, &src, &dst, &kind, fragment, lineno);
                    refs += 1;
                    links.insert(path.to_string());
                }
                _ => {}
            }
        }
        // Wiki-style links connect requirements without a path.
        let mut rest: &str = line;
        while let Some(open) = rest.find("[[") {
            let after = &rest[open + 2..];
            let Some(close) = after.find("]]") else { break };
            let id = after[..close].trim();
            if valid_id(id) {
                add_edge(
                    &mut res,
                    root,
                    &src,
                    &format!("design:{root}:{id}"),
                    "design_link",
                    "wiki",
                    lineno,
                );
                links.insert(id.to_string());
            }
            rest = &after[close + 2..];
        }
        // Normal Markdown links become doc → file relationships, so design
        // folders, art references and spec cross-links are navigable too.
        let mut rest: &str = line;
        while let Some(pos) = rest.find("](") {
            let after = &rest[pos + 2..];
            let Some(close) = after.find(')') else { break };
            let target = after[..close].trim();
            if let Some(dst_rel) = resolve_doc_target(rel, target) {
                add_edge(
                    &mut res,
                    root,
                    &doc,
                    &format!("asset:{root}:{dst_rel}"),
                    "doc_link",
                    target,
                    lineno,
                );
                doc_links += 1;
            }
            rest = &after[close + 1..];
        }
    }

    meta.insert("title".into(), title.clone());
    meta.insert("sections".into(), sections.to_string());
    meta.insert("requirements".into(), requirements.to_string());
    meta.insert("references".into(), refs.to_string());
    meta.insert("doc_links".into(), doc_links.to_string());
    meta.insert("links".into(), links.len().to_string());
    push_asset(&mut res, root, doc.clone(), "design_doc", rel, &title, 1, meta);

    if requirements == 0 {
        // A doc without explicit ids is still a searchable section tree.
        res.assets
            .retain(|a| a.key == doc || (a.kind != "design_requirement"));
    }
    res
}

fn resolve_doc_target(rel: &str, raw: &str) -> Option<String> {
    let target = raw.split(['#', '?']).next()?.trim();
    if target.is_empty() {
        return None;
    }
    let lower = target.to_ascii_lowercase();
    for prefix in ["http://", "https://", "mailto:", "data:", "javascript:"] {
        if lower.starts_with(prefix) {
            return None;
        }
    }
    if lower.starts_with('#') {
        return None;
    }
    let ext = lower.rsplit('.').next()?;
    if ext == lower || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_string()
    } else {
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if dir.is_empty() {
            target.to_string()
        } else {
            format!("{dir}/{target}")
        }
    };
    normalize_rel(&joined)
}

fn normalize_rel(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("/"))
    }
}

// -------------------------------------------------------------------- yarn

fn parse_yarn(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "yarn_script".into(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    let mut nodes = 0usize;
    let mut tags = 0usize;
    let mut options = 0usize;
    let mut jumps = 0usize;
    let mut current = file.clone();
    let mut seen_tags = HashSet::new();

    for (i, line) in text.lines().enumerate() {
        let lineno = i as i32 + 1;
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("title:") {
            let name = rest.trim();
            if !name.is_empty() {
                let key = child_key(root, rel, "yarn_node", &slugify(name));
                let mut m = HashMap::new();
                m.insert("title".into(), name.to_string());
                push_asset(&mut res, root, key.clone(), "yarn_node", rel, name, lineno, m);
                add_edge(&mut res, root, &file, &key, "contains", "yarn_node", lineno);
                current = key;
                nodes += 1;
            }
        } else if let Some(rest) = trimmed.strip_prefix("->") {
            let text_val = rest.trim();
            if !text_val.is_empty() {
                let key = child_key(root, rel, "yarn_option", &slugify(text_val));
                push_asset(
                    &mut res,
                    root,
                    key.clone(),
                    "yarn_option",
                    rel,
                    text_val,
                    lineno,
                    HashMap::new(),
                );
                add_edge(&mut res, root, &current, &key, "contains", "yarn_option", lineno);
                options += 1;
            }
        } else if let Some(cmd) = trimmed.strip_prefix("<<") {
            if let Some(rest) = cmd.strip_prefix("jump ") {
                let target = rest.trim_end_matches(">>").trim();
                if !target.is_empty() {
                    add_edge(
                        &mut res,
                        root,
                        &current,
                        &child_key(root, rel, "yarn_node", &slugify(target)),
                        "jumps_to",
                        target,
                        lineno,
                    );
                    jumps += 1;
                }
            }
        } else {
            for (pos, _) in line.match_indices('#') {
                if pos > 0 && !line[..pos].ends_with(char::is_whitespace) {
                    continue;
                }
                let tag: String = line[pos + 1..]
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                if tag.is_empty() || !seen_tags.insert((current.clone(), tag.clone())) {
                    continue;
                }
                let key = child_key(root, rel, "yarn_tag", &slugify(&tag));
                push_asset(
                    &mut res,
                    root,
                    key.clone(),
                    "yarn_tag",
                    rel,
                    &tag,
                    lineno,
                    HashMap::new(),
                );
                add_edge(&mut res, root, &current, &key, "contains", "yarn_tag", lineno);
                tags += 1;
            }
        }
    }

    meta.insert("nodes".into(), nodes.to_string());
    meta.insert("tags".into(), tags.to_string());
    meta.insert("options".into(), options.to_string());
    meta.insert("jumps".into(), jumps.to_string());
    push_asset(
        &mut res,
        root,
        file,
        "yarn_script",
        rel,
        &stem(rel),
        1,
        meta,
    );
    res
}

// --------------------------------------------------------------------- ink

fn parse_ink(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "ink_script".into(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    let mut knots = 0usize;
    let mut stitches = 0usize;
    let mut tags = 0usize;
    let mut choices = 0usize;
    let mut refs = 0usize;
    let mut current = file.clone();
    let mut seen_tags = HashSet::new();

    for (i, line) in text.lines().enumerate() {
        let lineno = i as i32 + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("===") && trimmed.ends_with("===") && trimmed.len() > 6 {
            let name = trimmed.trim_matches('=').trim();
            if !name.is_empty() {
                let key = child_key(root, rel, "ink_knot", &slugify(name));
                push_asset(
                    &mut res,
                    root,
                    key.clone(),
                    "ink_knot",
                    rel,
                    name,
                    lineno,
                    HashMap::new(),
                );
                add_edge(&mut res, root, &file, &key, "contains", "ink_knot", lineno);
                current = key;
                knots += 1;
            }
        } else if trimmed.starts_with('=') && !trimmed.starts_with("==") {
            let name = trimmed[1..].trim();
            if !name.is_empty() {
                let key = child_key(root, rel, "ink_stitch", &slugify(name));
                push_asset(
                    &mut res,
                    root,
                    key.clone(),
                    "ink_stitch",
                    rel,
                    name,
                    lineno,
                    HashMap::new(),
                );
                add_edge(&mut res, root, &current, &key, "contains", "ink_stitch", lineno);
                current = key;
                stitches += 1;
            }
        } else if trimmed.starts_with('#') {
            let tag = trimmed.trim_start_matches('#').trim();
            if !tag.is_empty() && seen_tags.insert((file.clone(), tag.to_string())) {
                let key = child_key(root, rel, "ink_tag", &slugify(tag));
                push_asset(
                    &mut res,
                    root,
                    key.clone(),
                    "ink_tag",
                    rel,
                    tag,
                    lineno,
                    HashMap::new(),
                );
                add_edge(&mut res, root, &file, &key, "contains", "ink_tag", lineno);
                tags += 1;
            }
        } else if trimmed.starts_with('*') || trimmed.starts_with('+') {
            choices += 1;
        } else if let Some(target) = trimmed.strip_prefix("->") {
            let name = target.trim();
            if !name.is_empty() {
                add_edge(
                    &mut res,
                    root,
                    &current,
                    &child_key(root, rel, "ink_knot", &slugify(name)),
                    "jumps_to",
                    name,
                    lineno,
                );
                refs += 1;
            }
        }
    }

    meta.insert("knots".into(), knots.to_string());
    meta.insert("stitches".into(), stitches.to_string());
    meta.insert("tags".into(), tags.to_string());
    meta.insert("choices".into(), choices.to_string());
    meta.insert("jumps".into(), refs.to_string());
    push_asset(
        &mut res,
        root,
        file,
        "ink_script",
        rel,
        &stem(rel),
        1,
        meta,
    );
    res
}

// ---------------------------------------------------------------- subtitles

fn timestamp_ms(text: &str) -> Option<i64> {
    let t = text.trim().replace(',', ".");
    let parts: Vec<&str> = t.split(':').collect();
    let (h, m, s) = match parts.len() {
        3 => (parts[0].parse::<i64>().ok()?, parts[1].parse::<i64>().ok()?, parts[2]),
        2 => (0, parts[0].parse::<i64>().ok()?, parts[1]),
        _ => return None,
    };
    let secs: f64 = s.parse().ok()?;
    Some(h * 3_600_000 + m * 60_000 + (secs * 1000.0) as i64)
}

fn parse_subtitle(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "subtitle".into(),
        ..Default::default()
    };
    let mut cues = 0usize;
    let mut first = i64::MAX;
    let mut last = 0i64;
    let mut first_text = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.contains("-->") {
            continue;
        }
        let mut times = line.split("-->");
        let start = times.next().and_then(timestamp_ms);
        let end = times.next().and_then(timestamp_ms);
        if let (Some(s), Some(e)) = (start, end) {
            cues += 1;
            first = first.min(s);
            last = last.max(e);
            let caption = lines
                .peek()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && !l.contains("-->"))
                .unwrap_or_default();
            if first_text.is_empty() {
                first_text = caption;
            }
        }
    }
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), text.len().to_string());
    meta.insert("cues".into(), cues.to_string());
    if cues > 0 && last >= first {
        meta.insert("duration_sec".into(), format!("{:.2}", (last - first) as f64 / 1000.0));
    }
    if !first_text.is_empty() {
        meta.insert("first_text".into(), first_text);
    }
    push_asset(
        &mut res,
        root,
        file_key(root, rel),
        "subtitle",
        rel,
        &stem(rel),
        1,
        meta,
    );
    res
}

// ------------------------------------------------------------------- video

fn parse_video(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "video".into(),
        ..Default::default()
    };
    let mut meta = HashMap::new();
    meta.insert("bytes".into(), data.len().to_string());
    if data.len() < 8 || data[0..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        res.errors
            .push("not a WebM/Matroska container (missing EBML magic)".into());
        return res;
    }
    let probe = &data[..data.len().min(256)];
    let container = if probe.windows(4).any(|w| w == b"webm") {
        "webm"
    } else if probe.windows(8).any(|w| w == b"matroska") {
        "matroska"
    } else {
        "ebml"
    };
    meta.insert("container".into(), container.into());
    push_asset(
        &mut res,
        root,
        file_key(root, rel),
        "video",
        rel,
        &stem(rel),
        1,
        meta,
    );
    res
}

// ---------------------------------------------------- code-side annotations

/// Scan source text for `@design:` / `@spec:` / `@doc:` annotations and emit
/// `implements` edges from the source file to the design requirement.
pub fn code_markers(rel: &str, root: &str, text: &str) -> Vec<AssetEdge> {
    let mut out = Vec::new();
    let src = file_key(root, rel);
    for (marker, kind) in [("@design:", "design"), ("@spec:", "spec"), ("@doc:", "doc")] {
        let mut from = 0usize;
        while let Some(at) = text[from..].find(marker) {
            let pos = from + at;
            let id_start = pos + marker.len();
            let id: String = text[id_start..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .collect();
            if !id.is_empty() && id.len() <= 128 {
                let line = text[..pos].bytes().filter(|b| *b == b'\n').count() as i32 + 1;
                let edge = AssetEdge {
                    src: src.clone(),
                    dst: format!("design:{root}:{id}"),
                    kind: "implements".into(),
                    detail: format!("{kind}:{id}"),
                    root: root.to_string(),
                    line,
                };
                if !out
                    .iter()
                    .any(|e: &AssetEdge| e.dst == edge.dst && e.line == edge.line)
                {
                    out.push(edge);
                }
            }
            from = id_start;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_asset(fp: &FileParse, kind: &str) -> bool {
        fp.assets.iter().any(|a| a.kind == kind)
    }

    fn has_edge(fp: &FileParse, kind: &str, dst_suffix: &str) -> bool {
        fp.asset_edges
            .iter()
            .any(|e| e.kind == kind && e.dst.ends_with(dst_suffix))
    }

    #[test]
    fn design_doc_extracts_requirements_sections_and_links() {
        let text = "---\ntitle: Combat\nid: DOC-COMBAT\n---\n# Combat\n@id: REQ-COMBAT\n@code: src/combat.rs#damage\n@asset: Assets/Enemy.prefab\n@audio: Audio/hit.wav\n@video: Media/trailer.webm\n@story: Story/quests.yarn#intro\n@value: Tables/combat.csv#Damage\n@link: REQ-STATS\nSee [[REQ-UI]] for the HUD.\n";
        let fp = parse_design_doc("docs/Combat.md", "app", text);
        assert!(fp.errors.is_empty());
        assert_eq!(fp.assets.iter().filter(|a| a.kind == "design_section").count(), 1);
        assert!(has_asset(&fp, "design_requirement"));
        assert!(has_edge(&fp, "design_code", "asset:app:src/combat.rs"));
        assert!(has_edge(&fp, "design_audio", "asset:app:Audio/hit.wav"));
        assert!(has_edge(&fp, "design_video", "asset:app:Media/trailer.webm"));
        assert!(has_edge(&fp, "design_link", "design:app:REQ-STATS"));
        assert!(has_edge(&fp, "design_link", "design:app:REQ-UI"));
        let doc = fp.assets.iter().find(|a| a.kind == "design_doc").unwrap();
        assert_eq!(doc.name, "Combat");
        assert_eq!(doc.meta.get("requirements").map(String::as_str), Some("1"));
    }

    #[test]
    fn yarn_nodes_capture_tags_options_and_jumps() {
        let text = "title: Start\n---\nHello #greeting\n-> Go left\n-> Go right\n<<jump Ending>>\n===\ntitle: Ending\n---\nBye\n";
        let fp = parse_yarn("Story/quest.yarn", "app", text);
        assert_eq!(fp.assets.iter().filter(|a| a.kind == "yarn_node").count(), 2);
        assert!(has_asset(&fp, "yarn_tag"));
        assert_eq!(fp.assets.iter().filter(|a| a.kind == "yarn_option").count(), 2);
        assert!(has_edge(&fp, "jumps_to", "yarn_node:ending"));
    }

    #[test]
    fn ink_knots_stitches_and_choices_are_indexed() {
        let text = "=== start ===\n= intro\n* choice one\n* choice two\n-> ending\n# intro_tag\n=== ending ===\nEnd\n";
        let fp = parse_ink("Story/branch.ink", "app", text);
        assert_eq!(fp.assets.iter().filter(|a| a.kind == "ink_knot").count(), 2);
        assert!(has_asset(&fp, "ink_stitch"));
        assert!(has_asset(&fp, "ink_tag"));
        assert!(has_edge(&fp, "jumps_to", "ink_knot:ending"));
    }

    #[test]
    fn subtitle_counts_cues_and_duration() {
        let text = "1\n00:00:01,000 --> 00:00:03,500\nHello\n\n2\n00:00:04,000 --> 00:00:06,000\nBye\n";
        let fp = parse_subtitle("Media/cutscene.srt", "app", text);
        assert!(has_asset(&fp, "subtitle"));
        let a = &fp.assets[0];
        assert_eq!(a.meta.get("cues").map(String::as_str), Some("2"));
        assert_eq!(a.meta.get("duration_sec").map(String::as_str), Some("5.00"));
    }

    #[test]
    fn webm_header_is_recognized_and_bad_magic_is_visible() {
        let mut data = vec![0x1A, 0x45, 0xDF, 0xA3];
        data.extend_from_slice(b"webm");
        let fp = parse_video("Media/trailer.webm", "app", &data);
        assert!(fp.errors.is_empty());
        assert!(has_asset(&fp, "video"));
        assert_eq!(
            fp.assets[0].meta.get("container").map(String::as_str),
            Some("webm")
        );
        let bad = parse_video("Media/bad.webm", "app", b"not ebml");
        assert_eq!(bad.errors.len(), 1);
    }

    #[test]
    fn code_annotations_emit_implements_edges_with_lines() {
        let text = "// @design:REQ-COMBAT\nfn damage() {}\n// @spec:REQ-STATS#Damage\n";
        let edges = code_markers("src/combat.rs", "app", text);
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].kind, "implements");
        assert_eq!(edges[0].dst, "design:app:REQ-COMBAT");
        assert_eq!(edges[0].line, 1);
        assert_eq!(edges[1].dst, "design:app:REQ-STATS");
        assert_eq!(edges[1].line, 3);
    }

    #[test]
    fn markdown_relative_links_become_doc_links() {
        let text = "# Design\n\nSee [Spec](specs/Combat.md) and [Hero](../art/hero.png).\n\n[Web](https://example.com/a.md)\n";
        let fp = parse("docs/Design.md", "app", "design_doc", text.as_bytes());
        let dst: Vec<&str> = fp
            .asset_edges
            .iter()
            .filter(|e| e.kind == "doc_link")
            .map(|e| e.dst.as_str())
            .collect();
        assert!(dst.contains(&"asset:app:docs/specs/Combat.md"));
        assert!(dst.contains(&"asset:app:art/hero.png"));
        assert!(!dst.iter().any(|d| d.contains("example.com")));
    }
}
