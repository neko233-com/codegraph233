//! SQL schema graphs.
//!
//! SQL is read line/statement-wise: comments and string payloads are masked,
//! statements are split at top-level semicolons, and a bounded amount of DDL is
//! recognised. No database connection is opened and nothing is executed.
//!
//! The output links a schema to itself and to application code:
//! tables/columns/FKs, views and routines reading or writing tables, indexes,
//! triggers, and `queries` edges from source files that contain literal SQL.

use crate::model::{Asset, AssetEdge, FileParse};
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

const MAX_STATEMENTS: usize = 20_000;
const MAX_TABLES: usize = 500;
const MAX_COLUMNS: usize = 500;
const MAX_CODE_BYTES: usize = 1_048_576;
const MAX_CODE_REFS: usize = 100;

/// Parse one SQL source file into a schema graph.
pub fn parse(rel: &str, root: &str, _lang: &str, data: &[u8]) -> FileParse {
    let text = String::from_utf8_lossy(data);
    let masked = mask_comments(&text);
    let file_key = format!("asset:{root}:{rel}");

    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "sql".to_string(),
        module: crate::tree_langs::module_of(rel),
        ..Default::default()
    };
    let mut ctx = Ctx {
        rel,
        root,
        file_key: &file_key,
        seen_tables: HashSet::new(),
        seen_edges: HashSet::new(),
        counts: Counts::default(),
    };
    let mut statements = 0usize;

    for (start, end) in split_statements(&masked) {
        if statements >= MAX_STATEMENTS {
            break;
        }
        statements += 1;
        let raw = &text[start..end];
        let stmt = &masked[start..end];
        let line = 1 + text[..start].matches('\n').count() as i32;
        if stmt.trim().is_empty() {
            continue;
        }
        let upper = stmt.trim_start().to_ascii_uppercase();
        if upper.starts_with("CREATE TABLE")
            || upper.starts_with("CREATE TEMP")
            || upper.starts_with("CREATE TEMPORARY")
            || upper.starts_with("CREATE UNLOGGED")
            || upper.starts_with("CREATE GLOBAL")
            || upper.starts_with("CREATE LOCAL")
        {
            parse_create_table(&mut ctx, &mut res, stmt, line);
        } else if upper.starts_with("CREATE VIEW")
            || upper.starts_with("CREATE OR REPLACE VIEW")
            || upper.starts_with("CREATE MATERIALIZED VIEW")
            || upper.starts_with("CREATE OR REPLACE MATERIALIZED VIEW")
        {
            parse_create_view(&mut ctx, &mut res, stmt, raw, line);
        } else if (upper.starts_with("CREATE FUNCTION")
            || upper.starts_with("CREATE OR REPLACE FUNCTION")
            || upper.starts_with("CREATE PROCEDURE")
            || upper.starts_with("CREATE OR REPLACE PROCEDURE"))
            && ctx.counts.routines < MAX_TABLES
        {
            parse_create_routine(&mut ctx, &mut res, stmt, raw, line);
        } else if upper.starts_with("CREATE INDEX")
            || upper.starts_with("CREATE UNIQUE INDEX")
            || upper.starts_with("CREATE INDEX CONCURRENTLY")
            || upper.starts_with("CREATE UNIQUE INDEX CONCURRENTLY")
        {
            parse_create_index(&mut ctx, &mut res, stmt, line);
        } else if upper.starts_with("CREATE TRIGGER") || upper.starts_with("CREATE OR REPLACE TRIGGER") {
            parse_create_trigger(&mut ctx, &mut res, stmt, line);
        } else if upper.starts_with("ALTER TABLE") {
            parse_alter_table(&mut ctx, &mut res, stmt, raw, line);
        }
    }

    let mut meta = BTreeMap::new();
    meta.insert("format".to_string(), "sql".to_string());
    meta.insert("dialect".to_string(), dialect_of(rel).to_string());
    meta.insert("statements".to_string(), statements.to_string());
    meta.insert("tables".to_string(), ctx.counts.tables.to_string());
    meta.insert("columns".to_string(), ctx.counts.columns.to_string());
    meta.insert("views".to_string(), ctx.counts.views.to_string());
    meta.insert("indexes".to_string(), ctx.counts.indexes.to_string());
    meta.insert("routines".to_string(), ctx.counts.routines.to_string());
    meta.insert("triggers".to_string(), ctx.counts.triggers.to_string());
    meta.insert(
        "foreign_keys".to_string(),
        ctx.counts.foreign_keys.to_string(),
    );
    res.assets.insert(
        0,
        Asset {
            key: file_key,
            root: root.to_string(),
            kind: "sql_script".to_string(),
            path: rel.to_string(),
            name: base_name(rel),
            unity_id: String::new(),
            line: 1,
            meta: meta.into_iter().collect(),
        },
    );
    res
}

