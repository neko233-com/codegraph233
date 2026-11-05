//! Go front-end: package/type/func/method scanning with import-aware receivers.

use crate::lexer::{last_segment, Cursor, Options};
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::{HashMap, HashSet};

const KEYWORDS: &[&str] = &[
    "if", "for", "range", "switch", "select", "case", "return", "go", "defer", "chan",
    "func", "var", "const", "type", "struct", "interface", "map", "make", "new", "len",
    "cap", "append", "copy", "delete", "panic", "recover", "print", "println",
];

pub fn parse(rel: &str, root: &str, module_path: &str, rel_dir: &str, src: &[u8]) -> FileParse {
    let mut p = Parser {
        c: Cursor::new(src, Options { backtick_strings: true, ..Default::default() }),
        res: FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: "go".into(),
            module: rel_dir.to_string(),
            ..Default::default()
        },
        pkg: String::new(),
        imports: HashMap::new(),
        field_types: HashMap::new(),
        module_path: module_path.to_string(),
        rel_dir: rel_dir.to_string(),
        seen: HashSet::new(),
    };
    p.run();
    p.res
}

struct Parser<'a> {
    c: Cursor<'a>,
    res: FileParse,
    pkg: String,
    imports: HashMap<String, String>,
    field_types: HashMap<String, String>,
    module_path: String,
    rel_dir: String,
    seen: HashSet<String>,
}

impl<'a> Parser<'a> {
    fn pkg_path(&self) -> String {
        if self.rel_dir.is_empty() || self.rel_dir == "." {
            self.module_path.clone()
        } else {
            format!("{}/{}", self.module_path.trim_end_matches('/'), self.rel_dir)
        }
    }

    fn add_symbol(&mut self, kind: &str, name: &str, qual: &str, line: i32, end: i32, sig: String, params: i32, container: &str) {
        self.res.symbols.push(Symbol {
            key: symbol_key(qual, kind, line),
            lang: "go".into(),
            kind: kind.to_string(),
            name: name.to_string(),
            qualname: qual.to_string(),
            namespace: self.pkg_path(),
            start_line: line,
            end_line: end,
            signature: sig,
            modifiers: String::new(),
            param_count: params,
            container: container.to_string(),
            module: self.rel_dir.clone(),
        });
    }

    fn add_ref(&mut self, src_key: &str, name: &str, kind: &str, rtype: &str, line: i32, arity: i32) {
        if name.is_empty() {
            return;
        }
        self.res.refs.push(Ref {
            src_key: src_key.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            receiver_type: rtype.to_string(),
            namespace_hint: self.pkg_path(),
            line,
            arity,
        });
    }

    fn run(&mut self) {
        let n = self.c.len();
        let mut i = 0usize;
        while i < n {
            if !self.c.is_ident(i) {
                i += 1;
                continue;
            }
            match self.c.text(i) {
                "package" => {
                    if self.c.is_ident(i + 1) {
                        self.pkg = self.c.text(i + 1).to_string();
                    }
                    i += 2;
                }
                "import" => {
                    i = self.parse_import(i);
                }
                "type" => {
                    i = self.parse_type(i);
                }
                "func" => {
                    i = self.parse_func(i);
                }
                _ => i += 1,
            }
        }
    }

    fn parse_import(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if self.c.is_punct(j, "(") {
            let close = self.c.match_close(j);
            j += 1;
            while j < close {
                self.read_import_spec(j);
                j += 1;
            }
            return close + 1;
        }
        self.read_import_spec(j);
        j + 1
    }

    fn read_import_spec(&mut self, j: usize) {
        // alias? "path"
        let mut k = j;
        let mut alias = String::new();
        if self.c.is_ident(k) {
            alias = self.c.text(k).to_string();
            k += 1;
        }
        if let Some(t) = self.c.toks.get(k) {
            let raw = std::str::from_utf8(&self.c.src[t.start..t.end]).unwrap_or("");
            if raw.starts_with('"') {
                let path = raw.trim_matches('"').to_string();
                if alias.is_empty() {
                    alias = path.rsplit('/').next().unwrap_or(&path).to_string();
                }
                if alias != "_" && alias != "." {
                    self.imports.insert(alias, path);
                }
            }
        }
    }

