//! TypeScript/JavaScript front-end: import aliases, class/interface/enum/function.

use crate::lexer::{last_segment, Cursor, Options};
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::HashMap;

const KW: &[&str] = &[
    "if", "for", "while", "switch", "catch", "return", "new", "typeof", "instanceof",
    "delete", "in", "of", "do", "else", "case", "throw", "await", "yield", "super", "this",
    "function", "export", "default", "import", "from", "const", "let", "var", "class",
    "interface", "type", "enum", "extends", "implements", "async", "public", "private",
    "protected", "static", "readonly", "abstract", "get", "set", "constructor",
];

pub fn parse(rel: &str, root: &str, src: &[u8]) -> FileParse {
    let mut p = Parser {
        c: Cursor::new(src, Options { template_strings: true, ..Default::default() }),
        res: FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: "typescript".into(),
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
    let mut r = rel.to_string();
    for ext in [".tsx", ".ts", ".jsx", ".js"] {
        if let Some(s) = r.strip_suffix(ext) {
            r = s.to_string();
            break;
        }
    }
    if let Some(s) = r.strip_suffix("/index") {
        r = s.to_string();
    }
    r.replace('/', ".")
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
            lang: "typescript".into(),
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
                "import" => i = self.parse_import(i),
                "class" => i = self.parse_class(i),
                "interface" => i = self.parse_interface(i),
                "enum" => i = self.parse_enum(i),
                "type" => i = self.parse_type_alias(i),
                "function" => i = self.parse_function(i),
                "const" | "let" | "var" => i = self.parse_var(i),
                _ => i += 1,
            }
        }
    }

    fn parse_import(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        let mut from = String::new();
        let end = {
            let mut k = j;
            while k < self.c.len() && !self.c.is_punct(k, ";") {
                if self.c.is_ident(k) && self.c.text(k) == "from" && k + 1 < self.c.len() {
                    if let Some(t) = self.c.toks.get(k + 1) {
                        let raw = std::str::from_utf8(&self.c.src[t.start..t.end]).unwrap_or("");
                        from = raw.trim_matches(|c| c == '"' || c == '\'').to_string();
                    }
                }
                k += 1;
            }
            k
        };
        let module = resolve_module(&from, &self.res.rel_path);
        if !module.is_empty() {
            let mut braces = 0i32;
            let mut k = i + 1;
            while k < end {
                if self.c.toks[k].kind == crate::lexer::Kind::Punct {
                    match self.c.text(k) {
                        "{" => braces += 1,
                        "}" => braces -= 1,
                        _ => {}
                    }
                } else if self.c.is_ident(k) {
                    let t = self.c.text(k);
                    if matches!(t, "from" | "import" | "type" | "as" | "default") {
                        k += 1;
                        continue;
                    }
                    if braces > 0 {
                        let orig = t.to_string();
                        let mut local = orig.clone();
                        if self.c.is_ident(k + 1) && self.c.text(k + 1) == "as" && self.c.is_ident(k + 2) {
                            local = self.c.text(k + 2).to_string();
                            k += 2;
                        }
                        self.aliases.insert(local, format!("{module}.{orig}"));
                    } else if self.c.is_punct(k + 1, ",") || (self.c.is_ident(k + 1) && self.c.text(k + 1) == "from") {
                        self.aliases.insert(t.to_string(), format!("{module}.{t}"));
                    }
                }
                k += 1;
            }
        }
        end + 1
    }

    fn parse_class(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}.{}", self.res.module, name);
        let line = self.c.line(j);
        let mut bases: Vec<(&str, String)> = Vec::new();
        let mut k = j + 1;
        let mut body_open = None;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_ident(k) && (self.c.text(k) == "extends" || self.c.text(k) == "implements") {
                let kind = if self.c.text(k) == "extends" { "inherits" } else { "implements" };
                k += 1;
                while k < self.c.len() && !self.c.is_punct(k, "{") {
                    if self.c.is_ident(k) {
                        bases.push((kind, self.c.text(k).to_string()));
                    }
                    k += 1;
                }
                continue;
            }
            k += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        self.add_symbol("class", &name, &qual, line, end_line, self.c.raw_text(i, body_open.unwrap_or(k)), -1, "");
        let key = symbol_key(&qual, "class", line);
        for (kind, base) in &bases {
            let rt = self.resolve(base);
            self.add_ref(&key, &last_segment(base), kind, &rt, line, -1);
        }
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            self.body(o + 1, close, &qual);
            return close + 1;
        }
        k
    }

    fn body(&mut self, from: usize, to: usize, type_qual: &str) {
        let mut i = from;
        while i < to {
            if self.c.is_punct(i, "{") {
                i = self.c.match_close(i) + 1;
                continue;
            }
            if !self.c.is_ident(i) {
                i += 1;
                continue;
            }
            let name = self.c.text(i).to_string();
            if KW.contains(&name.as_str()) && name != "constructor" && name != "get" && name != "set" {
                i += 1;
                continue;
            }
            let is_method = self.c.is_punct(i + 1, "(") || self.c.is_punct(i + 1, "<");
            if is_method {
                let mut p = i + 1;
                if self.c.is_punct(p, "<") {
                    if let Some(gt) = self.c.match_angle(p, to) {
                        p = gt + 1;
                    }
                }
                if !self.c.is_punct(p, "(") {
                    i += 1;
                    continue;
                }
                let pc = self.c.match_close(p);
                let kind = if name == "constructor" { "constructor" } else { "method" };
                let mq = format!("{type_qual}.{name}");
                let line = self.c.line(i);
                let mut body_open = None;
                let mut k = pc + 1;
                while k < to {
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
                self.add_symbol(kind, &name, &mq, line, end_line, self.c.raw_text(i, body_open.unwrap_or(k)), count_params(&self.c, p, pc), type_qual);
                let key = symbol_key(&mq, kind, line);
                if let Some(o) = body_open {
                    let close = self.c.match_close(o);
                    let mut vars: HashMap<String, String> = HashMap::new();
                    collect_param_vars(&self.c, p + 1, pc, &mut vars);
                    self.scan_refs(o + 1, close, &key, type_qual, &vars);
                    i = close + 1;
                } else {
                    i = pc + 1;
                }
                continue;
            }
            // property: name[: type] = / ;
            if self.c.is_punct(i + 1, ":") && self.c.is_ident(i + 2) {
                let ptype = self.c.text(i + 2).to_string();
                let pq = format!("{type_qual}.{name}");
                self.add_symbol("property", &name, &pq, self.c.line(i), self.c.line(i), format!("{name}: {ptype}"), -1, type_qual);
                let key = symbol_key(type_qual, "class", self.c.line(i));
                let _ = key;
            }
            i += 1;
        }
    }

    fn parse_interface(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}.{}", self.res.module, name);
        let line = self.c.line(j);
        let mut bases: Vec<String> = Vec::new();
        let mut k = j + 1;
        let mut body_open = None;
        while k < self.c.len() {
            if self.c.is_punct(k, "{") {
                body_open = Some(k);
                break;
            }
            if self.c.is_ident(k) && self.c.text(k) == "extends" {
                k += 1;
                while k < self.c.len() && !self.c.is_punct(k, "{") {
                    if self.c.is_ident(k) {
                        bases.push(self.c.text(k).to_string());
                    }
                    k += 1;
                }
                continue;
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
            let rt = self.resolve(b);
            self.add_ref(&key, &last_segment(b), "inherits", &rt, line, -1);
        }
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut m = o + 1;
            while m < close {
                if self.c.is_ident(m) && self.c.is_punct(m + 1, "(") {
                    let mn = self.c.text(m).to_string();
                    let mc = self.c.match_close(m + 1);
                    let mq = format!("{qual}.{mn}");
                    self.add_symbol("method", &mn, &mq, self.c.line(m), self.c.line(m), mn.clone(), count_params(&self.c, m + 1, mc), &qual);
                    m = mc + 1;
                    continue;
                }
                if self.c.is_ident(m) && self.c.is_punct(m + 1, ":") && self.c.is_ident(m + 2) {
                    let pn = self.c.text(m).to_string();
                    let pt = self.c.text(m + 2).to_string();
                    let pq = format!("{qual}.{pn}");
                    self.add_symbol("property", &pn, &pq, self.c.line(m), self.c.line(m), format!("{pn}: {pt}"), -1, &qual);
                    m += 3;
                    continue;
                }
                m += 1;
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
        let qual = format!("{}.{}", self.res.module, name);
        let line = self.c.line(j);
        let mut k = j + 1;
        while k < self.c.len() && !self.c.is_punct(k, "{") {
            k += 1;
        }
        let mut end_line = line;
        if self.c.is_punct(k, "{") {
            end_line = self.c.line(self.c.match_close(k));
        }
        self.add_symbol("enum", &name, &qual, line, end_line, self.c.raw_text(i, k), -1, "");
        if self.c.is_punct(k, "{") {
            let close = self.c.match_close(k);
            let mut d = 0i32;
            let mut v = k + 1;
            while v < close {
                if self.c.toks[v].kind == crate::lexer::Kind::Punct {
                    match self.c.text(v) {
                        "(" | "{" | "[" => d += 1,
                        ")" | "}" | "]" => d -= 1,
                        _ => {}
                    }
                } else if d == 0 && self.c.is_ident(v) {
                    let vn = self.c.text(v).to_string();
                    let vq = format!("{qual}.{vn}");
                    self.add_symbol("enum_member", &vn, &vq, self.c.line(v), self.c.line(v), vn.clone(), -1, &qual);
                }
                v += 1;
            }
            return close + 1;
        }
        k
    }

    fn parse_type_alias(&mut self, i: usize) -> usize {
        let j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}.{}", self.res.module, name);
        let line = self.c.line(j);
        self.add_symbol("type", &name, &qual, line, line, self.c.raw_text(i, j + 1), -1, "");
        let mut k = j + 1;
        while k < self.c.len() && !self.c.is_punct(k, ";") {
            k += 1;
        }
        k + 1
    }

    fn parse_function(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if !self.c.is_ident(j) {
            return i + 1;
        }
        let name = self.c.text(j).to_string();
        let qual = format!("{}.{}", self.res.module, name);
        let line = self.c.line(j);
        let mut k = j + 1;
        if self.c.is_punct(k, "<") {
            if let Some(gt) = self.c.match_angle(k, self.c.len()) {
                k = gt + 1;
            }
        }
        if !self.c.is_punct(k, "(") {
            return k;
        }
        let pc = self.c.match_close(k);
        let mut body_open = None;
        let mut b = pc + 1;
        while b < self.c.len() {
            if self.c.is_punct(b, "{") {
                body_open = Some(b);
                break;
            }
            if self.c.is_punct(b, ";") {
                break;
            }
            b += 1;
        }
        let mut end_line = line;
        if let Some(o) = body_open {
            end_line = self.c.line(self.c.match_close(o));
        }
        self.add_symbol("function", &name, &qual, line, end_line, self.c.raw_text(i, body_open.unwrap_or(b)), count_params(&self.c, k, pc), "");
        let key = symbol_key(&qual, "function", line);
        if let Some(o) = body_open {
            let close = self.c.match_close(o);
            let mut vars: HashMap<String, String> = HashMap::new();
            collect_param_vars(&self.c, k + 1, pc, &mut vars);
            self.scan_refs(o + 1, close, &key, "", &vars);
            return close + 1;
        }
        b
    }

    fn parse_var(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        while j < self.c.len() && !self.c.is_ident(j) {
            j += 1;
        }
        if j >= self.c.len() {
            return j;
        }
        let name = self.c.text(j).to_string();
        let mut k = j + 1;
        if self.c.is_punct(k, ":") {
            while k < self.c.len() && !self.c.is_punct(k, "=") && !self.c.is_punct(k, ";") {
                k += 1;
            }
        }
        if !self.c.is_punct(k, "=") {
            return k + 1;
        }
        // arrow function or function expression
        let mut arrow_body = None;
        let mut x = k + 1;
        if self.c.is_ident(x) && self.c.text(x) == "async" {
            x += 1;
        }
        if self.c.is_punct(x, "(") {
            let pc = self.c.match_close(x);
            let mut y = pc + 1;
            if self.c.is_punct(y, "=>") {
                if self.c.is_punct(y + 1, "{") {
                    arrow_body = Some((y, self.c.match_close(y + 1)));
                } else {
                    arrow_body = Some((y, y + 2));
                }
            }
        } else if self.c.is_ident(x) && self.c.text(x) == "function" {
            let mut y = x + 1;
            if self.c.is_ident(y) {
                y += 1;
            }
            if self.c.is_punct(y, "(") {
                let pc = self.c.match_close(y);
                if self.c.is_punct(pc + 1, "{") {
                    arrow_body = Some((y - 1, self.c.match_close(pc + 1)));
                }
            }
        }
        let qual = format!("{}.{}", self.res.module, name);
        if let Some((body_start, body_close)) = arrow_body {
            let line = self.c.line(j);
            self.add_symbol("function", &name, &qual, line, self.c.line(body_close), self.c.raw_text(i, body_start), -1, "");
            let key = symbol_key(&qual, "function", line);
            self.scan_refs(body_start, body_close, &key, "", &HashMap::new());
            return body_close + 1;
        }
        // plain constant
        let line = self.c.line(j);
        self.add_symbol("field", &name, &qual, line, line, self.c.raw_text(i, k), -1, "");
        let key = symbol_key(&qual, "field", line);
        let mut stop = k + 1;
        while stop < self.c.len() && !self.c.is_punct(stop, ";") && !self.c.is_punct(stop, "}") {
            stop += 1;
        }
        self.scan_refs(k + 1, stop, &key, "", &HashMap::new());
        stop + 1
    }

    fn resolve(&self, name: &str) -> String {
        self.aliases.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    fn scan_refs(&mut self, from: usize, to: usize, src_key: &str, owner: &str, vars: &HashMap<String, String>) {
        let mut j = from;
        while j < to {
            if !self.c.is_ident(j) {
                j += 1;
                continue;
            }
            let word = self.c.text(j).to_string();
            if word == "new" && self.c.is_ident(j + 1) {
                let name = self.c.text(j + 1).to_string();
                let rt = self.resolve(&name);
                let dedup = format!("{src_key}|{name}|constructs");
                if self.seen.insert(dedup) {
                    self.add_ref(src_key, &name, "constructs", &rt, self.c.line(j), -1);
                }
                j += 1;
                continue;
            }
            if KW.contains(&word.as_str()) {
                j += 1;
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
            if (self.c.is_punct(j + 1, ".") || self.c.is_punct(j + 1, "?.")) && self.c.is_ident(j + 2) {
                let member = self.c.text(j + 2).to_string();
                let rtype = if let Some(t) = vars.get(&word) {
                    t.clone()
                } else if let Some(t) = self.aliases.get(&word) {
                    t.clone()
                } else if word == "this" {
                    owner.to_string()
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
                let dedup = format!("{src_key}|{word}|{member}|{kind}");
                if self.seen.insert(dedup) {
                    self.add_ref(src_key, &member, kind, &rtype, self.c.line(j), arity);
                }
                j += 2;
                continue;
            }
            j += 1;
        }
    }
}

fn resolve_module(spec: &str, rel: &str) -> String {
    if spec.is_empty() {
        return String::new();
    }
    if spec.starts_with('.') {
        let base = std::path::Path::new(rel)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let joined = format!("{base}/{spec}");
        let mut parts: Vec<&str> = Vec::new();
        for seg in joined.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                s => parts.push(s),
            }
        }
        let mut m = parts.join(".");
        for ext in [".tsx", ".ts", ".jsx", ".js"] {
            if let Some(s) = m.strip_suffix(ext) {
                m = s.to_string();
                break;
            }
        }
        if let Some(s) = m.strip_suffix(".index") {
            m = s.to_string();
        }
        return m;
    }
    spec.replace('/', ".")
}

fn collect_param_vars(c: &Cursor, from: usize, to: usize, vars: &mut HashMap<String, String>) {
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
