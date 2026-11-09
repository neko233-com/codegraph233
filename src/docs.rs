//! Office / PDF document frontends.
//!
//! These frontends are deliberately structural: DOCX/PPTX are read as ZIP
//! packages, PDF content streams are only decompressed when they use the
//! standard Flate filter, and nothing is executed. Legal, patent and
//! compliance documents are usually long prose: the index keeps the title,
//! headings/slides, word counts and a bounded excerpt, plus explicit
//! `@code:` / `@asset:` / `[[REQ-...]]` markers that connect the document to
//! source files and requirements. Full-text search or OCR is out of scope.

use crate::model::{Asset, AssetEdge, FileParse};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SECTIONS: usize = 500;
const MAX_SLIDES: usize = 500;
const MAX_TEXT_CHARS: usize = 200_000;
const EXCERPT_CHARS: usize = 240;
const MAX_REFS: usize = 200;

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    match lang {
        "docx" => parse_docx(rel, root, data),
        "pptx" => parse_pptx(rel, root, data),
        "pdf" => parse_pdf(rel, root, data),
        "legacy_office" => legacy_office(rel, root, lang),
        other => error_res(rel, root, other, format!("unsupported document frontend {other}")),
    }
}

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn child_key(root: &str, rel: &str, kind: &str, name: &str) -> String {
    format!("asset:{root}:{rel}#{kind}:{name}")
}

fn base_name(rel: &str) -> String {
    Path::new(rel)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    out.trim_end_matches('-').to_string()
}

fn error_res(rel: &str, root: &str, lang: &str, msg: String) -> FileParse {
    FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        errors: vec![msg],
        ..Default::default()
    }
}

fn push_asset(
    res: &mut FileParse,
    root: &str,
    key: String,
    kind: &str,
    rel: &str,
    name: &str,
    meta: HashMap<String, String>,
) {
    res.assets.push(Asset {
        key,
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 0,
        meta,
    });
}

fn add_edge(res: &mut FileParse, src: &str, dst: &str, kind: &str, detail: &str) {
    if src.is_empty() || dst.is_empty() {
        return;
    }
    res.asset_edges.push(AssetEdge {
        src: src.to_string(),
        dst: dst.to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
        root: String::new(),
        line: 0,
    });
}

fn excerpt(text: &str) -> String {
    let trimmed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = trimmed.chars().take(EXCERPT_CHARS).collect();
    if trimmed.chars().count() > EXCERPT_CHARS {
        out.push('…');
    }
    out
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

// ---------------------------------------------------------------- zip helpers

fn read_zip_entry(data: &[u8], name: &str) -> Option<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).ok()?;
    let mut file = archive.by_name(name).ok()?;
    if file.size() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

// ---------------------------------------------------------------- docx

#[derive(Default)]
struct DocSection {
    level: usize,
    title: String,
    text: String,
}

fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn collect_tag_text(block: &str, tag: &str, out: &mut String) {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = block;
    while let Some(start) = rest.find(&open) {
        let Some(gt) = rest[start..].find('>') else { break };
        let body_start = start + gt + 1;
        let Some(end) = rest[body_start..].find(&close) else { break };
        out.push_str(&xml_unescape(&rest[body_start..body_start + end]));
        rest = &rest[body_start + end + close.len()..];
    }
}

fn wml_paragraphs(xml: &str) -> Vec<(usize, String)> {
    // Returns (heading_level, text); level 0 means body paragraph.
    let mut out = Vec::new();
    let mut rest = xml;
    let open = "<w:p";
    let close = "</w:p>";
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        if !after.starts_with(['>', ' ', '\t', '\r', '\n', '/']) {
            rest = after;
            continue;
        }
        let Some(gt) = rest[start..].find('>') else { break };
        let body_start = start + gt + 1;
        let Some(end) = rest[body_start..].find(close) else { break };
        let block = &rest[body_start..body_start + end];
        let mut text = String::new();
        collect_tag_text(block, "w:t", &mut text);
        let mut level = 0;
        if let Some(style_pos) = block.find("w:pStyle") {
            if let Some(val) = block[style_pos..].find("w:val=\"") {
                let start = style_pos + val + 7;
                if let Some(endq) = block[start..].find('"') {
                    let style = &block[start..start + endq];
                    level = heading_level(style);
                }
            }
        }
        if level == 0 {
            if let Some(pos) = block.find("w:outlineLvl") {
                if let Some(val) = block[pos..].find("w:val=\"") {
                    let start = pos + val + 7;
                    if let Some(endq) = block[start..].find('"') {
                        level = block[start..start + endq]
                            .parse::<usize>()
                            .map(|n| n.saturating_add(1))
                            .unwrap_or(0);
                    }
                }
            }
        }
        if !text.trim().is_empty() {
            out.push((level, text));
        }
        rest = &rest[body_start + end + close.len()..];
    }
    out
}