/// Scan a source file for literal SQL and link it to table keys.
pub fn scan_code(rel: &str, root: &str, text: &str) -> Vec<AssetEdge> {
    if text.len() > MAX_CODE_BYTES {
        return Vec::new();
    }
    let lower = text.to_ascii_lowercase();
    if !lower.contains("select")
        && !lower.contains("insert")
        && !lower.contains("update")
        && !lower.contains("delete")
        && !lower.contains("join")
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(String, i32)> = HashSet::new();
    let patterns = code_patterns();
    for re in patterns {
        for m in re.captures_iter(text) {
            if out.len() >= MAX_CODE_REFS {
                return out;
            }
            let name = &m[1];
            if RESERVED.contains(&last_lower(name).as_str()) {
                continue;
            }
            let Some(key) = table_key_checked(root, name) else {
                continue;
            };
            let line = 1 + text[..m.get(0).unwrap().start()].matches('\n').count() as i32;
            if !seen.insert((key.clone(), line)) {
                continue;
            }
            out.push(AssetEdge {
                src: format!("asset:{root}:{rel}"),
                dst: key,
                kind: "queries".to_string(),
                detail: "literal_sql".to_string(),
                root: root.to_string(),
                line,
            });
        }
    }
    out
}

#[derive(Default)]
struct Counts {
    tables: usize,
    columns: usize,
    views: usize,
    indexes: usize,
    routines: usize,
    triggers: usize,
    foreign_keys: usize,
}

struct Ctx<'a> {
    rel: &'a str,
    root: &'a str,
    file_key: &'a str,
    seen_tables: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
    counts: Counts,
}

impl Ctx<'_> {
    fn edge(&mut self, res: &mut FileParse, src: &str, dst: &str, kind: &str, detail: &str, line: i32) {
        let key = (src.to_string(), dst.to_string(), kind.to_string());
        if !self.seen_edges.insert(key) {
            return;
        }
        res.asset_edges.push(AssetEdge {
            src: src.to_string(),
            dst: dst.to_string(),
            kind: kind.to_string(),
            detail: detail.to_string(),
            root: self.root.to_string(),
            line,
        });
    }

    fn declare(&mut self, res: &mut FileParse, key: &str, kind: &str, line: i32) {
        self.edge(res, self.file_key, key, "declares", kind, line);
    }

    fn ensure_table(&mut self, res: &mut FileParse, name: &str, line: i32) -> Option<String> {
        if self.counts.tables >= MAX_TABLES {
            return None;
        }
        let key = table_key_checked(self.root, name)?;
        if self.seen_tables.insert(key.clone()) {
            let mut meta = BTreeMap::new();
            meta.insert("table".to_string(), display_name(name));
            if let Some(schema) = schema_of(name) {
                meta.insert("schema".to_string(), schema);
            }
            res.assets.push(Asset {
                key: key.clone(),
                root: self.root.to_string(),
                kind: "sql_table".to_string(),
                path: self.rel.to_string(),
                name: display_name(name),
                unity_id: String::new(),
                line,
                meta: meta.into_iter().collect(),
            });
            self.counts.tables += 1;
            self.declare(res, &key, "table", line);
        }
        Some(key)
    }
}

