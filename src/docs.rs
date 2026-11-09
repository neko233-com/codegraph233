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
use regex::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SECTIONS: usize = 500;
const MAX_SLIDES: usize = 500;
const MAX_TEXT_CHARS: usize = 200_000;
const EXCERPT_CHARS: usize = 240;
const MAX_REFS: usize = 200;
const MAX_CLAIMS: usize = 100;
const MAX_CLAUSES: usize = 200;

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    match lang {
        "docx" => parse_docx(rel, root, data),
        "pptx" => parse_pptx(rel, root, data),
        "pdf" => parse_pdf(rel, root, data),
        "plain_text" => parse_plain_text(rel, root, data),
        "rtf" => parse_rtf(rel, root, data),
        "odt" => parse_odt(rel, root, data),
        "epub" => parse_epub(rel, root, data),
        "html" => parse_html(rel, root, data),
        "latex" => parse_latex(rel, root, data),
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
    finish_document(rel, root, &joined, &mut res);
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
    finish_document(rel, root, &all_text, &mut res);
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
        finish_document(rel, root, &text, &mut res);
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

fn finish_document(rel: &str, root: &str, text: &str, res: &mut FileParse) {
    scan_refs(rel, root, text, res);
    scan_legal(rel, root, text, res);
    let words = word_count(text).to_string();
    let fallback = excerpt(text);
    if let Some(asset) = res.assets.first_mut() {
        asset.meta.entry("words".into()).or_insert(words);
        let empty = asset
            .meta
            .get("excerpt")
            .map(|e| e.trim().is_empty())
            .unwrap_or(true);
        if empty {
            asset.meta.insert("excerpt".into(), fallback);
        }
    }
}

fn scan_legal(rel: &str, root: &str, text: &str, res: &mut FileParse) {
    let src = file_key(root, rel);
    let claims = extract_claims(text);
    if !claims.is_empty() {
        let count = claims.len().min(MAX_CLAIMS);
        for (index, claim) in claims.iter().take(MAX_CLAIMS).enumerate() {
            let key = format!("asset:{root}:{rel}#patent_claim:{}", index + 1);
            let mut meta = HashMap::new();
            meta.insert("index".into(), (index + 1).to_string());
            meta.insert("excerpt".into(), excerpt(claim));
            meta.insert("words".into(), word_count(claim).to_string());
            push_asset(
                res,
                root,
                key.clone(),
                "patent_claim",
                rel,
                &format!("Claim {}", index + 1),
                meta,
            );
            add_edge(res, &src, &key, "claims", &format!("claim {}", index + 1));
        }
        if let Some(asset) = res.assets.first_mut() {
            asset.meta.insert("claim_count".into(), count.to_string());
            asset.meta.insert("claims_dialect".into(), "numbered".into());
        }
    }

    let clauses = extract_clauses(text);
    if !clauses.is_empty() {
        let count = clauses.len().min(MAX_CLAUSES);
        for (index, (name, body)) in clauses.iter().take(MAX_CLAUSES).enumerate() {
            let slug = slugify(name);
            let slug = if slug.is_empty() { format!("clause-{}", index + 1) } else { slug };
            let key = child_key(root, rel, "legal_clause", &slug);
            let mut meta = HashMap::new();
            meta.insert("index".into(), (index + 1).to_string());
            meta.insert("excerpt".into(), excerpt(body));
            meta.insert("words".into(), word_count(body).to_string());
            push_asset(res, root, key.clone(), "legal_clause", rel, name, meta);
            add_edge(res, &src, &key, "contains", "legal_clause");
        }
        if let Some(asset) = res.assets.first_mut() {
            asset.meta.insert("legal_clause_count".into(), count.to_string());
        }
    }

    let numbers = patent_numbers(text);
    if !numbers.is_empty() {
        if let Some(asset) = res.assets.first_mut() {
            asset
                .meta
                .insert("patent_numbers".into(), numbers.join(";"));
            asset
                .meta
                .insert("patent_number_count".into(), numbers.len().to_string());
        }
    }
}

fn extract_claims(text: &str) -> Vec<String> {
    let mut started = false;
    let mut claims: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !started {
            let lower = trimmed.to_lowercase();
            if lower == "claims"
                || trimmed == "权利要求书"
                || trimmed.contains("权利要求")
                || lower.starts_with("what is claimed")
                || lower.starts_with("the invention claimed")
            {
                started = true;
            }
            continue;
        }
        if trimmed.is_empty() {
            if !current.is_empty() {
                current.push(' ');
            }
            continue;
        }
        if claim_boundary(trimmed) {
            break;
        }
        if claim_number(trimmed).is_some() {
            if !current.trim().is_empty() {
                claims.push(current.trim().to_string());
            }
            current.clear();
            current.push_str(strip_claim_prefix(trimmed));
            if claims.len() >= MAX_CLAIMS {
                break;
            }
        } else if !current.is_empty() {
            if current.len() < 4000 {
                current.push(' ');
                current.push_str(trimmed);
            }
        }
    }
    if !current.trim().is_empty() && claims.len() < MAX_CLAIMS {
        claims.push(current.trim().to_string());
    }
    claims
}

