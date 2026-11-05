//! Rust front-end: mod/struct/enum/trait/impl/fn/type/const with use-aliases.

use crate::lexer::{last_segment, Cursor, Options};
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::HashMap;

const KW: &[&str] = &[
    "pub", "crate", "super", "self", "async", "unsafe", "extern", "const", "static",
    "default", "move", "ref", "mut", "dyn", "impl", "where", "let", "match", "if", "for",
    "while", "loop", "return", "fn", "struct", "enum", "trait", "type", "mod", "use", "as",
];

pub fn parse(rel: &str, root: &str, src: &[u8]) -> FileParse {
    let mut p = Parser {
        c: Cursor::new(src, Options { nested_comments: true, rust_literals: true, ..Default::default() }),
        res: FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: "rust".into(),
            module: module_of(rel),
            ..Default::default()
        },
        aliases: HashMap::new(),
        seen: std::collections::HashSet::new(),
    };
    p.run();
    p.res
}

fn module_of(rel: &str) -> String {
    let rel = rel.trim_end_matches(".rs");
    let mut parts: Vec<&str> = rel.split('/').collect();
    if parts.first().map(|p| matches!(*p, "src" | "lib" | "tests" | "benches")).unwrap_or(false) {
        parts.remove(0);
    }
    if parts.last() == Some(&"mod") {
        parts.pop();
    }
    if parts.is_empty() || (parts.len() == 1 && matches!(parts[0], "main" | "lib")) {
        return "crate".into();
    }
    if matches!(parts.last(), Some(&"main") | Some(&"lib")) {
        parts.pop();
    }
    if parts.is_empty() {
        "crate".into()
    } else {
        format!("crate::{}", parts.join("::"))
    }
}

struct Parser<'a> {
    c: Cursor<'a>,
    res: FileParse,
    aliases: HashMap<String, String>,
    seen: std::collections::HashSet<String>,
}

impl<'a> Parser<'a> {
    fn add_symbol(&mut self, kind: &str, name: &str, qual: &str, line: i32, end: i32, sig: String, params: i32, container: &str) {
        self.res.symbols.push(Symbol {
            key: symbol_key(qual, kind, line),
            root: self.res.root.clone(),
            lang: "rust".into(),
            kind: kind.to_string(),
            name: name.to_string(),
            qualname: qual.to_string(),
            namespace: self.res.module.clone(),
            start_line: line,
            end_line: end,
            signature: sig,
            modifiers: String::new(),
            param_count: params,
            container: container.to_string(),
            module: self.res.module.clone(),
        });
    }

    fn add_ref(&mut self, src_key: &str, name: &str, kind: &str, rtype: &str, line: i32, arity: i32) {
        if name.is_empty() {
            return;
        }
        self.res.refs.push(Ref {
            src_key: src_key.to_string(),
            root: self.res.root.clone(),
            name: name.to_string(),
            kind: kind.to_string(),
            receiver_type: rtype.to_string(),
            namespace_hint: self.res.module.clone(),
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
                "mod" => i = self.parse_mod(i),
                "use" => i = self.parse_use(i),
                "struct" => i = self.parse_struct(i),
                "enum" => i = self.parse_enum(i),
                "trait" => i = self.parse_trait(i),
                "impl" => i = self.parse_impl(i),
                "fn" => i = self.parse_fn(i, ""),
                "type" => i = self.parse_alias(i),
                "const" | "static" => i = self.parse_const(i),
                _ => i += 1,
            }
        }
    }

    fn parse_mod(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        while j < self.c.len() && !(self.c.is_punct(j, "{") || self.c.is_punct(j, ";")) {
            j += 1;
        }
        if self.c.is_punct(j, "{") {
            let close = self.c.match_close(j);
            // nested items currently share the file module path
            let mut k = j + 1;
            while k < close {
                if self.c.is_ident(k) {
                    match self.c.text(k) {
                        "struct" => k = self.parse_struct(k),
                        "enum" => k = self.parse_enum(k),
                        "trait" => k = self.parse_trait(k),
                        "impl" => k = self.parse_impl(k),
                        "fn" => k = self.parse_fn(k, ""),
                        "mod" => k = self.parse_mod(k),
                        "use" => k = self.parse_use(k),
                        _ => k += 1,
                    }
                } else {
                    k += 1;
                }
            }
            return close + 1;
        }
        j + 1
    }