fn parse_create_table(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, line: i32) {
    let Some(p) = find_word(stmt, "TABLE") else {
        return;
    };
    let after_word = skip_prefix_words(stmt, p + 5, &["IF", "NOT", "EXISTS"]);
    let Some((name, after)) = read_name(stmt, after_word) else {
        return;
    };
    let Some(open) = find_char(stmt, after, '(') else {
        return;
    };
    let key = match ctx.ensure_table(res, &name, line) {
        Some(k) => k,
        None => return,
    };
    let Some(close) = matching_paren(stmt, open) else {
        res.errors
            .push(format!("SQL table {name} has an unclosed column list"));
        return;
    };
    let inside = &stmt[open + 1..close];
    let mut columns = 0usize;
    for part in split_top(inside) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let upper = part.to_ascii_uppercase();
        let constraint = upper.starts_with("CONSTRAINT")
            || upper.starts_with("PRIMARY KEY")
            || upper.starts_with("UNIQUE")
            || upper.starts_with("CHECK")
            || upper.starts_with("EXCLUDE")
            || upper.starts_with("KEY ")
            || upper.starts_with("INDEX ")
            || upper.starts_with("FOREIGN KEY");
        if constraint {
            if upper.contains("REFERENCES") {
                add_reference(ctx, res, &key, part, line, true);
            }
            continue;
        }
        if columns >= MAX_COLUMNS {
            break;
        }
        let Some((col, rest_at)) = read_name(part, 0) else {
            continue;
        };
        let rest = part[rest_at..].trim();
        let col_key = format!("{key}#column:{}", last_lower(&col));
        columns += 1;
        ctx.counts.columns += 1;
        let mut meta = BTreeMap::new();
        meta.insert("table".to_string(), display_name(&name));
        meta.insert("type".to_string(), column_type(rest));
        if upper.contains("PRIMARY KEY") {
            meta.insert("primary_key".to_string(), "1".to_string());
        }
        res.assets.push(Asset {
            key: col_key.clone(),
            root: ctx.root.to_string(),
            kind: "sql_column".to_string(),
            path: ctx.rel.to_string(),
            name: display_name(&col),
            unity_id: String::new(),
            line,
            meta: meta.into_iter().collect(),
        });
        ctx.edge(res, &key, &col_key, "contains", "column", line);
        if find_word(rest, "REFERENCES").is_some() {
            add_reference(ctx, res, &col_key, rest, line, false);
        }
    }
    if columns == 0 && !inside.trim().is_empty() {
        res.errors
            .push(format!("SQL table {name} has a column list that could not be parsed"));
    }
}

fn add_reference(ctx: &mut Ctx, res: &mut FileParse, src: &str, text: &str, line: i32, table_level: bool) {
    let Some(p) = find_word(text, "REFERENCES") else {
        return;
    };
    let Some((target, _)) = read_name(text, p + "REFERENCES".len()) else {
        return;
    };
    let Some(dst) = table_key_checked(ctx.root, &target) else {
        return;
    };
    ctx.counts.foreign_keys += 1;
    let detail = if table_level { "foreign_key" } else { "column" };
    ctx.edge(res, src, &dst, "references", detail, line);
}

fn parse_create_view(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, raw: &str, line: i32) {
    let Some(p) = find_word(stmt, "VIEW") else {
        return;
    };
    let Some((name, after)) = read_name(stmt, p + 4) else {
        return;
    };
    let Some(as_pos) = find_word(stmt, "AS") else {
        return;
    };
    let _ = after;
    let key = view_key(ctx.root, &name);
    if ctx.seen_tables.contains(&key) || ctx.counts.views >= MAX_TABLES {
        return;
    }
    ctx.seen_tables.insert(key.clone());
    ctx.counts.views += 1;
    let mut meta = BTreeMap::new();
    meta.insert("view".to_string(), display_name(&name));
    res.assets.push(Asset {
        key: key.clone(),
        root: ctx.root.to_string(),
        kind: "sql_view".to_string(),
        path: ctx.rel.to_string(),
        name: display_name(&name),
        unity_id: String::new(),
        line,
        meta: meta.into_iter().collect(),
    });
    ctx.declare(res, &key, "view", line);
    let body = &raw[as_pos + 2..];
    for (target, kind, offset) in table_refs(body) {
        let Some(dst) = table_key_checked(ctx.root, &target) else {
            continue;
        };
        let ref_line = line + raw[..as_pos + 2 + offset].matches('\n').count() as i32;
        ctx.edge(res, &key, &dst, kind, "view", ref_line);
    }
}

