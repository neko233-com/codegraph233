//! Localization frontends: gettext PO/POT, XLIFF 1.2/2.0, .NET RESX and
//! Application Resource Bundle (ARB).
//!
//! These are static text inventories. Entries are exposed as assets with a
//! key/source/target/status pair so a game's text pipeline can be traced,
//! diffed and joined to design documents. No translation service or runtime
//! locale resolution is involved.

use crate::model::{Asset, AssetEdge, FileParse};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MAX_ENTRIES: usize = 5000;

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn base_name(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn new_res(rel: &str, root: &str, lang: &str) -> FileParse {
    FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        ..Default::default()
    }
}

fn push_file(
    res: &mut FileParse,
    rel: &str,
    root: &str,
    name: &str,
    meta: HashMap<String, String>,
) -> String {
    let key = file_key(root, rel);
    res.assets.push(Asset {
        key: key.clone(),
        root: root.to_string(),
        kind: "localization".to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 0,
        meta,
    });
    key
}

fn push_entry(
    res: &mut FileParse,
    root: &str,
    rel: &str,
    key: &str,
    source: &str,
    target: &str,
    status: &str,
    language: &str,
    format: &str,
    line: i32,
) {
    let entry_key = if language.is_empty() {
        format!("asset:{root}:{rel}#localization_entry:{key}")
    } else {
        format!("asset:{root}:{rel}#localization_entry:{key}:{language}")
    };
    let mut meta = HashMap::new();
    meta.insert("key".into(), key.to_string());
    meta.insert("source".into(), source.to_string());
    meta.insert("target".into(), target.to_string());
    meta.insert("status".into(), status.to_string());
    meta.insert("language".into(), language.to_string());
    meta.insert("format".into(), format.to_string());
    res.assets.push(Asset {
        key: entry_key.clone(),
        root: root.to_string(),
        kind: "localization_entry".to_string(),
        path: rel.to_string(),
        name: key.to_string(),
        unity_id: String::new(),
        line,
        meta,
    });
    res.asset_edges.push(AssetEdge {
        src: file_key(root, rel),
        dst: entry_key,
        kind: "contains".to_string(),
        detail: "localization_entry".to_string(),
        root: root.to_string(),
        line,
    });
}

fn entry_status(target: &str, state: &str, fuzzy: bool) -> &'static str {
    if fuzzy {
        return "fuzzy";
    }
    let lowered = state.to_ascii_lowercase();
    if target.trim().is_empty() || lowered.contains("needs") || lowered.contains("new") {
        return "untranslated";
    }
    "translated"
}

fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];
        let Some(semi) = tail.find(';').filter(|s| *s <= 10) else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..semi];
        let replacement = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| {
                    entity
                        .strip_prefix('#')
                        .and_then(|dec| dec.parse::<u32>().ok())
                })
                .and_then(char::from_u32),
        };
        match replacement {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=semi]),
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
    out
}