    fn parse_use(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        let end = {
            let mut k = j;
            let mut depth = 0i32;
            while k < self.c.len() {
                if self.c.is_punct(k, "{") {
                    depth += 1;
                } else if self.c.is_punct(k, "}") {
                    depth -= 1;
                } else if self.c.is_punct(k, ";") && depth == 0 {
                    break;
                }
                k += 1;
            }
            k
        };
        self.record_use(j, end, "");
        j = end;
        j + 1
    }

    fn record_use(&mut self, from: usize, to: usize, prefix: &str) {
        let mut depth = 0i32;
        let mut seg = from;
        let mut j = from;
        while j <= to {
            let at_end = j == to;
            let mut sep = false;
            if !at_end && self.c.toks[j].kind == crate::lexer::Kind::Punct {
                match self.c.text(j) {
                    "{" => depth += 1,
                    "}" => depth -= 1,
                    "," if depth == 0 => sep = true,
                    _ => {}
                }
            }
            if at_end || sep {
                self.record_use_seg(seg, j, prefix);
                seg = j + 1;
            }
            j += 1;
        }
    }

    fn record_use_seg(&mut self, from: usize, to: usize, prefix: &str) {
        let mut parts: Vec<String> = Vec::new();
        let mut alias = String::new();
        let mut group = None;
        let mut j = from;
        while j < to {
            if self.c.is_ident(j) {
                let t = self.c.text(j);
                if t == "as" {
                    if self.c.is_ident(j + 1) {
                        alias = self.c.text(j + 1).to_string();
                    }
                    j += 2;
                    continue;
                }
                if alias.is_empty() {
                    parts.push(t.to_string());
                }
            } else if self.c.is_punct(j, "{") {
                group = Some(j);
                break;
            }
            j += 1;
        }
        if let Some(g) = group {
            let close = self.c.match_close(g);
            let base = parts.join("::");
            let full_base = if prefix.is_empty() {
                base
            } else if base.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}::{base}")
            };
            self.record_use(g + 1, close, &full_base);
            return;
        }
        if parts.is_empty() {
            return;
        }
        let mut full = parts.join("::");
        if !prefix.is_empty() {
            full = format!("{prefix}::{full}");
        }
        let name = if alias.is_empty() { parts[parts.len() - 1].clone() } else { alias };
        self.aliases.insert(name, full);
    }

    fn parse_struct(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}::{}", self.res.module, name);
        let line = self.c.line(j);
        j += 1;
        if self.c.is_punct(j, "<") {
            if let Some(gt) = self.c.match_angle(j, self.c.len()) {
                j = gt + 1;
            }
        }
        let mut body_open = None;
        let mut k = j;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_punct(k, ";") {
                break;
            }
            k += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        self.add_symbol("struct", &name, &qual, line, end_line, self.c.raw_text(i, body_open.unwrap_or(k)), -1, "");
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut f = o + 1;
            while f < close {
                if self.c.is_ident(f) && self.c.is_punct(f + 1, ":") {
                    let fname = self.c.text(f).to_string();
                    let mut ft = String::new();
                    let mut x = f + 2;
                    while x < close && !self.c.is_punct(x, ",") && !self.c.is_punct(x, "}") {
                        if self.c.is_ident(x) {
                            ft.push_str(self.c.text(x));
                        }
                        x += 1;
                    }
                    let fq = format!("{qual}::{fname}");
                    self.add_symbol("field", &fname, &fq, self.c.line(f), self.c.line(f), format!("{fname}: {ft}"), -1, &qual);
                    f = x;
                    continue;
                }
                f += 1;
            }
            return close + 1;
        }
        k
    }

    fn parse_enum(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}::{}", self.res.module, name);
        let line = self.c.line(j);
        let mut body_open = None;
        let mut k = j + 1;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_punct(k, ";") {
                break;
            }
            k += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        self.add_symbol("enum", &name, &qual, line, end_line, self.c.raw_text(i, body_open.unwrap_or(k)), -1, "");
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut d = 0i32;
            let mut v = o + 1;
            while v < close {
                if self.c.toks[v].kind == crate::lexer::Kind::Punct {
                    match self.c.text(v) {
                        "(" | "{" | "[" => d += 1,
                        ")" | "}" | "]" => d -= 1,
                        _ => {}
                    }
                } else if d == 0 && self.c.is_ident(v) {
                    let vname = self.c.text(v).to_string();
                    let vq = format!("{qual}::{vname}");
                    self.add_symbol("enum_member", &vname, &vq, self.c.line(v), self.c.line(v), vname.clone(), -1, &qual);
                }
                v += 1;
            }
            return close + 1;
        }
        k
    }

    fn parse_trait(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}::{}", self.res.module, name);
        let line = self.c.line(j);
        let mut bases: Vec<String> = Vec::new();
        let mut k = j + 1;
        if self.c.is_punct(k, ":") {
            k += 1;
            let mut cur = String::new();
            while k < self.c.len() && !self.c.is_punct(k, "{") {
                if self.c.is_ident(k) && !KW.contains(&self.c.text(k)) {
                    cur.push_str(self.c.text(k));
                } else if self.c.is_punct(k, "+") || self.c.is_punct(k, ",") {
                    if !cur.is_empty() {
                        bases.push(std::mem::take(&mut cur));
                    }
                } else if self.c.is_punct(k, ";") {
                    break;
                }
                k += 1;
            }
            if !cur.is_empty() {
                bases.push(cur);
            }
        }
        let mut body_open = None;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_punct(k, ";") {
                break;
            }
            k += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        self.add_symbol("interface", &name, &qual, line, end_line, self.c.raw_text(i, body_open.unwrap_or(k)), -1, "");
        let key = symbol_key(&qual, "interface", line);
        for b in &bases {
            self.add_ref(&key, &last_segment(b), "implements", &self.resolve_path(b), line, -1);
        }
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut t = o + 1;
            while t < close {
                if self.c.is_ident(t) && self.c.text(t) == "fn" && self.c.is_ident(t + 1) {
                    let m = self.c.text(t + 1).to_string();
                    let mq = format!("{qual}::{m}");
                    self.add_symbol("method", &m, &mq, self.c.line(t), self.c.line(t), m.clone(), -1, &qual);
                    while t < close && !self.c.is_punct(t, ";") && !self.c.is_punct(t, "{") {
                        t += 1;
                    }
                    if self.c.is_punct(t, "{") {
                        t = self.c.match_close(t);
                    }
                }
                t += 1;
            }
            return close + 1;
        }
        k
    }

    fn parse_impl(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if self.c.is_punct(j, "<") {
            if let Some(gt) = self.c.match_angle(j, self.c.len()) {
                j = gt + 1;
            }
        }
        let mut head: Vec<String> = Vec::new();
        let mut k = j;
        while k < self.c.len() && !self.c.is_punct(k, "{") && !self.c.is_punct(k, ";") {
            if self.c.is_ident(k) {
                head.push(self.c.text(k).to_string());
            }
            if self.c.is_punct(k, "<") {
                if let Some(gt) = self.c.match_angle(k, self.c.len()) {
                    k = gt + 1;
                    continue;
                }
            }
            k += 1;
        }
        let mut trait_name = String::new();
        let mut type_name = String::new();
        for (idx, w) in head.iter().enumerate() {
            if w == "for" {
                if idx > 0 {
                    trait_name = head[idx - 1].clone();
                }
                if idx + 1 < head.len() {
                    type_name = head[idx + 1].clone();
                }
                break;
            }
        }
        if type_name.is_empty() {
            if let Some(last) = head.last() {
                type_name = last.clone();
            }
        }
        if type_name.is_empty() {
            return k;
        }
        let type_qual = format!("{}::{}", self.res.module, type_name);
        if self.c.is_punct(k, "{") {
            let close = self.c.match_close(k);
            if !trait_name.is_empty() {
                let line = self.c.line(i);
                let src = symbol_key(&type_qual, "struct", line);
                let t = self.resolve_path(&trait_name);
                self.add_ref(&src, &last_segment(&trait_name), "implements", &t, line, -1);
            }
            let mut b = k + 1;
            while b < close {
                if self.c.is_ident(b) && self.c.text(b) == "fn" && self.c.is_ident(b + 1) {
                    b = self.parse_fn(b, &type_qual);
                } else if self.c.is_punct(b, "{") {
                    b = self.c.match_close(b) + 1;
                } else {
                    b += 1;
                }
            }
            return close + 1;
        }
        k
    }

    fn parse_fn(&mut self, i: usize, container: &str) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let line = self.c.line(j);
        let (qual, kind) = if container.is_empty() {
            (format!("{}::{}", self.res.module, name), "function")
        } else {
            (format!("{container}::{name}"), "method")
        };
        j += 1;
        if self.c.is_punct(j, "<") {
            if let Some(gt) = self.c.match_angle(j, self.c.len()) {
                j = gt + 1;
            }
        }
        let mut params_open = None;
        let mut params_close = None;
        if self.c.is_punct(j, "(") {
            params_open = Some(j);
            params_close = Some(self.c.match_close(j));
        }
        let mut body_open = None;
        if let Some(pc) = params_close {
            let mut k = pc + 1;
            while k < self.c.len() {
                if self.c.is_punct(k, "{") {
                    body_open = Some(k);
                    break;
                }
                if self.c.is_punct(k, ";") {
                    break;
                }
                k += 1;
            }
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        let sig_end = body_open.unwrap_or(params_close.unwrap_or(j) + 1);
        let pc = match (params_open, params_close) {
            (Some(o), Some(c)) => count_params(&self.c, o, c),
            _ => -1,
        };
        self.add_symbol(kind, &name, &qual, line, end_line, self.c.raw_text(i, sig_end), pc, container);
        let key = symbol_key(&qual, kind, line);
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut vars: HashMap<String, String> = HashMap::new();
            if let (Some(po), Some(pc2)) = (params_open, params_close) {
                collect_vars(&self.c, po + 1, pc2, &mut vars);
            }
            self.scan_refs(o + 1, close, &key, container, &vars);
            return close + 1;
        }
        params_close.unwrap_or(j) + 1
    }

    fn parse_alias(&mut self, i: usize) -> usize {
        let j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}::{}", self.res.module, name);
        let line = self.c.line(j);
        self.add_symbol("type", &name, &qual, line, line, self.c.raw_text(i, j + 1), -1, "");
        let mut k = j + 1;
        while k < self.c.len() && !self.c.is_punct(k, ";") {
            k += 1;
        }
        k + 1
    }

    fn parse_const(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        while j < self.c.len() && !self.c.is_ident(j) {
            j += 1;
        }
        if j >= self.c.len() {
            return j;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}::{}", self.res.module, name);
        let line = self.c.line(j);
        self.add_symbol("field", &name, &qual, line, line, self.c.raw_text(i, j + 1), -1, "");
        while j < self.c.len() && !self.c.is_punct(j, ";") {
            j += 1;
        }
        j + 1
    }

    fn resolve_path(&self, p: &str) -> String {
        if let Some(full) = self.aliases.get(p) {
            return full.clone();
        }
        let mut parts: Vec<String> = p.split("::").map(|s| s.to_string()).collect();
        if let Some(first) = parts.first().cloned() {
            if let Some(full) = self.aliases.get(&first) {
                parts[0] = full.clone();
                return parts.join("::");
            }
            match first.as_str() {
                "crate" => return p.to_string(),
                "self" => {
                    let rest = parts[1..].join("::");
                    return format!("{}::{rest}", self.res.module);
                }
                "super" => {
                    let mut base = self.res.module.clone();
                    while parts.first().map(|s| s == "super").unwrap_or(false) {
                        parts.remove(0);
                        if let Some(i) = base.rfind("::") {
                            base.truncate(i);
                        }
                    }
                    if parts.is_empty() {
                        return base;
                    }
                    return format!("{base}::{}", parts.join("::"));
                }
                _ => {}
            }
        }
        p.to_string()
    }

    fn scan_refs(&mut self, from: usize, to: usize, src_key: &str, owner: &str, vars: &HashMap<String, String>) {
        let mut j = from;
        while j < to {
            if !self.c.is_ident(j) {
                j += 1;
                continue;
            }
            let word = self.c.text(j).to_string();
            if KW.contains(&word.as_str()) {
                j += 1;
                continue;
            }
            if self.c.is_punct(j + 1, "::") {
                let mut segs: Vec<String> = vec![word.clone()];
                let mut k = j + 1;
                while k < to && (self.c.is_ident(k) || self.c.is_punct(k, "::")) {
                    if self.c.is_ident(k) {
                        segs.push(self.c.text(k).to_string());
                    }
                    k += 1;
                }
                if segs.len() >= 2 && (self.c.is_punct(k, "(") || self.c.is_punct(k, "{")) {
                    let name = segs[segs.len() - 1].clone();
                    let recv = segs[..segs.len() - 1].join("::");
                    let rtype = self.resolve_path(&recv);
                    let kind = if self.c.is_punct(k, "{") { "constructs" } else { "calls" };
                    let arity = if self.c.is_punct(k, "(") {
                        count_params(&self.c, k, self.c.match_close(k))
                    } else {
                        -1
                    };
                    let dedup = format!("{src_key}|{word}|{name}|{kind}");
                    if self.seen.insert(dedup) {
                        self.add_ref(src_key, &name, kind, &rtype, self.c.line(j), arity);
                    }
                    j = k;
                    continue;
                }
            }
            if self.c.is_punct(j + 1, ".") && self.c.is_ident(j + 2) {
                let member = self.c.text(j + 2).to_string();
                let rtype = if word == "self" {
                    owner.to_string()
                } else if let Some(t) = vars.get(&word) {
                    t.clone()
                } else if let Some(t) = self
                    .res
                    .symbols
                    .iter()
                    .find(|s| s.kind == "field" && s.container == owner && s.name == word)
                {
                    s_type(&t.signature)
                } else {
                    String::new()
                };
                let kind = if self.c.is_punct(j + 3, "(") { "calls" } else { "references" };
                let arity = if kind == "calls" {
                    count_params(&self.c, j + 3, self.c.match_close(j + 3))
                } else {
                    -1
                };
                let dedup = format!("{src_key}|{word}|{member}|{kind}");
                if self.seen.insert(dedup) {
                    self.add_ref(src_key, &member, kind, &rtype, self.c.line(j), arity);
                }
                j += 2;
                continue;
            }
            if self.c.is_punct(j + 1, "(") {
                let arity = count_params(&self.c, j + 1, self.c.match_close(j + 1));
                let dedup = format!("{src_key}|call|{word}");
                if self.seen.insert(dedup) {
                    self.add_ref(src_key, &word, "calls", "", self.c.line(j), arity);
                }
                j += 1;
                continue;
            }
            if self.c.is_punct(j + 1, "{") && word.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                let dedup = format!("{src_key}|{word}|constructs");
                if self.seen.insert(dedup) {
                    self.add_ref(src_key, &word, "constructs", &word, self.c.line(j), -1);
                }
            }
            j += 1;
        }
    }
}

fn s_type(sig: &str) -> String {
    sig.split(':').nth(1).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn collect_vars(c: &Cursor, from: usize, to: usize, vars: &mut HashMap<String, String>) {
    let mut j = from;
    while j < to {
        if c.is_ident(j) && c.is_punct(j + 1, ":") && c.is_ident(j + 2) {
            vars.insert(c.text(j).to_string(), c.text(j + 2).to_string());
            j += 3;
            continue;
        }
        j += 1;
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
            "(" | "[" | "{" | "<" => depth += 1,
            ")" | "]" | "}" | ">" => depth -= 1,
            "," if depth == 0 => count += 1,
            _ => {}
        }
    }
    if has { count } else { 0 }
}