fn parse_create_routine(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, raw: &str, line: i32) {
    let Some(p) = find_word(stmt, "FUNCTION").or_else(|| find_word(stmt, "PROCEDURE")) else {
        return;
    };
    let word = if find_word(stmt, "FUNCTION").is_some() { "FUNCTION" } else { "PROCEDURE" };
    let Some((name, after)) = read_name(stmt, p + word.len()) else {
        return;
    };
    let key = format!("sql_routine:{}:{}", ctx.root, last_lower(&name));
    if ctx.seen_tables.contains(&key) {
        return;
    }
    ctx.seen_tables.insert(key.clone());
    ctx.counts.routines += 1;
    let mut meta = BTreeMap::new();
    meta.insert("routine".to_string(), display_name(&name));
    meta.insert("kind".to_string(), word.to_ascii_lowercase());
    res.assets.push(Asset {
        key: key.clone(),
        root: ctx.root.to_string(),
        kind: "sql_routine".to_string(),
        path: ctx.rel.to_string(),
        name: display_name(&name),
        unity_id: String::new(),
        line,
        meta: meta.into_iter().collect(),
    });
    ctx.declare(res, &key, "routine", line);
    for (target, kind, offset) in table_refs(&raw[after..]) {
        let Some(dst) = table_key_checked(ctx.root, &target) else {
            continue;
        };
        let ref_line = line + raw[..after + offset].matches('\n').count() as i32;
        ctx.edge(res, &key, &dst, kind, "routine", ref_line);
    }
}

fn parse_create_index(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, line: i32) {
    let Some(p) = find_word(stmt, "INDEX") else {
        return;
    };
    let after_word = skip_prefix_words(stmt, p + 5, &["CONCURRENTLY", "IF", "NOT", "EXISTS"]);
    let Some((name, after)) = read_name(stmt, after_word) else {
        return;
    };
    let Some(on) = find_word(stmt, "ON") else {
        return;
    };
    let Some((table, _)) = read_name(stmt, on + 2) else {
        return;
    };
    let Some(table_key) = table_key_checked(ctx.root, &table) else {
        return;
    };
    let key = format!("sql_index:{}:{}", ctx.root, last_lower(&name));
    if !ctx.seen_tables.insert(key.clone()) {
        return;
    }
    ctx.counts.indexes += 1;
    let mut meta = BTreeMap::new();
    meta.insert("index".to_string(), display_name(&name));
    meta.insert("table".to_string(), display_name(&table));
    res.assets.push(Asset {
        key: key.clone(),
        root: ctx.root.to_string(),
        kind: "sql_index".to_string(),
        path: ctx.rel.to_string(),
        name: display_name(&name),
        unity_id: String::new(),
        line,
        meta: meta.into_iter().collect(),
    });
    ctx.declare(res, &key, "index", line);
    let _ = after;
    ctx.edge(res, &key, &table_key, "indexed_on", "index", line);
}

fn parse_create_trigger(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, line: i32) {
    let Some(p) = find_word(stmt, "TRIGGER") else {
        return;
    };
    let after_word = skip_prefix_words(stmt, p + 7, &["IF", "NOT", "EXISTS"]);
    let Some((name, _after)) = read_name(stmt, after_word) else {
        return;
    };
    let Some(on) = find_word(stmt, "ON") else {
        return;
    };
    let Some((table, _)) = read_name(stmt, on + 2) else {
        return;
    };
    let Some(table_key) = table_key_checked(ctx.root, &table) else {
        return;
    };
    let key = format!("sql_trigger:{}:{}", ctx.root, last_lower(&name));
    if !ctx.seen_tables.insert(key.clone()) {
        return;
    }
    ctx.counts.triggers += 1;
    let mut meta = BTreeMap::new();
    meta.insert("trigger".to_string(), display_name(&name));
    meta.insert("table".to_string(), display_name(&table));
    res.assets.push(Asset {
        key: key.clone(),
        root: ctx.root.to_string(),
        kind: "sql_trigger".to_string(),
        path: ctx.rel.to_string(),
        name: display_name(&name),
        unity_id: String::new(),
        line,
        meta: meta.into_iter().collect(),
    });
    ctx.declare(res, &key, "trigger", line);
    ctx.edge(res, &key, &table_key, "triggered_on", "trigger", line);
}