fn claim_number(text: &str) -> Option<usize> {
    let t = text.trim();
    for prefix in ["Claim ", "claim ", "CLAIM "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                let after = rest[digits.len()..].trim_start();
                if after.starts_with(['.', ')', ':', '：']) {
                    return digits.parse().ok();
                }
            }
        }
    }
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = t[digits.len()..].trim_start();
    if after.starts_with(['.', '．', '、', ')', ']', ':', '：']) {
        digits.parse().ok()
    } else {
        None
    }
}

fn strip_claim_prefix(text: &str) -> &str {
    let t = text.trim();
    for prefix in ["Claim ", "claim ", "CLAIM "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let digits: usize = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            let rest = rest[digits..].trim_start();
            if let Some(pos) = rest.find(['.', ')', ':', '：']) {
                return rest[pos + 1..].trim_start();
            }
        }
    }
    let digits: usize = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let rest = t[digits..].trim_start();
    rest.trim_start_matches(['.', '．', '、', ')', ']', ':', '：'])
        .trim_start()
}

fn claim_boundary(text: &str) -> bool {
    let upper = text.trim().to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "ABSTRACT" | "DESCRIPTION" | "DETAILED DESCRIPTION" | "BACKGROUND" | "DRAWINGS"
    ) || matches!(text.trim(), "说明书" | "附图说明" | "具体实施方式" | "技术领域" | "背景技术")
}

fn extract_clauses(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = clause_heading(trimmed) {
            if let Some(item) = current.take() {
                out.push(item);
            }
            current = Some((heading, String::new()));
            continue;
        }
        if let Some((_, body)) = current.as_mut() {
            if body.len() < 4000 {
                body.push(' ');
                body.push_str(trimmed);
            }
        }
    }
    if let Some(item) = current.take() {
        out.push(item);
    }
    out.retain(|(name, _)| !name.is_empty());
    out.truncate(MAX_CLAUSES);
    out
}

fn clause_heading(line: &str) -> Option<String> {
    if line.starts_with('第') {
        let mut chars = line.char_indices();
        for (idx, ch) in chars.by_ref() {
            if ch == '条' {
                let end = idx + ch.len_utf8();
                let heading = line[..end].trim();
                if heading.chars().count() <= 24 {
                    let rest = line[end..].trim_start();
                    if rest.is_empty()
                        || rest.starts_with([':', '：', '.', '．', '、', ' ', '\u{3000}'])
                    {
                        return Some(heading.to_string());
                    }
                }
                return None;
            }
            if idx > 30 {
                return None;
            }
        }
        return None;
    }
    for prefix in ["Article ", "article ", "Clause ", "clause ", "Section ", "section "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let token: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if token.is_empty() {
                continue;
            }
            let after = rest[token.len()..].trim_start();
            if after.is_empty() || after.starts_with([':', '：', '.', '．', '–', '-']) {
                let label = prefix.trim_end().to_string();
                return Some(format!("{label} {token}"));
            }
        }
    }
    None
}