fn parse_po(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    #[derive(Default)]
    struct Entry {
        ctx: String,
        id: String,
        id_plural: String,
        strs: Vec<String>,
        fuzzy: bool,
        line: i32,
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Field {
        Ctx,
        Id,
        Plural,
        Str,
    }

    let mut res = new_res(rel, root, lang);
    let mut language = String::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut cur = Entry::default();
    let mut field: Option<Field> = None;

    let flush = |cur: &mut Entry, entries: &mut Vec<Entry>| {
        if !cur.id.is_empty() || !cur.ctx.is_empty() || !cur.strs.is_empty() {
            entries.push(std::mem::take(cur));
        }
    };

    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut cur, &mut entries);
            field = None;
            continue;
        }
        if line.starts_with('#') {
            if line.starts_with("#,") && line.contains("fuzzy") {
                cur.fuzzy = true;
            }
            continue;
        }
        if let Some(v) = line.strip_prefix("msgctxt ") {
            cur.ctx = po_unquote(v);
            field = Some(Field::Ctx);
        } else if let Some(v) = line.strip_prefix("msgid_plural ") {
            cur.id_plural = po_unquote(v);
            field = Some(Field::Plural);
        } else if let Some(v) = line.strip_prefix("msgid ") {
            cur.id = po_unquote(v);
            cur.line = i as i32 + 1;
            field = Some(Field::Id);
        } else if let Some(v) = line.strip_prefix("msgstr ") {
            cur.strs = vec![po_unquote(v)];
            field = Some(Field::Str);
        } else if let Some(v) = line.strip_prefix("msgstr[") {
            let idx = v
                .split(']')
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let value = v.split_once(']').map(|x| x.1).unwrap_or("").trim();
            let decoded = po_unquote(value);
            while cur.strs.len() <= idx {
                cur.strs.push(String::new());
            }
            cur.strs[idx] = decoded;
            field = Some(Field::Str);
        } else if line.starts_with('"') {
            let decoded = po_unquote(line);
            match field {
                Some(Field::Ctx) => cur.ctx.push_str(&decoded),
                Some(Field::Id) => cur.id.push_str(&decoded),
                Some(Field::Plural) => cur.id_plural.push_str(&decoded),
                Some(Field::Str) => {
                    if let Some(last) = cur.strs.last_mut() {
                        last.push_str(&decoded);
                    }
                }
                None => {}
            }
        }
    }
    flush(&mut cur, &mut entries);

    let mut translated = 0usize;
    let mut untranslated = 0usize;
    let mut fuzzy = 0usize;
    let mut emitted = 0usize;
    for entry in &entries {
        // The header entry carries metadata such as Language and Plural-Forms.
        if entry.id.is_empty() && entry.ctx.is_empty() {
            for value in &entry.strs {
                for line in value.lines() {
                    if let Some(rest) = line.trim().strip_prefix("Language:") {
                        language = rest.trim().to_string();
                    }
                }
            }
            continue;
        }
        if emitted >= MAX_ENTRIES {
            continue;
        }
        let key = if entry.ctx.is_empty() {
            &entry.id
        } else {
            &entry.ctx
        };
        let target = entry.strs.first().cloned().unwrap_or_default();
        let source = if entry.id_plural.is_empty() {
            entry.id.clone()
        } else {
            format!("{} (plural: {})", entry.id, entry.id_plural)
        };
        let status = entry_status(&target, "", entry.fuzzy);
        match status {
            "translated" => translated += 1,
            "untranslated" => untranslated += 1,
            "fuzzy" => fuzzy += 1,
            _ => {}
        }
        push_entry(
            &mut res,
            root,
            rel,
            key,
            &source,
            &target,
            status,
            &language,
            "po",
            entry.line,
        );
        emitted += 1;
    }

    let mut meta = HashMap::new();
    meta.insert("format".into(), "po".to_string());
    meta.insert("language".into(), language);
    meta.insert("entries".into(), emitted.to_string());
    meta.insert("translated".into(), translated.to_string());
    meta.insert("untranslated".into(), untranslated.to_string());
    meta.insert("fuzzy".into(), fuzzy.to_string());
    if entries.len() > MAX_ENTRIES {
        meta.insert("truncated".into(), "true".to_string());
    }
    push_file(&mut res, rel, root, &base_name(rel), meta);
    res
}

fn po_unquote(value: &str) -> String {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Default)]
struct XliffEntry {
    id: String,
    source: String,
    target: String,
    state: String,
    line: i32,
}