fn parse_alter_table(ctx: &mut Ctx, res: &mut FileParse, stmt: &str, raw: &str, line: i32) {
    let Some(p) = find_word(stmt, "TABLE") else {
        return;
    };
    let Some((name, _)) = read_name(stmt, p + 5) else {
        return;
    };
    let Some(key) = table_key_checked(ctx.root, &name) else {
        return;
    };
    let mut offset = 0usize;
    while let Some(pos) = find_word(&raw[offset..], "REFERENCES") {
        let at = offset + pos;
        if let Some((target, _)) = read_name(raw, at + "REFERENCES".len()) {
            if let Some(dst) = table_key_checked(ctx.root, &target) {
                ctx.counts.foreign_keys += 1;
                let ref_line = line + raw[..at].matches('\n').count() as i32;
                ctx.edge(res, &key, &dst, "references", "alter_table", ref_line);
            }
        }
        offset = at + "REFERENCES".len();
    }
}

fn table_refs(raw: &str) -> Vec<(String, &'static str, usize)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)\b(from|join|update|into)\s+([A-Za-z_][A-Za-z0-9_$.]*)").unwrap()
    });
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in re.captures_iter(raw) {
        let keyword = m[1].to_ascii_lowercase();
        let name = m[2].to_string();
        let key = last_lower(&name);
        if RESERVED.contains(&key.as_str()) {
            continue;
        }
        let kind = if keyword == "update" || keyword == "into" {
            "writes"
        } else {
            "reads"
        };
        if !seen.insert((key, kind)) {
            continue;
        }
        let offset = m.get(0).unwrap().start();
        out.push((name, kind, offset));
    }
    out
}

fn code_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"(?is)\bselect\b.{0,240}?\bfrom\s+([A-Za-z_][A-Za-z0-9_$.]*)").unwrap(),
            Regex::new(r"(?i)\b(?:insert\s+into|delete\s+from)\s+([A-Za-z_][A-Za-z0-9_$.]*)")
                .unwrap(),
            Regex::new(r"(?i)\bjoin\s+([A-Za-z_][A-Za-z0-9_$.]*)").unwrap(),
            Regex::new(r"(?i)\bupdate\s+([A-Za-z_][A-Za-z0-9_$.]*)\s+set\b").unwrap(),
        ]
    })
}

const RESERVED: &[&str] = &[
    "select", "where", "set", "values", "value", "dual", "lateral", "unnest", "only", "table",
    "returning", "default", "null", "true", "false", "case", "when", "then", "else", "end",
    "union", "all", "distinct", "group", "order", "limit", "offset", "having", "window",
];

fn dialect_of(rel: &str) -> &'static str {
    match ext_of(rel).as_str() {
        "psql" | "pgsql" => "postgresql",
        "mysql" => "mysql",
        "sqlite" | "sqlite3" => "sqlite",
        "tsql" | "mssql" => "mssql",
        "plsql" | "pkb" | "pks" => "oracle",
        "cql" => "cassandra",
        _ => "sql",
    }
}

fn table_key_checked(root: &str, name: &str) -> Option<String> {
    let last = last_lower(name);
    if last.is_empty() {
        return None;
    }
    Some(format!("sql_table:{root}:{last}"))
}

fn view_key(root: &str, name: &str) -> String {
    format!("sql_view:{root}:{}", last_lower(name))
}

fn last_lower(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).trim().to_ascii_lowercase()
}

fn schema_of(name: &str) -> Option<String> {
    let (schema, _) = name.rsplit_once('.')?;
    Some(schema.trim_matches('"').to_string())
}

fn display_name(name: &str) -> String {
    name.trim().to_string()
}