fn patent_numbers(text: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '/') {
        let token = raw.trim_matches(['-', '/']);
        if token.len() < 8 || token.len() > 24 {
            continue;
        }
        let upper = token.to_ascii_uppercase();
        let mut matched = false;
        for prefix in ["CN", "US", "WO", "EP", "JP", "KR"] {
            if let Some(rest) = upper.strip_prefix(prefix) {
                if rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                    && rest.chars().filter(|c| c.is_ascii_digit()).count() >= 6
                {
                    matched = true;
                }
            }
        }
        if matched {
            set.insert(upper);
        }
    }
    set.into_iter().take(50).collect()
}

fn parse_plain_text(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let text = decode_text(data);
    parse_text_like(rel, root, "plain_text", "txt", data.len(), &text)
}

fn decode_text(data: &[u8]) -> String {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = data.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(data).into_owned()
}

fn parse_text_like(
    rel: &str,
    root: &str,
    lang: &str,
    format: &str,
    bytes: usize,
    text: &str,
) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        ..Default::default()
    };
    let mut sections: Vec<(usize, String, String)> = Vec::new();
    let mut preamble = String::new();
    for line in text.lines() {
        if let Some((level, title)) = text_heading(line) {
            if sections.len() < MAX_SECTIONS {
                sections.push((level, title, String::new()));
            }
        } else if let Some(last) = sections.last_mut() {
            if last.2.len() < 8000 {
                last.2.push_str(line);
                last.2.push('\n');
            }
        } else {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }
    let first_line = text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let title = sections
        .first()
        .map(|(_, title, _)| title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            if !first_line.is_empty() && first_line.chars().count() <= 120 {
                first_line.to_string()
            } else {
                base_name(rel)
            }
        });
    let mut meta = HashMap::new();
    meta.insert("format".into(), format.to_string());
    meta.insert("bytes".into(), bytes.to_string());
    meta.insert("lines".into(), text.lines().count().to_string());
    meta.insert("sections".into(), sections.len().to_string());
    meta.insert("words".into(), word_count(text).to_string());
    meta.insert("excerpt".into(), excerpt(&preamble));
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for (index, (level, name, body)) in sections.iter().enumerate() {
        let slug = slugify(name);
        let slug = if slug.is_empty() { format!("section-{}", index + 1) } else { slug };
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), level.to_string());
        m.insert("excerpt".into(), excerpt(body));
        push_asset(&mut res, root, key.clone(), "doc_section", rel, name, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    finish_document(rel, root, text, &mut res);
    res
}

fn text_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        let level = trimmed.chars().take_while(|c| *c == '#').count();
        let title = rest.trim_start_matches('#').trim();
        if !title.is_empty() && level <= 6 {
            return Some((level, title.to_string()));
        }
    }
    let upper = trimmed.to_ascii_uppercase();
    if matches!(upper.as_str(), "CLAIMS" | "ABSTRACT" | "DESCRIPTION" | "BACKGROUND")
        || trimmed == "权利要求书"
        || trimmed == "说明书"
    {
        return Some((1, trimmed.to_string()));
    }
    clause_heading(trimmed).map(|h| (1, h))
}

fn parse_rtf(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let text = rtf_text(data);
    let title = rtf_title(data).filter(|t| !t.trim().is_empty());
    let mut res = parse_text_like(rel, root, "rtf", "rtf", data.len(), &text);
    if let (Some(title), Some(asset)) = (title, res.assets.first_mut()) {
        asset.name = title;
    }
    res
}