fn parse_xliff(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    let attrs = Regex::new(r#"([A-Za-z_:][A-Za-z0-9_.:-]*)\s*=\s*"([^"]*)""#).unwrap();
    let mut res = new_res(rel, root, lang);
    let mut language = String::new();
    let mut entries: Vec<XliffEntry> = Vec::new();
    let mut cur: Option<XliffEntry> = None;
    let mut capture_source = false;
    let mut capture_target = false;
    let mut buf = String::new();
    let mut i = 0usize;
    let mut line = 1i32;

    let flush = |cur: &mut Option<XliffEntry>, entries: &mut Vec<XliffEntry>| {
        if let Some(entry) = cur.take() {
            if !entry.id.is_empty() || !entry.source.is_empty() {
                entries.push(entry);
            }
        }
    };

    while i < text.len() {
        if text.as_bytes()[i] == b'<' {
            let Some(end_rel) = text[i..].find('>') else {
                break;
            };
            let end = i + end_rel;
            let tag = &text[i + 1..end];
            let self_closing = tag.trim_end().ends_with('/');
            let body = tag.trim_start_matches('/').trim_end_matches('/').trim();
            let name = body
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if tag.starts_with('?') || tag.starts_with('!') {
                if tag.starts_with('?') {
                    if let Some(decl_end) = text[i..].find("?>") {
                        line += text[i..i + decl_end].matches('\n').count() as i32;
                        i += decl_end + 2;
                        continue;
                    }
                }
                if let Some(comment_end) = text[i..].find("-->") {
                    line += text[i..i + comment_end].matches('\n').count() as i32;
                    i += comment_end + 3;
                    continue;
                }
            } else if tag.starts_with('/') {
                match name.as_str() {
                    "source" => {
                        if let Some(entry) = cur.as_mut() {
                            entry.source = buf.trim().to_string();
                        }
                        capture_source = false;
                        buf.clear();
                    }
                    "target" => {
                        if let Some(entry) = cur.as_mut() {
                            entry.target = buf.trim().to_string();
                        }
                        capture_target = false;
                        buf.clear();
                    }
                    "trans-unit" | "unit" => {
                        flush(&mut cur, &mut entries);
                        capture_source = false;
                        capture_target = false;
                        buf.clear();
                    }
                    _ => {}
                }
            } else {
                let attr = |key: &str| -> Option<String> {
                    attrs
                        .captures_iter(body)
                        .find(|c| {
                            c.get(1).map(|m| m.as_str()) == Some(key)
                        })
                        .and_then(|c| c.get(2).map(|m| decode_entities(m.as_str())))
                };
                match name.as_str() {
                    "file" => {
                        if let Some(v) = attr("target-language").or_else(|| attr("src-lang")) {
                            language = v;
                        }
                    }
                    "trans-unit" | "unit" => {
                        flush(&mut cur, &mut entries);
                        if entries.len() + usize::from(cur.is_some()) < MAX_ENTRIES {
                            let id = attr("id")
                                .or_else(|| attr("resname"))
                                .unwrap_or_default();
                            let state = attr("state").unwrap_or_default();
                            cur = Some(XliffEntry {
                                id,
                                state,
                                line,
                                ..Default::default()
                            });
                        }
                        capture_source = false;
                        capture_target = false;
                        buf.clear();
                    }
                    "source" if cur.is_some() => {
                        capture_source = !self_closing;
                        capture_target = false;
                        buf.clear();
                    }
                    "target" if cur.is_some() => {
                        capture_target = !self_closing;
                        capture_source = false;
                        buf.clear();
                    }
                    _ => {}
                }
            }
            line += text[i..=end].matches('\n').count() as i32;
            i = end + 1;
        } else {
            let next = text[i..].find('<').map(|p| i + p).unwrap_or(text.len());
            if capture_source || capture_target {
                buf.push_str(&decode_entities(&text[i..next]));
            }
            line += text[i..next].matches('\n').count() as i32;
            i = next;
        }
    }
    flush(&mut cur, &mut entries);

    let mut translated = 0usize;
    let mut untranslated = 0usize;
    for entry in &entries {
        let status = entry_status(&entry.target, &entry.state, false);
        if status == "untranslated" {
            untranslated += 1;
        } else {
            translated += 1;
        }
        let key = if entry.id.is_empty() {
            entry.source.clone()
        } else {
            entry.id.clone()
        };
        push_entry(
            &mut res,
            root,
            rel,
            &key,
            &entry.source,
            &entry.target,
            status,
            &language,
            "xliff",
            entry.line,
        );
    }

    let mut meta = HashMap::new();
    meta.insert("format".into(), "xliff".to_string());
    meta.insert("language".into(), language);
    meta.insert("entries".into(), entries.len().to_string());
    meta.insert("translated".into(), translated.to_string());
    meta.insert("untranslated".into(), untranslated.to_string());
    if entries.len() >= MAX_ENTRIES {
        meta.insert("truncated".into(), "true".to_string());
    }
    push_file(&mut res, rel, root, &base_name(rel), meta);
    res
}

fn language_from_resx_name(rel: &str) -> String {
    let stem = base_name(rel);
    if let Some((_, tail)) = stem.rsplit_once('.') {
        let looks_like_locale = (2..=16).contains(&tail.len())
            && tail
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && tail
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-');
        if looks_like_locale {
            return tail.to_string();
        }
    }
    String::new()
}

fn parse_resx(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    let data = Regex::new(
        r#"(?s)<data\s+[^>]*name\s*=\s*"([^"]+)"[^>]*>\s*<value>(.*?)</value>"#,
    )
    .unwrap();
    let mut res = new_res(rel, root, lang);
    let language = language_from_resx_name(rel);
    let mut entries = 0usize;
    let mut translated = 0usize;
    let mut untranslated = 0usize;
    for (idx, cap) in data.captures_iter(text).enumerate() {
        if idx >= MAX_ENTRIES {
            break;
        }
        let key = decode_entities(cap.get(1).map(|m| m.as_str()).unwrap_or(""));
        let value = decode_entities(cap.get(2).map(|m| m.as_str()).unwrap_or(""));
        let status = entry_status(&value, "", false);
        if status == "untranslated" {
            untranslated += 1;
        } else {
            translated += 1;
        }
        let line = cap
            .get(0)
            .map(|m| text[..m.start()].matches('\n').count() as i32 + 1)
            .unwrap_or(0);
        push_entry(
            &mut res,
            root,
            rel,
            &key,
            "",
            &value,
            status,
            &language,
            "resx",
            line,
        );
        entries += 1;
    }
    let mut meta = HashMap::new();
    meta.insert("format".into(), "resx".to_string());
    meta.insert("language".into(), language);
    meta.insert("entries".into(), entries.to_string());
    meta.insert("translated".into(), translated.to_string());
    meta.insert("untranslated".into(), untranslated.to_string());
    push_file(&mut res, rel, root, &base_name(rel), meta);
    res
}

fn parse_arb(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    let mut res = new_res(rel, root, lang);
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            res.errors
                .push(format!("ARB JSON is invalid: {e}"));
            return res;
        }
    };
    let Some(map) = value.as_object() else {
        res.errors.push("ARB root is not a JSON object".to_string());
        return res;
    };
    let language = map
        .get("@@locale")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut entries = 0usize;
    let mut translated = 0usize;
    let mut untranslated = 0usize;
    for (key, val) in map {
        if key.starts_with('@') {
            continue;
        }
        if entries >= MAX_ENTRIES {
            break;
        }
        let target = val.as_str().unwrap_or("");
        let status = entry_status(target, "", false);
        if status == "untranslated" {
            untranslated += 1;
        } else {
            translated += 1;
        }
        push_entry(
            &mut res,
            root,
            rel,
            key,
            "",
            target,
            status,
            &language,
            "arb",
            0,
        );
        entries += 1;
    }
    let mut meta = HashMap::new();
    meta.insert("format".into(), "arb".to_string());
    meta.insert("language".into(), language);
    meta.insert("entries".into(), entries.to_string());
    meta.insert("translated".into(), translated.to_string());
    meta.insert("untranslated".into(), untranslated.to_string());
    push_file(&mut res, rel, root, &base_name(rel), meta);
    res
}

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    let text = String::from_utf8_lossy(data);
    match lang {
        "localization_po" => parse_po(rel, root, lang, &text),
        "localization_xliff" => parse_xliff(rel, root, lang, &text),
        "localization_resx" => parse_resx(rel, root, lang, &text),
        "localization_arb" => parse_arb(rel, root, lang, &text),
        "localization_csv" => parse_csv(rel, root, lang, &text),
        _ => {
            let mut res = new_res(rel, root, lang);
            res.errors
                .push(format!("unsupported localization frontend: {lang}"));
            res
        }
    }
}