fn heading_level(style: &str) -> usize {
    if style.eq_ignore_ascii_case("title") {
        return 1;
    }
    let lower = style.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("heading") {
        return rest.trim().parse::<usize>().unwrap_or(0).min(6);
    }
    0
}

fn count_occurrences(text: &str, needle: &str) -> usize {
    text.match_indices(needle).count()
}

fn parse_docx(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "docx".into(),
        ..Default::default()
    };
    let Some(document) = read_zip_entry(data, "word/document.xml") else {
        return error_res(rel, root, "docx", "docx is not a readable ZIP package (missing word/document.xml)".into());
    };
    let core = read_zip_entry(data, "docProps/core.xml").unwrap_or_default();
    let mut meta = HashMap::new();
    meta.insert("format".into(), "docx".into());
    meta.insert("bytes".into(), data.len().to_string());

    let paragraphs = wml_paragraphs(&document);
    let mut sections: Vec<DocSection> = Vec::new();
    let mut joined = String::new();
    for (level, text) in &paragraphs {
        joined.push_str(text);
        joined.push('\n');
        if *level > 0 {
            if sections.len() < MAX_SECTIONS {
                sections.push(DocSection {
                    level: *level,
                    title: text.trim().to_string(),
                    text: String::new(),
                });
            }
        } else if let Some(current) = sections.last_mut() {
            if current.text.len() < MAX_TEXT_CHARS {
                current.text.push_str(text.trim());
                current.text.push('\n');
            }
        }
    }
    let title = xml_field(&core, "dc:title")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| base_name(rel));
    let author = xml_field(&core, "dc:creator").unwrap_or_default();
    let tables = count_occurrences(&document, "<w:tbl");
    meta.insert("title".into(), title.clone());
    if !author.is_empty() {
        meta.insert("author".into(), author);
    }
    if let Some(created) = xml_field(&core, "dcterms:created") {
        meta.insert("created".into(), created);
    }
    meta.insert("sections".into(), sections.len().to_string());
    meta.insert("paragraphs".into(), paragraphs.len().to_string());
    meta.insert("tables".into(), tables.to_string());
    meta.insert("words".into(), word_count(&joined).to_string());
    if let Some(first) = paragraphs.iter().find(|(_, t)| !t.trim().is_empty()) {
        meta.insert("excerpt".into(), excerpt(&first.1));
    }

    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title.clone(),
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for section in &sections {
        let slug = slugify(&section.title);
        if slug.is_empty() {
            continue;
        }
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), section.level.to_string());
        m.insert("excerpt".into(), excerpt(&section.text));
        push_asset(&mut res, root, key.clone(), "doc_section", rel, &section.title, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    scan_refs(rel, root, &joined, &mut res);
    res
}

fn xml_field(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let gt = xml[start..].find('>')? + start;
    let body_start = gt + 1;
    let end = xml[body_start..].find(&close)? + body_start;
    Some(xml_unescape(&xml[body_start..end]).trim().to_string())
}

// ---------------------------------------------------------------- pptx