fn rtf_title(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let start = text.find("{\\title")?;
    let mut depth = 0i32;
    let mut end = None;
    let mut escaped = false;
    for (idx, ch) in text[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let inner = text[start + 7..end].trim();
    let cleaned = rtf_text(inner.as_bytes());
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

fn rtf_text(data: &[u8]) -> String {
    let chars: Vec<char> = String::from_utf8_lossy(data).chars().collect();
    let mut out = String::new();
    let mut stack: Vec<bool> = Vec::new();
    let mut skip: Option<usize> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '{' {
            let j = i + 1;
            if j < chars.len() && chars[j] == '\\' && j + 1 < chars.len() && chars[j + 1] == '*' {
                skip = Some(stack.len() + 1);
            } else if j < chars.len() && chars[j] == '\\' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_alphabetic() {
                    k += 1;
                }
                let word: String = chars[j + 1..k].iter().collect();
                if matches!(
                    word.as_str(),
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "object"
                        | "header" | "footer" | "filetbl" | "listtable" | "listoverridetable"
                ) {
                    skip = Some(stack.len() + 1);
                }
            }
            stack.push(true);
            i += 1;
            continue;
        }
        if ch == '}' {
            stack.pop();
            if let Some(target) = skip {
                if stack.len() < target {
                    skip = None;
                }
            }
            i += 1;
            continue;
        }
        if skip.is_some() {
            i += 1;
            continue;
        }
        if ch == '\\' {
            if i + 1 >= chars.len() {
                break;
            }
            let next = chars[i + 1];
            if !next.is_ascii_alphabetic() {
                match next {
                    '\\' | '{' | '}' => out.push(next),
                    '\'' => {
                        let hex: String = chars
                            .get(i + 2..i + 4)
                            .map(|s| s.iter().collect())
                            .unwrap_or_default();
                        if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                            out.push(byte as char);
                        }
                        i += 4;
                        continue;
                    }
                    '~' => out.push(' '),
                    '_' => out.push('-'),
                    _ => {}
                }
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let word: String = chars[i + 1..j].iter().collect();
            let mut k = j;
            while k < chars.len() && (chars[k] == '-' || chars[k].is_ascii_digit()) {
                k += 1;
            }
            let param: i64 = chars[j..k].iter().collect::<String>().parse().unwrap_or(0);
            if k < chars.len() && chars[k] == ' ' {
                k += 1;
            }
            match word.as_str() {
                "par" | "line" => out.push('\n'),
                "tab" => out.push('\t'),
                "emdash" => out.push('—'),
                "endash" => out.push('–'),
                "u" if param >= 0 => {
                    if let Some(c) = char::from_u32(param as u32) {
                        out.push(c);
                    }
                    if k < chars.len() {
                        k += 1;
                    }
                }
                _ => {}
            }
            i = k;
            continue;
        }
        if ch != '\r' && ch != '\n' {
            out.push(ch);
        }
        i += 1;
    }
    out
}

fn parse_odt(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let Some(content) = read_zip_entry(data, "content.xml") else {
        return error_res(rel, root, "odt", "odt is not a readable ZIP package (missing content.xml)".into());
    };
    let meta_xml = read_zip_entry(data, "meta.xml").unwrap_or_default();
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "odt".into(),
        ..Default::default()
    };
    let blocks = odt_blocks(&content);
    let mut joined = String::new();
    let mut sections: Vec<(usize, String, String)> = Vec::new();
    for (level, text) in blocks {
        joined.push_str(&text);
        joined.push('\n');
        if level > 0 {
            if sections.len() < MAX_SECTIONS {
                sections.push((level, text.trim().to_string(), String::new()));
            }
        } else if let Some(last) = sections.last_mut() {
            if last.2.len() < 8000 {
                last.2.push_str(text.trim());
                last.2.push('\n');
            }
        }
    }
    let title = xml_field(&meta_xml, "dc:title")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| base_name(rel));
    let mut meta = HashMap::new();
    meta.insert("format".into(), "odt".into());
    meta.insert("bytes".into(), data.len().to_string());
    meta.insert("sections".into(), sections.len().to_string());
    meta.insert("words".into(), word_count(&joined).to_string());
    meta.insert("excerpt".into(), excerpt(&joined));
    if let Some(author) = xml_field(&meta_xml, "meta:initial-creator")
        .or_else(|| xml_field(&meta_xml, "dc:creator"))
    {
        if !author.is_empty() {
            meta.insert("author".into(), author);
        }
    }
    if let Some(created) = xml_field(&meta_xml, "meta:creation-date") {
        meta.insert("created".into(), created);
    }
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for (index, (level, name, body)) in sections.iter().enumerate() {
        let slug = slugify(name);
        let slug = if slug.is_empty() { format!("section-{}", index + 1) } else { slug };
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), level.to_string());
        m.insert("excerpt".into(), excerpt(body));
        push_asset(&mut res, root, key.clone(), "doc_section", rel, name, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    finish_document(rel, root, &joined, &mut res);
    res
}