    fn parse_type(&mut self, i: usize) -> usize {
        // type Name [=] struct/interface/other { ... }
        let mut j = i + 1;
        if self.c.is_punct(j, "(") {
            return j + 1;
        }
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let line = self.c.line(j);
        j += 1;
        if self.c.is_punct(j, "=") {
            j += 1;
        }
        let kw = if self.c.is_ident(j) { self.c.text(j).to_string() } else { String::new() };
        let kind = match kw.as_str() {
            "struct" => "struct",
            "interface" => "interface",
            _ => "type",
        };
        let qual = format!("{}.{}", self.pkg_path(), name);
        let mut end_line = line;
        let mut body_open = None;
        if kind != "type" && self.c.is_punct(j + 1, "{") {
            body_open = Some(j + 1);
            end_line = self.c.line(self.c.match_close(j + 1));
        }
        self.add_symbol(kind, &name, &qual, line, end_line, format!("{kw} {name}"), -1, "");
        let key = symbol_key(&qual, kind, line);
        if let Some(open) = body_open {
            let close = self.c.match_close(open);
            if kind == "struct" {
                self.parse_struct_fields(open + 1, close, &qual, &key);
            } else {
                self.parse_interface_methods(open + 1, close, &qual);
            }
            return close + 1;
        }
        j
    }

    fn parse_struct_fields(&mut self, from: usize, to: usize, qual: &str, parent_key: &str) {
        let mut j = from;
        while j < to {
            if self.c.is_punct(j, "(") {
                j = self.c.match_close(j) + 1;
                continue;
            }
            if self.c.is_ident(j) && !KEYWORDS.contains(&self.c.text(j)) {
                let name = self.c.text(j).to_string();
                // embedded field: no following type token on same line
                let next_line = self.c.line(j + 1);
                if self.c.line(j + 1) > self.c.line(j) || j + 1 >= to {
                    // treat as embedded type
                    self.add_ref(parent_key, &last_segment(&name), "embeds", "", self.c.line(j), -1);
                    j += 1;
                    continue;
                }
                if self.c.is_ident(j + 1) {
                    let ftype = self.c.text(j + 1).to_string();
                    let fq = format!("{qual}.{name}");
                    self.add_symbol("field", &name, &fq, self.c.line(j), self.c.line(j), format!("{name} {ftype}"), -1, qual);
                    self.field_types.insert(format!("{qual}|{name}"), ftype);
                    j += 2;
                    continue;
                }
                let _ = next_line;
            }
            j += 1;
        }
    }

    fn parse_interface_methods(&mut self, from: usize, to: usize, qual: &str) {
        let mut j = from;
        while j < to {
            if self.c.is_ident(j) && self.c.is_punct(j + 1, "(") {
                let name = self.c.text(j).to_string();
                let close = self.c.match_close(j + 1);
                let mq = format!("{qual}.{name}");
                self.add_symbol(
                    "method", &name, &mq, self.c.line(j), self.c.line(close),
                    self.c.raw_text(j, close + 1), count_params(&self.c, j + 1, close), qual,
                );
                j = close + 1;
                continue;
            }
            j += 1;
        }
    }