fn parse_pptx(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "pptx".into(),
        ..Default::default()
    };
    let mut archive = match zip::ZipArchive::new(Cursor::new(data)) {
        Ok(a) => a,
        Err(e) => return error_res(rel, root, "pptx", format!("pptx is not a valid ZIP package: {e}")),
    };
    let mut slides: Vec<(u32, String)> = Vec::new();
    for i in 0..archive.len() {
        let name = match archive.by_index(i) {
            Ok(f) => f.name().to_string(),
            Err(_) => continue,
        };
        if !name.starts_with("ppt/slides/slide") || !name.ends_with(".xml") {
            continue;
        }
        let number = name
            .trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(0);
        slides.push((number, name));
    }
    slides.sort_by_key(|(n, _)| *n);
    let mut meta = HashMap::new();
    meta.insert("format".into(), "pptx".into());
    meta.insert("bytes".into(), data.len().to_string());
    let first_slide_title = slides
        .iter()
        .take(1)
        .find_map(|(_, name)| read_zip_entry(data, name))
        .and_then(|xml| tag_texts(&xml, "a:t").into_iter().next())
        .unwrap_or_default();
    let title = read_zip_entry(data, "docProps/core.xml")
        .as_deref()
        .and_then(|core| xml_field(core, "dc:title"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let t = first_slide_title.trim();
            (!t.is_empty()).then(|| t.to_string())
        })
        .unwrap_or_else(|| base_name(rel));
    meta.insert("title".into(), title.clone());
    meta.insert("slides".into(), slides.len().to_string());
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "presentation".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    let mut all_text = String::new();
    for (index, (_, name)) in slides.iter().take(MAX_SLIDES).enumerate() {
        let Some(xml) = read_zip_entry(data, name) else { continue };
        let runs = tag_texts(&xml, "a:t");
        let slide_title = runs.first().map(|s| s.trim().to_string()).unwrap_or_default();
        let body = runs
            .iter()
            .skip(1)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        all_text.push_str(&slide_title);
        all_text.push('\n');
        all_text.push_str(&body);
        all_text.push('\n');
        let slug = slugify(&slide_title);
        let slug = if slug.is_empty() { format!("slide-{}", index + 1) } else { slug };
        let key = format!("asset:{root}:{rel}#doc_slide:{slug}");
        let mut m = HashMap::new();
        m.insert("index".into(), (index + 1).to_string());
        m.insert("excerpt".into(), excerpt(&body));
        m.insert("words".into(), word_count(&body).to_string());
        push_asset(&mut res, root, key.clone(), "doc_slide", rel, &slide_title, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_slide");
    }
    meta_text_stats(&mut res, &all_text);
    scan_refs(rel, root, &all_text, &mut res);
    res
}

fn tag_texts(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let Some(gt) = rest[start..].find('>') else { break };
        let body_start = start + gt + 1;
        let Some(end) = rest[body_start..].find(&close) else { break };
        out.push(xml_unescape(&rest[body_start..body_start + end]));
        rest = &rest[body_start + end + close.len()..];
    }
    out
}

fn meta_text_stats(res: &mut FileParse, text: &str) {
    if let Some(asset) = res.assets.first_mut() {
        asset.meta.insert("words".into(), word_count(text).to_string());
        asset.meta.insert("excerpt".into(), excerpt(text));
    }
}

// ---------------------------------------------------------------- pdf

fn parse_pdf(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "pdf".into(),
        ..Default::default()
    };
    if !data.starts_with(b"%PDF-") {
        return error_res(rel, root, "pdf", "not a PDF (missing %PDF- header)".into());
    }
    let head = String::from_utf8_lossy(&data[..data.len().min(4096)]);
    let version = head
        .split_once("%PDF-")
        .map(|(_, v)| v.chars().take_while(|c| !c.is_whitespace()).collect::<String>())
        .unwrap_or_default();
    let pages = count_pdf_pages(data).or_else(|| pdf_page_count(&head)).unwrap_or(0);
    let mut meta = HashMap::new();
    meta.insert("format".into(), "pdf".into());
    meta.insert("version".into(), version);
    meta.insert("bytes".into(), data.len().to_string());
    meta.insert("pages".into(), pages.to_string());
    if let Some(title) = pdf_info_string(data, "Title") {
        meta.insert("title".into(), title.clone());
        if let Some(asset) = res.assets.first_mut() {
            asset.name = title;
        }
    }
    if let Some(author) = pdf_info_string(data, "Author") {
        meta.insert("author".into(), author);
    }
    if let Some(created) = pdf_info_string(data, "CreationDate") {
        meta.insert("created".into(), created);
    }
    let text = pdf_text(data);
    let chars = text.chars().count();
    meta.insert("text_chars".into(), chars.to_string());
    meta.insert("text_extracted".into(), if chars > 0 { "1" } else { "0" }.into());
    if chars > 0 {
        meta.insert("excerpt".into(), excerpt(&text));
    }
    meta.insert(
        "limits".into(),
        "content-stream text only; no OCR, CMap/font decoding or encrypted streams".into(),
    );
    let name = meta
        .get("title")
        .cloned()
        .unwrap_or_else(|| base_name(rel));
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    if chars > 0 {
        scan_refs(rel, root, &text, &mut res);
    }
    res
}

fn count_pdf_pages(data: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(data);
    let mut count = 0;
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("/Type") {
        let tail = &rest[pos + 5..];
        let trimmed = tail.trim_start();
        if let Some(after) = trimmed.strip_prefix("/Page") {
            if !after.starts_with('s') {
                count += 1;
            }
        }
        rest = &rest[pos + 5..];
    }
    if count > 0 {
        Some(count)
    } else {
        None
    }
}