fn odt_blocks(xml: &str) -> Vec<(usize, String)> {
    let heading = Regex::new(r#"(?s)<text:h[^>]*text:outline-level="(\d+)"[^>]*>(.*?)</text:h>"#)
        .expect("odt heading regex");
    let paragraph = Regex::new(r#"(?s)<text:p[^>]*>(.*?)</text:p>"#).expect("odt paragraph regex");
    let mut items: Vec<(usize, usize, String)> = Vec::new();
    for cap in heading.captures_iter(xml) {
        let Some(full) = cap.get(0) else { continue };
        let level = cap.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        items.push((full.start(), level, strip_xml(&cap[2])));
    }
    for cap in paragraph.captures_iter(xml) {
        let Some(full) = cap.get(0) else { continue };
        items.push((full.start(), 0, strip_xml(&cap[1])));
    }
    items.sort_by_key(|(start, _, _)| *start);
    items
        .into_iter()
        .map(|(_, level, text)| (level, text))
        .filter(|(_, text)| !text.trim().is_empty())
        .collect()
}

fn strip_xml(block: &str) -> String {
    let prepared = block
        .replace("<text:tab/>", " ")
        .replace("<text:tab></text:tab>", " ")
        .replace("<text:line-break/>", "\n")
        .replace("<text:line-break></text:line-break>", "\n")
        .replace("<text:s/>", " ");
    let mut out = String::new();
    let mut in_tag = false;
    for ch in prepared.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    xml_unescape(&out).trim().to_string()
}

fn parse_epub(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let Some(container) = read_zip_entry(data, "META-INF/container.xml") else {
        return error_res(rel, root, "epub", "epub is not a readable ZIP package (missing META-INF/container.xml)".into());
    };
    let Some(opf_path) = xml_attr(&container, "full-path") else {
        return error_res(rel, root, "epub", "epub container.xml has no rootfile full-path".into());
    };
    let Some(opf) = read_zip_entry(data, &opf_path) else {
        return error_res(rel, root, "epub", format!("epub package document {opf_path} is missing"));
    };
    let mut manifest: HashMap<String, String> = HashMap::new();
    for tag in opf.split('<') {
        if !tag.starts_with("item ") && !tag.starts_with("item\t") {
            continue;
        }
        if let (Some(id), Some(href)) = (xml_attr(tag, "id"), xml_attr(tag, "href")) {
            manifest.insert(id, href);
        }
    }
    let mut spine: Vec<String> = Vec::new();
    for tag in opf.split('<') {
        if !tag.starts_with("itemref ") && !tag.starts_with("itemref\t") {
            continue;
        }
        if let Some(idref) = xml_attr(tag, "idref") {
            spine.push(idref);
        }
    }
    let base_dir = opf_path.rsplit_once('/').map(|(dir, _)| dir.to_string()).unwrap_or_default();
    let mut chapters: Vec<(String, String)> = Vec::new();
    for idref in spine.iter().take(MAX_SECTIONS) {
        let Some(href) = manifest.get(idref) else { continue };
        let path = join_posix(&base_dir, href);
        let Some(chapter) = read_zip_entry(data, &path) else { continue };
        let title = html_title(&chapter).unwrap_or_else(|| format!("Chapter {}", chapters.len() + 1));
        chapters.push((title, html_text(&chapter)));
    }
    if chapters.is_empty() {
        for name in zip_names(data) {
            if !(name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm")) {
                continue;
            }
            let Some(chapter) = read_zip_entry(data, &name) else { continue };
            let title = html_title(&chapter).unwrap_or_else(|| name.clone());
            chapters.push((title, html_text(&chapter)));
            if chapters.len() >= MAX_SECTIONS {
                break;
            }
        }
    }
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "epub".into(),
        ..Default::default()
    };
    let title = xml_field(&opf, "dc:title")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| base_name(rel));
    let mut meta = HashMap::new();
    meta.insert("format".into(), "epub".into());
    meta.insert("bytes".into(), data.len().to_string());
    meta.insert("chapters".into(), chapters.len().to_string());
    if let Some(author) = xml_field(&opf, "dc:creator") {
        if !author.is_empty() {
            meta.insert("author".into(), author);
        }
    }
    let joined: String = chapters.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join("\n");
    meta.insert("words".into(), word_count(&joined).to_string());
    meta.insert("excerpt".into(), excerpt(&joined));
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for (name, body) in &chapters {
        let slug = slugify(name);
        let slug = if slug.is_empty() { format!("chapter-{}", res.assets.len()) } else { slug };
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), "1".into());
        m.insert("excerpt".into(), excerpt(body));
        push_asset(&mut res, root, key.clone(), "doc_section", rel, name, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    finish_document(rel, root, &joined, &mut res);
    res
}

fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn zip_names(data: &[u8]) -> Vec<String> {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(data)) else {
        return Vec::new();
    };
    (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .collect()
}

fn join_posix(base: &str, href: &str) -> String {
    let clean = href.split('#').next().unwrap_or(href);
    let combined = if clean.starts_with('/') {
        clean.trim_start_matches('/').to_string()
    } else if base.is_empty() {
        clean.to_string()
    } else {
        format!("{base}/{clean}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn parse_html(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let text = decode_text(data);
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "html".into(),
        ..Default::default()
    };
    let title = html_title(&text).unwrap_or_else(|| base_name(rel));
    let body = html_text(&text);
    let headings = html_headings(&text);
    let mut meta = HashMap::new();
    meta.insert("format".into(), "html".into());
    meta.insert("bytes".into(), data.len().to_string());
    meta.insert("headings".into(), headings.len().to_string());
    meta.insert("words".into(), word_count(&body).to_string());
    meta.insert("excerpt".into(), excerpt(&body));
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for (level, name) in headings.iter().take(MAX_SECTIONS) {
        let slug = slugify(name);
        if slug.is_empty() {
            continue;
        }
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), level.to_string());
        push_asset(&mut res, root, key.clone(), "doc_section", rel, name, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    for href in html_links(&text).iter().take(MAX_REFS) {
        if let Some(target) = resolve_link(rel, href) {
            add_edge(&mut res, &file_key(root, rel), &format!("asset:{root}:{target}"), "doc_link", href);
        }
    }
    finish_document(rel, root, &body, &mut res);
    res
}

fn html_title(xml: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    let cap = re.captures(xml)?;
    let title = strip_xml(&cap[1]);
    (!title.is_empty()).then_some(title)
}

fn html_headings(xml: &str) -> Vec<(usize, String)> {
    let Ok(re) = Regex::new(r"(?is)<h([1-3])[^>]*>(.*?)</h[1-3]>") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cap in re.captures_iter(xml) {
        let level = cap.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
        let text = strip_xml(&cap[2]);
        if !text.is_empty() {
            out.push((level, text));
        }
    }
    out
}

fn html_links(xml: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r#"(?is)<a\s[^>]*href="([^"]+)""#) else {
        return Vec::new();
    };
    re.captures_iter(xml)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn html_text(xml: &str) -> String {
    let mut cleaned = xml.to_string();
    for pattern in [
        r"(?is)<script[^>]*>.*?</script>",
        r"(?is)<style[^>]*>.*?</style>",
        r"(?s)<!--.*?-->",
    ] {
        if let Ok(re) = Regex::new(pattern) {
            cleaned = re.replace_all(&cleaned, " ").into_owned();
        }
    }
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in cleaned.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                if tag.contains("br") || tag.starts_with("p") || tag.starts_with("/p") || tag.starts_with("h") {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    let decoded = xml_unescape(&out);
    decoded
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolve_link(rel: &str, href: &str) -> Option<String> {
    let lower = href.trim().to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("data:")
        || lower.starts_with("javascript:")
        || lower.starts_with('#')
    {
        return None;
    }
    let target = href.split('#').next().unwrap_or(href).split('?').next().unwrap_or(href);
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    let joined = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        join_posix(dir, target)
    };
    let ext = joined.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if ext.is_empty() || ext.len() > 8 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(joined)
}

fn parse_latex(rel: &str, root: &str, data: &[u8]) -> FileParse {
    let text = decode_text(data);
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "latex".into(),
        ..Default::default()
    };
    let title = Regex::new(r"\\title\{([^{}]*)\}")
        .ok()
        .and_then(|re| re.captures(&text))
        .and_then(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| base_name(rel));
    let section_re = Regex::new(r"\\(chapter|section|subsection|subsubsection)\*?\{([^{}]*)\}").expect("latex section regex");
    let mut sections: Vec<(usize, String)> = Vec::new();
    for cap in section_re.captures_iter(&text) {
        let level = match &cap[1] {
            "chapter" => 1,
            "section" => 1,
            "subsection" => 2,
            _ => 3,
        };
        let name = cap[2].trim().to_string();
        if !name.is_empty() && sections.len() < MAX_SECTIONS {
            sections.push((level, name));
        }
    }
    let citations = Regex::new(r"\\cite[a-zA-Z]*\{([^{}]*)\}")
        .expect("latex cite regex")
        .captures_iter(&text)
        .flat_map(|cap| cap[1].split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>();
    let plain = latex_plain(&text);
    let mut meta = HashMap::new();
    meta.insert("format".into(), "latex".into());
    meta.insert("bytes".into(), data.len().to_string());
    meta.insert("sections".into(), sections.len().to_string());
    meta.insert("citations".into(), citations.len().to_string());
    meta.insert("words".into(), word_count(&plain).to_string());
    meta.insert("excerpt".into(), excerpt(&plain));
    if !citations.is_empty() {
        meta.insert("citation_keys".into(), citations.iter().take(50).cloned().collect::<Vec<_>>().join(","));
    }
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "document".into(),
        path: rel.to_string(),
        name: title,
        unity_id: String::new(),
        line: 0,
        meta,
    });
    for (level, name) in &sections {
        let slug = slugify(name);
        if slug.is_empty() {
            continue;
        }
        let key = child_key(root, rel, "doc_section", &slug);
        let mut m = HashMap::new();
        m.insert("level".into(), level.to_string());
        push_asset(&mut res, root, key.clone(), "doc_section", rel, name, m);
        add_edge(&mut res, &file_key(root, rel), &key, "contains", "doc_section");
    }
    finish_document(rel, root, &plain, &mut res);
    res
}