/// Split one CSV record, honoring double quotes and escaped quotes.
fn csv_cells(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            }
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Unity Localization / Unreal CSV export:
/// `Key,SourceString,en,de` (a source column is optional; every other column
/// is treated as a locale).
fn parse_csv(rel: &str, root: &str, lang: &str, text: &str) -> FileParse {
    let mut res = new_res(rel, root, lang);
    let mut lines = text
        .lines()
        .map(|l| l.trim_start_matches('\u{feff}'))
        .filter(|l| !l.trim().is_empty());
    let header = match lines.next() {
        Some(h) => csv_cells(h),
        None => {
            res.errors.push("localization csv is empty".into());
            return res;
        }
    };
    if header.len() < 2 {
        res.errors
            .push("localization csv needs a key column and at least one locale".into());
        return res;
    }
    let mut source_col: Option<usize> = None;
    let mut locales: Vec<(usize, String)> = Vec::new();
    for (i, cell) in header.iter().enumerate().skip(1) {
        let name = cell.trim();
        let lower = name.to_ascii_lowercase();
        if lower == "source" || lower == "sourcestring" || lower == "source_string" || lower == "native" {
            source_col = Some(i);
            continue;
        }
        if lower.is_empty()
            || lower.contains("comment")
            || lower.contains("note")
            || lower.contains("description")
        {
            continue;
        }
        locales.push((i, name.to_string()));
    }
    if locales.is_empty() {
        res.errors
            .push("localization csv has no locale columns".into());
        return res;
    }

    let mut entries = 0usize;
    let mut translated = 0usize;
    let mut untranslated = 0usize;
    for (line_no, line) in lines.enumerate() {
        if entries >= MAX_ENTRIES {
            res.errors.push(format!(
                "localization csv truncated at {MAX_ENTRIES} entries"
            ));
            break;
        }
        let cells = csv_cells(line);
        let key = cells.first().map(|s| s.trim()).unwrap_or("");
        if key.is_empty() {
            continue;
        }
        let source = source_col
            .and_then(|i| cells.get(i))
            .map(|s| s.trim())
            .unwrap_or("");
        for (i, locale) in &locales {
            if entries >= MAX_ENTRIES {
                break;
            }
            let target = cells.get(*i).map(|s| s.trim()).unwrap_or("");
            let status = if target.is_empty() {
                untranslated += 1;
                "untranslated"
            } else {
                translated += 1;
                "translated"
            };
            push_entry(
                &mut res,
                root,
                rel,
                key,
                source,
                target,
                status,
                locale,
                "csv",
                line_no as i32 + 2,
            );
            entries += 1;
        }
    }
    if entries == 0 {
        res.errors
            .push("localization csv has no usable rows".into());
    }
    let mut meta = HashMap::new();
    meta.insert("format".into(), "csv".to_string());
    meta.insert("entries".into(), entries.to_string());
    meta.insert("translated".into(), translated.to_string());
    meta.insert("untranslated".into(), untranslated.to_string());
    if let Some(i) = source_col {
        meta.insert(
            "source_column".into(),
            header.get(i).cloned().unwrap_or_default(),
        );
    }
    push_file(&mut res, rel, root, &base_name(rel), meta);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_kind(fp: &FileParse, kind: &str) -> bool {
        fp.assets.iter().any(|a| a.kind == kind)
    }

    fn entries(fp: &FileParse) -> Vec<&Asset> {
        fp.assets
            .iter()
            .filter(|a| a.kind == "localization_entry")
            .collect()
    }

    #[test]
    fn po_extracts_entries_language_and_fuzzy_status() {
        let src = "msgid \"\"\nmsgstr \"\"\n\"Language: fr\\n\"\n\nmsgctxt \"MENU_PLAY\"\nmsgid \"Play\"\nmsgstr \"Jouer\"\n\nmsgid \"Quit\"\nmsgstr \"\"\n\n#, fuzzy\nmsgid \"Help\"\nmsgstr \"Aide\"\n";
        let fp = parse_po("Loc/Strings.po", "app", "localization_po", src);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let file = fp
            .assets
            .iter()
            .find(|a| a.kind == "localization")
            .unwrap();
        assert_eq!(file.meta.get("language").map(|s| s.as_str()), Some("fr"));
        assert_eq!(file.meta.get("entries").map(|s| s.as_str()), Some("3"));
        let entries = entries(&fp);
        assert_eq!(entries.len(), 3);
        let play = entries.iter().find(|a| a.name == "MENU_PLAY").unwrap();
        assert_eq!(play.meta.get("target").map(|s| s.as_str()), Some("Jouer"));
        assert_eq!(play.meta.get("status").map(|s| s.as_str()), Some("translated"));
        let quit = entries.iter().find(|a| a.name == "Quit").unwrap();
        assert_eq!(
            quit.meta.get("status").map(|s| s.as_str()),
            Some("untranslated")
        );
        let help = entries.iter().find(|a| a.name == "Help").unwrap();
        assert_eq!(help.meta.get("status").map(|s| s.as_str()), Some("fuzzy"));
    }

    #[test]
    fn xliff_extracts_units_states_and_language() {
        let src = r#"<?xml version="1.0"?>
<xliff version="1.2">
 <file source-language="en" target-language="de" original="game">
  <body>
   <trans-unit id="MENU_START"><source>Start</source><target>Start</target></trans-unit>
   <trans-unit id="MENU_QUIT" state="needs-translation"><source>Quit</source><target></target></trans-unit>
  </body>
 </file>
</xliff>"#;
        let fp = parse_xliff("Loc/Strings.xliff", "app", "localization_xliff", src);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_kind(&fp, "localization"));
        let entries = entries(&fp);
        assert_eq!(entries.len(), 2);
        let quit = entries.iter().find(|a| a.name == "MENU_QUIT").unwrap();
        assert_eq!(
            quit.meta.get("status").map(|s| s.as_str()),
            Some("untranslated")
        );
        let file = fp.assets.iter().find(|a| a.kind == "localization").unwrap();
        assert_eq!(file.meta.get("language").map(|s| s.as_str()), Some("de"));
    }

    #[test]
    fn resx_and_arb_extract_entries_and_locale() {
        let resx = r#"<root><data name="MENU_HELP" xml:space="preserve"><value>Hilfe &amp; Tipps</value></data></root>"#;
        let fp = parse_resx("Loc/Strings.de.resx", "app", "localization_resx", resx);
        let file = fp.assets.iter().find(|a| a.kind == "localization").unwrap();
        assert_eq!(file.meta.get("language").map(|s| s.as_str()), Some("de"));
        let entry = entries(&fp)[0];
        assert_eq!(entry.meta.get("target").map(|s| s.as_str()), Some("Hilfe & Tipps"));

        let arb = r#"{"@@locale":"ja","MENU_HELP":"ヘルプ","@MENU_HELP":{"description":"help"}}"#;
        let fp = parse_arb("Loc/strings.arb", "app", "localization_arb", arb);
        let file = fp.assets.iter().find(|a| a.kind == "localization").unwrap();
        assert_eq!(file.meta.get("language").map(|s| s.as_str()), Some("ja"));
        assert_eq!(entries(&fp).len(), 1);
    }

    #[test]
    fn bad_arb_and_partial_language_names_are_reported() {
        let fp = parse_arb("Loc/bad.arb", "app", "localization_arb", "{");
        assert!(fp.errors.iter().any(|e| e.contains("ARB JSON is invalid")));
        assert_eq!(language_from_resx_name("Loc/Strings.fr-CA.resx"), "fr-CA");
        assert_eq!(language_from_resx_name("Loc/Strings.resx"), "");
    }

    #[test]
    fn csv_extracts_locales_source_and_quoted_values() {
        let src = "Key,SourceString,en,de,Comment\nMENU_PLAY,Play,Play,\"Spielen, jetzt\",button\nMENU_QUIT,Quit,Quit,,\n";
        let fp = parse_csv("Localization/Strings.csv", "app", "localization_csv", src);
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        let file = fp.assets.iter().find(|a| a.kind == "localization").unwrap();
        assert_eq!(file.meta.get("entries").map(|s| s.as_str()), Some("4"));
        assert_eq!(file.meta.get("translated").map(|s| s.as_str()), Some("3"));
        assert_eq!(file.meta.get("untranslated").map(|s| s.as_str()), Some("1"));
        let entries = entries(&fp);
        let de = entries
            .iter()
            .find(|a| a.name == "MENU_PLAY" && a.meta["language"] == "de")
            .unwrap();
        assert_eq!(de.meta.get("target").map(|s| s.as_str()), Some("Spielen, jetzt"));
        assert_eq!(de.meta.get("source").map(|s| s.as_str()), Some("Play"));
        let quit_de = entries
            .iter()
            .find(|a| a.name == "MENU_QUIT" && a.meta["language"] == "de")
            .unwrap();
        assert_eq!(
            quit_de.meta.get("status").map(|s| s.as_str()),
            Some("untranslated")
        );
    }
}