fn pdf_page_count(head: &str) -> Option<usize> {
    let pos = head.find("/Count")?;
    head[pos + 6..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn pdf_info_string(data: &[u8], key: &str) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let needle = format!("/{key}");
    let mut from = 0;
    while let Some(pos) = text[from..].find(&needle) {
        let abs = from + pos;
        let after = text[abs + needle.len()..].trim_start();
        if let Some(rest) = after.strip_prefix('(') {
            let mut out = String::new();
            let mut escape = false;
            let mut depth = 0;
            for ch in rest.chars() {
                if escape {
                    out.push(match ch {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                    escape = false;
                    continue;
                }
                match ch {
                    '\\' => escape = true,
                    '(' => {
                        depth += 1;
                        out.push('(');
                    }
                    ')' if depth == 0 => break,
                    ')' => {
                        depth -= 1;
                        out.push(')');
                    }
                    other => out.push(other),
                }
            }
            let out = out.trim().to_string();
            if !out.is_empty() {
                return Some(out);
            }
        }
        from = abs + needle.len();
    }
    None
}

fn pdf_text(data: &[u8]) -> String {
    let mut out = String::new();
    let mut from = 0;
    while let Some(pos) = find_bytes(&data[from..], b"stream") {
        let abs = from + pos;
        // Require a line break after the keyword and a dictionary before it.
        let mut start = abs + 6;
        while start < data.len() && (data[start] == b'\r' || data[start] == b'\n') {
            start += 1;
        }
        let dict_start = abs.saturating_sub(400);
        let dict = String::from_utf8_lossy(&data[dict_start..abs]);
        let Some(rel_end) = find_bytes(&data[start..], b"endstream") else { break };
        let end = start + rel_end;
        let raw = &data[start..end];
        let content: Vec<u8> = if dict.contains("/FlateDecode") {
            let mut decoded = Vec::new();
            let mut decoder = flate2::read::ZlibDecoder::new(Cursor::new(raw));
            if decoder.read_to_end(&mut decoded).is_err() {
                from = end + 9;
                continue;
            }
            decoded
        } else if dict.contains("/Filter") {
            from = end + 9;
            continue;
        } else {
            raw.to_vec()
        };
        let text = String::from_utf8_lossy(&content);
        extract_pdf_literals(&text, &mut out);
        if out.len() >= MAX_TEXT_CHARS {
            break;
        }
        from = end + 9;
    }
    out
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn extract_pdf_literals(content: &str, out: &mut String) {
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        let mut value = String::new();
        i += 1;
        let mut depth = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    i += 1;
                    if i >= bytes.len() {
                        break;
                    }
                    let ch = match bytes[i] {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        other => other as char,
                    };
                    value.push(ch);
                }
                b'(' => {
                    depth += 1;
                    value.push('(');
                }
                b')' if depth == 0 => {
                    i += 1;
                    break;
                }
                b')' => {
                    depth -= 1;
                    value.push(')');
                }
                other => value.push(other as char),
            }
            i += 1;
        }
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
            out.push_str(trimmed);
            if out.len() >= MAX_TEXT_CHARS {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------- legacy office

fn legacy_office(rel: &str, root: &str, lang: &str) -> FileParse {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut res = error_res(
        rel,
        root,
        lang,
        format!(".{ext} is a legacy binary Office file; save it as .docx/.pptx/.xlsx to index it"),
    );
    let mut meta = HashMap::new();
    meta.insert("format".into(), ext);
    meta.insert("legacy".into(), "1".into());
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: base_name(rel),
        unity_id: String::new(),
        line: 0,
        meta,
    });
    res
}

// ---------------------------------------------------------------- markers

fn scan_refs(rel: &str, root: &str, text: &str, res: &mut FileParse) {
    let src = file_key(root, rel);
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut emitted = 0usize;
    for (marker, kind) in [
        ("@code:", "design_code"),
        ("@asset:", "design_asset"),
        ("@audio:", "design_audio"),
        ("@video:", "design_video"),
        ("@art:", "design_art"),
        ("@story:", "design_story"),
        ("@value:", "design_value"),
        ("@text:", "design_text"),
        ("@link:", "design_link"),
    ] {
        let mut from = 0;
        while let Some(pos) = text[from..].find(marker) {
            let mut start = from + pos + marker.len();
            let tail = &text[start..];
            start += tail.len() - tail.trim_start().len();
            let end = text[start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<')
                .map(|i| start + i)
                .unwrap_or(text.len());
            let raw = text[start..end].trim();
            if !raw.is_empty() && emitted < MAX_REFS {
                let (path, fragment) = match raw.find('#') {
                    Some(i) => (&raw[..i], &raw[i + 1..]),
                    None => (raw, ""),
                };
                let dst = if kind == "design_link" {
                    format!("design:{root}:{path}")
                } else {
                    format!("asset:{root}:{path}")
                };
                if seen.insert((dst.clone(), kind.to_string())) {
                    add_edge(res, &src, &dst, kind, fragment);
                    emitted += 1;
                }
            }
            from = end;
        }
    }
    let mut rest = text;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else { break };
        let id = after[..close].trim();
        if !id.is_empty() && id.len() <= 128 && emitted < MAX_REFS {
            let dst = format!("design:{root}:{id}");
            if seen.insert((dst.clone(), "design_link".into())) {
                add_edge(res, &src, &dst, "design_link", "");
                emitted += 1;
            }
        }
        rest = &after[close + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in files {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn has_asset(fp: &FileParse, kind: &str, name: &str) -> bool {
        fp.assets
            .iter()
            .any(|a| a.kind == kind && a.name == name)
    }

    fn has_edge(fp: &FileParse, kind: &str, dst_fragment: &str) -> bool {
        fp.asset_edges
            .iter()
            .any(|e| e.kind == kind && e.dst.contains(dst_fragment))
    }

    #[test]
    fn docx_extracts_headings_markers_and_metadata() {
        let document = r#"<w:document><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Claims</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">The invention </w:t></w:r><w:r><w:t>claims @code: src/Player.cs#take_damage and [[REQ-COMBAT]].</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#;
        let core = r#"<cp:coreProperties><dc:title>Patent Draft</dc:title><dc:creator>Neko</dc:creator></cp:coreProperties>"#;
        let data = zip_bytes(&[
            ("word/document.xml", document),
            ("docProps/core.xml", core),
        ]);
        let fp = parse("docs/Patent.docx", "app", "docx", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "document", "Patent Draft"));
        assert!(has_asset(&fp, "doc_section", "Claims"));
        assert!(has_edge(&fp, "contains", "#doc_section:claims"));
        assert!(has_edge(&fp, "design_code", "asset:app:src/Player.cs"));
        assert!(has_edge(&fp, "design_link", "design:app:REQ-COMBAT"));
        let doc = &fp.assets[0];
        assert_eq!(doc.meta.get("tables").map(String::as_str), Some("1"));
        assert_eq!(doc.meta.get("author").map(String::as_str), Some("Neko"));
        assert_eq!(doc.meta.get("sections").map(String::as_str), Some("1"));
    }

    #[test]
    fn pptx_extracts_slide_titles() {
        let slide1 = r#"<p:sld><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Pitch Deck</a:t></a:r></a:p><a:p><a:r><a:t>Milestones</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
        let slide2 = r#"<p:sld><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Team</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
        let data = zip_bytes(&[
            ("ppt/slides/slide1.xml", slide1),
            ("ppt/slides/slide2.xml", slide2),
        ]);
        let fp = parse("pitch/Deck.pptx", "app", "pptx", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(fp.assets.iter().any(|a| a.kind == "presentation"));
        assert_eq!(fp.assets.iter().filter(|a| a.kind == "doc_slide").count(), 2);
        assert!(has_edge(&fp, "contains", "#doc_slide:pitch-deck"));
    }

    #[test]
    fn pdf_extracts_pages_title_and_stream_text() {
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Count 2 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\n4 0 obj\n<< /Title (Patent Draft) /Author (Neko) >>\nendobj\n5 0 obj\n<< /Length 44 >>\nstream\nBT /F1 12 Tf (Hello patent) Tj ET\nendstream\nendobj\ntrailer\n%%EOF\n";
        let fp = parse("legal/draft.pdf", "app", "pdf", pdf);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp
            .assets
            .iter()
            .find(|a| a.kind == "document")
            .expect("document asset");
        assert_eq!(doc.meta.get("pages").map(String::as_str), Some("2"));
        assert_eq!(doc.meta.get("text_extracted").map(String::as_str), Some("1"));
        assert!(doc.meta.get("excerpt").unwrap().contains("Hello patent"));
        assert_eq!(doc.meta.get("title").map(String::as_str), Some("Patent Draft"));
    }

    #[test]
    fn bad_pdf_and_legacy_office_are_visible() {
        let fp = parse("legal/scanned.pdf", "app", "pdf", b"not a pdf");
        assert_eq!(fp.errors.len(), 1);
        assert!(fp.errors[0].contains("missing %PDF-"));
        let fp = parse("legal/old.doc", "app", "legacy_office", &[0xD0, 0xCF, 0x11, 0xE0]);
        assert!(fp.errors[0].contains("legacy binary Office"));
        assert!(fp.assets.iter().any(|a| a.kind == "document"));
    }
}