fn base_name(rel: &str) -> String {
    std::path::Path::new(rel)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

fn ext_of(rel: &str) -> String {
    std::path::Path::new(rel)
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Mask comments and single-quoted/dollar-quoted string payloads while keeping
/// identifiers (including `"..."`, backticks and `[...]`) and newlines intact.
fn mask_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'\'' => {
                i += 1;
                while i < b.len() {
                    if b[i] == b'\'' {
                        if i + 1 < b.len() && b[i + 1] == b'\'' {
                            out[i] = b' ';
                            out[i + 1] = b' ';
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if i + 1 < b.len() && b[i + 1] == b'-' => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                while i + 1 < b.len() {
                    if b[i] == b'*' && b[i + 1] == b'/' {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            b'$' => {
                if let Some(tag_end) = dollar_tag_end(text, i) {
                    let tag = &text[i..=tag_end];
                    if let Some(close) = text[tag_end + 1..].find(tag) {
                        let body_start = tag_end + 1;
                        let body_end = body_start + close;
                        for k in body_start..body_end {
                            if out[k] != b'\n' {
                                out[k] = b' ';
                            }
                        }
                        i = body_end + tag.len();
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn dollar_tag_end(text: &str, start: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut i = start + 1;
    while i < b.len() {
        if b[i] == b'$' {
            return Some(i);
        }
        if !(b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            return None;
        }
        i += 1;
    }
    None
}

fn split_statements(masked: &str) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => depth = (depth - 1).max(0),
            b';' if depth == 0 => {
                if masked[start..i].trim().len() > 1 {
                    out.push((start, i));
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < b.len() && masked[start..].trim().len() > 1 {
        out.push((start, b.len()));
    }
    out
}

fn find_word(s: &str, word: &str) -> Option<usize> {
    let up = s.to_ascii_uppercase();
    let w = word.to_ascii_uppercase();
    let bytes = up.as_bytes();
    let mut from = 0usize;
    while let Some(p) = up[from..].find(&w) {
        let i = from + p;
        let before = i == 0 || !is_ident(bytes[i - 1]);
        let after = i + w.len() >= bytes.len() || !is_ident(bytes[i + w.len()]);
        if before && after {
            return Some(i);
        }
        from = i + 1;
    }
    None
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn skip_ws(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Skip an optional sequence of leading keywords (e.g. `IF NOT EXISTS`).
fn skip_prefix_words(s: &str, i: usize, words: &[&str]) -> usize {
    let mut pos = skip_ws(s, i);
    loop {
        let mut advanced = false;
        for word in words {
            let end = pos + word.len();
            if end <= s.len()
                && s[pos..end].eq_ignore_ascii_case(word)
                && (end == s.len() || !is_ident(s.as_bytes()[end]))
            {
                pos = skip_ws(s, end);
                advanced = true;
                break;
            }
        }
        if !advanced {
            return pos;
        }
    }
}

fn find_char(s: &str, mut i: usize, target: char) -> Option<usize> {
    let b = s.as_bytes();
    while i < b.len() {
        if b[i] == target as u8 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top(inside: &str) -> Vec<&str> {
    let b = inside.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => depth = (depth - 1).max(0),
            b',' if depth == 0 => {
                out.push(&inside[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&inside[start..]);
    out
}

fn read_name(s: &str, i: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut pos = i;
    let mut first = true;
    loop {
        pos = skip_ws(s, pos);
        if pos >= s.len() {
            break;
        }
        let (part, next) = read_ident_part(s, pos)?;
        if part.is_empty() {
            break;
        }
        if !first {
            out.push('.');
        }
        out.push_str(&part);
        first = false;
        pos = skip_ws(s, next);
        if pos < s.len() && s.as_bytes()[pos] == b'.' {
            pos += 1;
            continue;
        }
        break;
    }
    if out.is_empty() {
        None
    } else {
        Some((out, pos))
    }
}

fn read_ident_part(s: &str, i: usize) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if i >= b.len() {
        return None;
    }
    match b[i] {
        b'"' | b'`' => {
            let quote = b[i];
            let mut out = String::new();
            let mut pos = i + 1;
            while pos < b.len() {
                if b[pos] == quote {
                    if pos + 1 < b.len() && b[pos + 1] == quote {
                        out.push(quote as char);
                        pos += 2;
                        continue;
                    }
                    return Some((out, pos + 1));
                }
                out.push(b[pos] as char);
                pos += 1;
            }
            None
        }
        b'[' => {
            let mut out = String::new();
            let mut pos = i + 1;
            while pos < b.len() {
                if b[pos] == b']' {
                    return Some((out, pos + 1));
                }
                out.push(b[pos] as char);
                pos += 1;
            }
            None
        }
        _ => {
            let mut pos = i;
            while pos < b.len() && is_ident(b[pos]) {
                pos += 1;
            }
            if pos == i {
                None
            } else {
                Some((s[i..pos].to_string(), pos))
            }
        }
    }
}

fn column_type(rest: &str) -> String {
    let up = rest.to_ascii_uppercase();
    let mut cut = rest.len();
    for needle in [
        " NOT NULL",
        " NULL",
        " DEFAULT",
        " PRIMARY",
        " UNIQUE",
        " REFERENCES",
        " CHECK",
        " COLLATE",
        " GENERATED",
        " AUTO_INCREMENT",
        " AUTOINCREMENT",
        " COMMENT",
        " ON UPDATE",
    ] {
        if let Some(p) = up.find(needle) {
            cut = cut.min(p);
        }
    }
    let ty = rest[..cut].trim();
    if ty.is_empty() {
        "unknown".to_string()
    } else {
        ty.chars().take(80).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_asset(fp: &FileParse, kind: &str, name: &str) -> bool {
        fp.assets
            .iter()
            .any(|a| a.kind == kind && a.name.eq_ignore_ascii_case(name))
    }

    fn has_edge(fp: &FileParse, src: &str, dst: &str, kind: &str) -> bool {
        fp.asset_edges
            .iter()
            .any(|e| e.src.contains(src) && e.dst.contains(dst) && e.kind == kind)
    }

    #[test]
    fn ddl_extracts_tables_columns_and_foreign_keys() {
        let sql = "CREATE TABLE public.users (\n  id BIGSERIAL PRIMARY KEY,\n  email VARCHAR(255) NOT NULL,\n  org_id BIGINT REFERENCES public.orgs(id)\n);\nCREATE TABLE IF NOT EXISTS orgs (id BIGINT PRIMARY KEY, name TEXT);\n";
        let fp = parse("db/schema.sql", "app", "sql", sql.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "sql_table", "public.users"), "{:?}", fp.assets);
        assert!(has_asset(&fp, "sql_table", "orgs"));
        assert!(has_asset(&fp, "sql_column", "email"));
        assert!(has_edge(
            &fp,
            "sql_table:app:users#column:org_id",
            "sql_table:app:orgs",
            "references"
        ));
        assert!(has_edge(&fp, "sql_table:app:users", "column:email", "contains"));
    }

    #[test]
    fn views_routines_indexes_and_triggers_reference_tables() {
        let sql = "CREATE VIEW active_users AS\n  SELECT id FROM users WHERE active;\nCREATE UNIQUE INDEX idx_users_email ON users (email);\nCREATE OR REPLACE FUNCTION touch_user() RETURNS trigger AS $$\nBEGIN\n  UPDATE users SET email = email;\n  RETURN NEW;\nEND;\n$$ LANGUAGE plpgsql;\nCREATE TRIGGER trg_users BEFORE UPDATE ON users FOR EACH ROW EXECUTE FUNCTION touch_user();\n";
        let fp = parse("db/schema.sql", "app", "sql", sql.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "sql_view", "active_users"), "{:?}", fp.assets);
        assert!(has_asset(&fp, "sql_index", "idx_users_email"));
        assert!(has_asset(&fp, "sql_routine", "touch_user"));
        assert!(has_asset(&fp, "sql_trigger", "trg_users"));
        assert!(has_edge(&fp, "sql_view:app:active_users", "sql_table:app:users", "reads"));
        assert!(has_edge(&fp, "sql_routine:app:touch_user", "sql_table:app:users", "writes"));
        assert!(has_edge(&fp, "sql_index:app:idx_users_email", "sql_table:app:users", "indexed_on"));
        assert!(has_edge(&fp, "sql_trigger:app:trg_users", "sql_table:app:users", "triggered_on"));
    }

    #[test]
    fn quoted_identifiers_comments_and_strings_are_handled() {
        let sql = "-- CREATE TABLE nope (id int);\n/* CREATE TABLE also_nope (id int); */\nCREATE TABLE \"Order\" (\"ID\" int, [Name] text DEFAULT 'a;b');\n";
        let fp = parse("db/orders.sql", "app", "sql", sql.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_asset(&fp, "sql_table", "Order"), "{:?}", fp.assets);
        assert!(has_asset(&fp, "sql_column", "ID"));
        assert!(has_asset(&fp, "sql_column", "Name"));
        assert!(!has_asset(&fp, "sql_table", "nope"));
    }

    #[test]
    fn code_queries_link_to_tables() {
        let code = "var q = \"SELECT id FROM Users WHERE id = @id\";\nExec(\"INSERT INTO audit_log (id) VALUES (1)\");\n";
        let edges = scan_code("src/Db.cs", "app", code);
        assert!(
            edges
                .iter()
                .any(|e| e.dst == "sql_table:app:users" && e.kind == "queries"),
            "{edges:?}"
        );
        assert!(edges.iter().any(|e| e.dst == "sql_table:app:audit_log"));
    }
}