fn latex_plain(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('%') {
            continue;
        }
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '%' => break,
                '\\' => {
                    let mut word = String::new();
                    while let Some(c) = chars.peek() {
                        if c.is_ascii_alphabetic() {
                            word.push(*c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if word.is_empty() {
                        if let Some(c) = chars.next() {
                            out.push(c);
                        }
                    } else if matches!(word.as_str(), "par" | "newline" | "linebreak") {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                }
                '{' | '}' => {}
                '$' | '&' | '#' | '_' | '^' => out.push(' '),
                _ => out.push(ch),
            }
        }
        out.push('\n');
    }
    out
}

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

    #[test]
    fn plain_text_extracts_claims_clauses_and_patent_numbers() {
        let txt = "Patent Application\n\nCLAIMS\n1. A method for indexing games.\n2. The method of claim 1, further comprising audio.\n\nArticle 1: Scope\nThis contract covers CN123456789A and US20200123456A1.\n";
        let fp = parse("legal/Patent.txt", "app", "plain_text", txt.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.meta.get("claim_count").map(String::as_str), Some("2"));
        assert!(doc.meta.get("patent_numbers").unwrap().contains("CN123456789A"));
        assert!(fp.assets.iter().any(|a| a.kind == "patent_claim" && a.name == "Claim 2"));
        assert!(fp.assets.iter().any(|a| a.kind == "legal_clause" && a.name == "Article 1"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "claims"));
        assert!(fp.asset_edges.iter().any(|e| e.kind == "contains" && e.detail == "legal_clause"));
    }

    #[test]
    fn rtf_extracts_text_title_and_claims() {
        let rtf = br"{\rtf1{\info{\title Patent RTF}}{\fonttbl{\f0 Arial;}}{\b CLAIMS}\par Claim 1. A device\par Claim 2. A method}";
        let fp = parse("legal/Patent.rtf", "app", "rtf", rtf);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.name, "Patent RTF");
        assert_eq!(doc.meta.get("claim_count").map(String::as_str), Some("2"));
        assert!(doc.meta.get("excerpt").unwrap().contains("Claim 1"));
    }

    #[test]
    fn odt_extracts_headings_metadata_and_claims() {
        let content = r#"<office:document-content><office:body><office:text><text:h text:outline-level="1">Claims</text:h><text:p>1. A device comprising a processor.</text:p><text:p>Article 2: Term</text:p></office:text></office:body></office:document-content>"#;
        let meta = r#"<office:document-meta><meta:meta><dc:title>Patent ODT</dc:title><meta:initial-creator>Neko</meta:initial-creator><meta:creation-date>2026-01-01</meta:creation-date></meta:meta></office:document-meta>"#;
        let data = zip_bytes(&[("content.xml", content), ("meta.xml", meta)]);
        let fp = parse("legal/Patent.odt", "app", "odt", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.name, "Patent ODT");
        assert_eq!(doc.meta.get("author").map(String::as_str), Some("Neko"));
        assert_eq!(doc.meta.get("claim_count").map(String::as_str), Some("1"));
        assert!(fp.assets.iter().any(|a| a.kind == "doc_section" && a.name == "Claims"));
    }

    #[test]
    fn epub_extracts_spine_chapters_and_metadata() {
        let container = r#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
        let opf = r#"<package><metadata><dc:title>Bestiary</dc:title><dc:creator>Neko</dc:creator></metadata><manifest><item id="c1" href="chapter1.xhtml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
        let chapter = r#"<html><head><title>Chapter One</title></head><body><h1>Chapter One</h1><p>Monsters.</p></body></html>"#;
        let data = zip_bytes(&[
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/chapter1.xhtml", chapter),
        ]);
        let fp = parse("books/Bestiary.epub", "app", "epub", &data);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.name, "Bestiary");
        assert_eq!(doc.meta.get("author").map(String::as_str), Some("Neko"));
        assert!(fp.assets.iter().any(|a| a.kind == "doc_section" && a.name == "Chapter One"));
    }

    #[test]
    fn html_extracts_title_headings_and_relative_links() {
        let html = r#"<html><head><title>Design Page</title></head><body><h1>Combat</h1><p>Read <a href="specs/Combat.md">the spec</a>.</p><a href="https://example.com/x">external</a></body></html>"#;
        let fp = parse("docs/page.html", "app", "html", html.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.name, "Design Page");
        assert!(fp.assets.iter().any(|a| a.kind == "doc_section" && a.name == "Combat"));
        assert!(fp
            .asset_edges
            .iter()
            .any(|e| e.kind == "doc_link" && e.dst == "asset:app:docs/specs/Combat.md"));
    }

    #[test]
    fn latex_extracts_sections_title_and_citations() {
        let tex = r"\documentclass{article}\title{Game Graph Paper}\begin{document}\section{Introduction}\cite{a,b}\subsection{Method}\end{document}";
        let fp = parse("papers/graph.tex", "app", "latex", tex.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let doc = fp.assets.iter().find(|a| a.kind == "document").unwrap();
        assert_eq!(doc.name, "Game Graph Paper");
        assert_eq!(doc.meta.get("citations").map(String::as_str), Some("2"));
        assert!(doc.meta.get("citation_keys").unwrap().contains('a'));
        assert!(fp.assets.iter().any(|a| a.kind == "doc_section" && a.name == "Introduction"));
        assert!(fp.assets.iter().any(|a| a.kind == "doc_section" && a.name == "Method"));
    }
}