    fn parse_func(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        let mut recv_type = String::new();
        let mut vars: HashMap<String, String> = HashMap::new();
        if self.c.is_punct(j, "(") {
            let close = self.c.match_close(j);
            // receiver: (u *Unit) or (u Unit)
            let mut k = j + 1;
            while k < close {
                if self.c.is_ident(k) && self.c.is_ident(k + 1) {
                    let var = self.c.text(k).to_string();
                    let ty = self.c.text(k + 1).trim_start_matches('*').to_string();
                    recv_type = ty.clone();
                    vars.insert(var, ty);
                } else if self.c.is_ident(k) {
                    recv_type = self.c.text(k).trim_start_matches('*').to_string();
                }
                k += 1;
            }
            j = close + 1;
        }
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let line = self.c.line(j);
        let qual = if recv_type.is_empty() {
            format!("{}.{}", self.pkg_path(), name)
        } else {
            format!("{}.{}.{}", self.pkg_path(), recv_type, name)
        };
        let kind = if recv_type.is_empty() { "function" } else { "method" };
        let mut params_close = j;
        if self.c.is_punct(j + 1, "(") {
            params_close = self.c.match_close(j + 1);
            self.collect_param_vars(j + 2, params_close, &mut vars);
        }
        let mut body_open = None;
        let mut k = params_close + 1;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_punct(k, ";") || self.c.line(k) > self.c.line(params_close) + 3 {
                break;
            }
            k += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        let sig_end = body_open.unwrap_or(params_close + 1);
        self.add_symbol(
            kind, &name, &qual, line, end_line, self.c.raw_text(i, sig_end),
            count_params(&self.c, j + 1, params_close), &recv_type,
        );
        let key = symbol_key(&qual, kind, line);
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            self.scan_refs(o + 1, close, &key, &recv_type, &vars);
            return close + 1;
        }
        params_close + 1
    }

    fn collect_param_vars(&self, from: usize, to: usize, vars: &mut HashMap<String, String>) {
        let mut j = from;
        while j < to {
            if self.c.is_ident(j) && self.c.is_ident(j + 1) {
                vars.insert(self.c.text(j).to_string(), self.c.text(j + 1).to_string());
                j += 2;
                continue;
            }
            j += 1;
        }
    }

    fn resolve_pkg(&self, alias: &str) -> String {
        self.imports.get(alias).cloned().unwrap_or_default()
    }

    fn scan_refs(&mut self, from: usize, to: usize, src_key: &str, owner: &str, vars: &HashMap<String, String>) {
        let mut j = from;
        while j < to {
            if !self.c.is_ident(j) {
                j += 1;
                continue;
            }
            let word = self.c.text(j).to_string();
            if KEYWORDS.contains(&word.as_str()) {
                j += 1;
                continue;
            }
            if self.c.is_punct(j + 1, "(") {
                // call: local func
                let arity = count_params(&self.c, j + 1, self.c.match_close(j + 1));
                self.add_ref(src_key, &word, "calls", "", self.c.line(j), arity);
                j += 1;
                continue;
            }
            if self.c.is_punct(j + 1, ".") && self.c.is_ident(j + 2) {
                let member = self.c.text(j + 2).to_string();
                let rtype = if let Some(t) = self.imports.get(&word) {
                    t.clone()
                } else if let Some(t) = vars.get(&word) {
                    t.trim_start_matches('*').to_string()
                } else if let Some(t) = self.field_types.get(&format!("{owner}|{word}")) {
                    t.trim_start_matches('*').to_string()
                } else if word.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                    word.clone()
                } else {
                    String::new()
                };
                let kind = if self.c.is_punct(j + 3, "(") { "calls" } else { "references" };
                let arity = if kind == "calls" {
                    count_params(&self.c, j + 3, self.c.match_close(j + 3))
                } else {
                    -1
                };
                let key = format!("{src_key}|{word}|{member}|{kind}");
                if self.seen.insert(key) {
                    self.add_ref(src_key, &member, kind, &rtype, self.c.line(j), arity);
                }
                j += 2;
                continue;
            }
            // composite literal Type{
            if self.c.is_punct(j + 1, "{") && word.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                let key = format!("{src_key}|{word}|constructs");
                if self.seen.insert(key) {
                    self.add_ref(src_key, &word, "constructs", &word, self.c.line(j), -1);
                }
            }
            j += 1;
        }
        let _ = self.resolve_pkg("");
    }
}

fn count_params(c: &Cursor, open: usize, close: usize) -> i32 {
    if close <= open + 1 {
        return 0;
    }
    let mut depth = 0i32;
    let mut count = 1i32;
    let mut has = false;
    for j in open + 1..close {
        if c.toks[j].kind == crate::lexer::Kind::Ident {
            has = true;
        }
        if c.toks[j].kind != crate::lexer::Kind::Punct {
            continue;
        }
        match c.text(j) {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "," if depth == 0 => count += 1,
            _ => {}
        }
    }
    if has { count } else { 0 }
}
