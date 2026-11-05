//! C# front-end: tolerant declaration scanner + reference harvesting.
//! Mirrors the Go implementation's behaviour (symbols, calls, constructs,
//! references, inheritance, optional type dependencies).

use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Ident,
    Punct,
    Number,
}

#[derive(Clone, Copy)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
    line: i32,
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'@' || c.is_ascii_alphabetic() || c >= 0x80
}
fn is_ident_part(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

const MULTI: &[&[u8]] = &[
    b"=>", b"::", b"==", b"!=", b"<=", b">=", b"&&", b"||", b"++", b"--", b"+=", b"-=",
    b"*=", b"/=", b"%=", b"&=", b"|=", b"^=", b"??=", b"??", b"?.", b"->", b"...",
];

struct Lexer<'a> {
    src: &'a [u8],
    i: usize,
    line: i32,
}

impl<'a> Lexer<'a> {
    fn tokenize(src: &'a [u8]) -> Vec<Tok> {
        let mut lx = Lexer { src, i: 0, line: 1 };
        lx.run()
    }

    fn run(&mut self) -> Vec<Tok> {
        let n = self.src.len();
        let mut out = Vec::with_capacity(n / 4 + 16);
        let mut line_start = true;
        while self.i < n {
            let c = self.src[self.i];
            match c {
                b'\n' => {
                    self.line += 1;
                    self.i += 1;
                    line_start = true;
                    continue;
                }
                b' ' | b'\r' | b'\t' => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            if c == b'#' && line_start {
                while self.i < n && self.src[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            line_start = false;
            if c == b'/' && self.i + 1 < n && self.src[self.i + 1] == b'/' {
                while self.i < n && self.src[self.i] != b'\n' {
                    self.i += 1;
                }
                continue;
            }
            if c == b'/' && self.i + 1 < n && self.src[self.i + 1] == b'*' {
                self.i += 2;
                while self.i + 1 < n && !(self.src[self.i] == b'*' && self.src[self.i + 1] == b'/') {
                    if self.src[self.i] == b'\n' {
                        self.line += 1;
                    }
                    self.i += 1;
                }
                self.i = (self.i + 2).min(n);
                continue;
            }
            if c == b'"' || ((c == b'@' || c == b'$') && self.i + 1 < n && matches!(self.src[self.i + 1], b'"' | b'@' | b'$')) {
                let mut j = self.i;
                let mut verbatim = false;
                while j < n && (self.src[j] == b'@' || self.src[j] == b'$') {
                    verbatim |= self.src[j] == b'@';
                    j += 1;
                }
                if j < n && self.src[j] == b'"' {
                    j += 1;
                    while j < n {
                        if self.src[j] == b'\n' {
                            self.line += 1;
                        }
                        if verbatim && self.src[j] == b'"' && j + 1 < n && self.src[j + 1] == b'"' {
                            j += 2;
                            continue;
                        }
                        if !verbatim && self.src[j] == b'\\' {
                            j += 2;
                            continue;
                        }
                        if self.src[j] == b'"' {
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                    self.i = j;
                    continue;
                }
            }
            if c == b'\'' {
                self.i += 1;
                while self.i < n && self.src[self.i] != b'\'' && self.src[self.i] != b'\n' {
                    if self.src[self.i] == b'\\' {
                        self.i += 1;
                    }
                    self.i += 1;
                }
                self.i = (self.i + 1).min(n);
                continue;
            }
            if is_ident_start(c) {
                let start = self.i;
                while self.i < n && is_ident_part(self.src[self.i]) {
                    self.i += 1;
                }
                out.push(Tok { kind: Kind::Ident, start, end: self.i, line: self.line });
                continue;
            }
            if c.is_ascii_digit() {
                let start = self.i;
                while self.i < n && (is_ident_part(self.src[self.i]) || self.src[self.i] == b'.') {
                    self.i += 1;
                }
                out.push(Tok { kind: Kind::Number, start, end: self.i, line: self.line });
                continue;
            }
            let mut matched = 0usize;
            for mp in MULTI {
                if self.i + mp.len() <= n && &self.src[self.i..self.i + mp.len()] == *mp {
                    matched = mp.len();
                    break;
                }
            }
            if matched > 0 {
                out.push(Tok { kind: Kind::Punct, start: self.i, end: self.i + matched, line: self.line });
                self.i += matched;
                continue;
            }
            out.push(Tok { kind: Kind::Punct, start: self.i, end: self.i + 1, line: self.line });
            self.i += 1;
        }
        out
    }
}

const TYPE_KW: &[(&str, &str)] = &[
    ("class", "class"),
    ("struct", "struct"),
    ("interface", "interface"),
    ("enum", "enum"),
    ("record", "record"),
    ("delegate", "delegate"),
];

const MODIFIERS: &[&str] = &[
    "public", "private", "protected", "internal", "static", "readonly", "const", "virtual",
    "override", "abstract", "async", "partial", "extern", "unsafe", "new", "sealed",
    "volatile", "required", "event", "file",
];

const NOT_CALLS: &[&str] = &[
    "if", "for", "foreach", "while", "switch", "catch", "using", "return", "new", "sizeof",
    "typeof", "nameof", "default", "checked", "unchecked", "lock", "fixed", "await", "throw",
    "when", "case", "yield", "delegate", "stackalloc",
];

const BUILTIN: &[&str] = &[
    "int", "uint", "long", "ulong", "short", "ushort", "byte", "sbyte", "float", "double",
    "decimal", "bool", "char", "string", "object", "void", "var", "dynamic", "nint", "nuint",
    "this", "base", "ref", "out", "in", "params", "readonly",
];

struct Parser<'a> {
    src: &'a [u8],
    toks: Vec<Tok>,
    res: FileParse,
    type_uses: HashSet<String>,
    include_type_deps: bool,
    members: HashMap<String, String>,
}

pub fn parse(rel: &str, root: &str, src: &[u8], include_type_deps: bool) -> FileParse {
    let toks = Lexer::tokenize(src);
    let mut p = Parser {
        src,
        toks,
        res: FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: "csharp".into(),
            module: String::new(),
            ..Default::default()
        },
        type_uses: HashSet::new(),
        include_type_deps,
        members: HashMap::new(),
    };
    let end = p.toks.len();
    p.parse_block(0, end, "", "");
    p.res
}

impl<'a> Parser<'a> {
    fn text(&self, i: usize) -> &str {
        if i >= self.toks.len() {
            return "";
        }
        let t = self.toks[i];
        std::str::from_utf8(&self.src[t.start..t.end]).unwrap_or("")
    }
    fn is_punct(&self, i: usize, s: &str) -> bool {
        i < self.toks.len() && self.toks[i].kind == Kind::Punct && self.text(i) == s
    }
    fn is_ident(&self, i: usize) -> bool {
        i < self.toks.len() && self.toks[i].kind == Kind::Ident
    }
    fn line(&self, i: usize) -> i32 {
        if i < self.toks.len() {
            self.toks[i].line
        } else {
            0
        }
    }
    fn is_modifier(&self, i: usize) -> bool {
        self.is_ident(i) && MODIFIERS.contains(&self.text(i))
    }

    fn match_close(&self, open: usize) -> usize {
        let mut depth = 0i32;
        for j in open..self.toks.len() {
            if self.toks[j].kind != Kind::Punct {
                continue;
            }
            match self.text(j) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return j;
                    }
                }
                _ => {}
            }
        }
        self.toks.len().saturating_sub(1)
    }

    fn match_angle(&self, open: usize, end: usize) -> Option<usize> {
        let mut depth = 0i32;
        for j in open..end {
            if self.toks[j].kind != Kind::Punct {
                continue;
            }
            match self.text(j) {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn raw_text(&self, from: usize, to: usize) -> String {
        let mut out = String::new();
        let mut prev_ident = false;
        for j in from..to.min(self.toks.len()) {
            let t = self.toks[j];
            if prev_ident && t.kind == Kind::Ident {
                out.push(' ');
            }
            out.push_str(self.text(j));
            prev_ident = t.kind == Kind::Ident;
        }
        out
    }

    fn add_symbol(&mut self, kind: &str, name: &str, qual: &str, ns: &str, start: i32, end: i32, sig: String, mods: String, params: i32, container: &str) {
        self.res.symbols.push(Symbol {
            key: symbol_key(qual, kind, start),
            lang: "csharp".into(),
            kind: kind.to_string(),
            name: name.to_string(),
            qualname: qual.to_string(),
            namespace: ns.to_string(),
            start_line: start,
            end_line: end,
            signature: sig,
            modifiers: mods,
            param_count: params,
            container: container.to_string(),
            module: String::new(),
        });
    }

    fn add_ref(&mut self, src_key: &str, name: &str, kind: &str, recv_type: &str, line: i32, ns: &str, arity: i32) {
        if name.is_empty() {
            return;
        }
        self.res.refs.push(Ref {
            src_key: src_key.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            receiver_type: recv_type.to_string(),
            namespace_hint: ns.to_string(),
            line,
            arity,
        });
    }

    fn parse_block(&mut self, from: usize, end: usize, ns: &str, parent_qual: &str) {
        let mut i = from;
        while i < end && i < self.toks.len() {
            let t = self.toks[i];
            if t.kind == Kind::Punct {
                match self.text(i) {
                    "}" => return,
                    "{" | "(" => {
                        i = self.match_close(i) + 1;
                        continue;
                    }
                    ";" | ")" | "]" | "," => {
                        i += 1;
                        continue;
                    }
                    _ => {
                        i += 1;
                        continue;
                    }
                }
            }
            if t.kind != Kind::Ident {
                i += 1;
                continue;
            }
            match self.text(i) {
                "using" => {
                    while i < end && !self.is_punct(i, ";") {
                        i += 1;
                    }
                    i += 1;
                    continue;
                }
                "namespace" => {
                    i = self.parse_namespace(i, end, parent_qual);
                    continue;
                }
                _ => {}
            }
            if let Some((_, kind)) = TYPE_KW.iter().find(|(kw, _)| *kw == self.text(i)) {
                if self.looks_like_type(i) {
                    i = self.parse_type(i, end, ns, parent_qual, kind);
                    continue;
                }
            }
            if !parent_qual.is_empty() {
                let next = self.parse_member(i, end, ns, parent_qual);
                if next > i {
                    i = next;
                    continue;
                }
            }
            i += 1;
        }
    }

    fn parse_namespace(&mut self, i: usize, end: usize, parent_qual: &str) -> usize {
        let mut j = i + 1;
        let mut ns = String::new();
        while j < end && !self.is_punct(j, "{") && !self.is_punct(j, ";") {
            if self.is_ident(j) || self.is_punct(j, ".") || self.is_punct(j, "::") {
                ns.push_str(self.text(j));
            }
            j += 1;
        }
        if self.is_punct(j, "{") {
            let close = self.match_close(j);
            self.parse_block(j + 1, close, &ns, parent_qual);
            return close + 1;
        }
        j + 1
    }

    fn looks_like_type(&self, i: usize) -> bool {
        if !self.is_ident(i + 1) {
            return false;
        }
        let mut j = i + 2;
        if self.is_punct(j, "<") {
            if let Some(gt) = self.match_angle(j, self.toks.len()) {
                j = gt + 1;
            }
        }
        self.is_punct(j, "{") || self.is_punct(j, ":") || self.is_punct(j, ";")
    }

    fn parse_type(&mut self, i: usize, end: usize, ns: &str, parent_qual: &str, kind: &str) -> usize {
        let name = self.text(i + 1).to_string();
        let qual = if parent_qual.is_empty() {
            if ns.is_empty() { name.clone() } else { format!("{ns}.{name}") }
        } else {
            format!("{parent_qual}.{name}")
        };
        let start = self.line(i);
        let mut next = i + 2;
        if self.is_punct(next, "<") {
            if let Some(gt) = self.match_angle(next, end) {
                next = gt + 1;
            }
        }
        let mut bases: Vec<String> = Vec::new();
        if self.is_punct(next, ":") {
            let mut cur = String::new();
            let mut j = next + 1;
            let mut paren = 0i32;
            let mut angle = 0i32;
            while j < end {
                if self.is_ident(j) && self.text(j) == "where" && paren == 0 && angle == 0 {
                    break;
                }
                if self.toks[j].kind == Kind::Punct {
                    match self.text(j) {
                        "{" | ";" => break,
                        "(" => paren += 1,
                        ")" => paren -= 1,
                        "<" => angle += 1,
                        ">" => angle -= 1,
                        "," if paren == 0 && angle == 0 => {
                            if !cur.is_empty() {
                                bases.push(cur.clone());
                                cur.clear();
                            }
                            j += 1;
                            continue;
                        }
                        _ => {}
                    }
                }
                if self.is_ident(j) || self.is_punct(j, ".") || self.is_punct(j, "::") {
                    cur.push_str(self.text(j));
                }
                j += 1;
            }
            if !cur.is_empty() {
                bases.push(cur);
            }
            next = j;
            while next < end && self.is_ident(next) && self.text(next) == "where" {
                while next < end && !self.is_punct(next, "{") && !self.is_punct(next, ";") {
                    next += 1;
                }
            }
        }
        let mut body_open: Option<usize> = None;
        for j in next..end {
            if self.is_punct(j, "{") {
                body_open = Some(j);
                break;
            }
            if self.is_punct(j, ";") {
                break;
            }
        }
        let mut end_line = start;
        if let Some(o) = body_open {
            end_line = self.line(self.match_close(o));
        }
        let sig = self.raw_text(i, body_open.unwrap_or((i + 24).min(end)));
        let mods = self.collect_modifiers(i);
        self.add_symbol(kind, &name, &qual, ns, start, end_line, sig, mods, -1, parent_qual);
        let src_key = symbol_key(&qual, kind, start);
        for b in &bases {
            if b.is_empty() {
                continue;
            }
            let last = last_type_name(b);
            self.add_ref(&src_key, &last, "inherits", b, start, ns, -1);
        }
        if let Some(o) = body_open {
            let close = self.match_close(o);
            if kind == "enum" {
                self.collect_enum_members(o + 1, close, &qual, ns, &src_key);
            } else {
                self.parse_block(o + 1, close, ns, &qual);
            }
            return close + 1;
        }
        next
    }

    fn collect_enum_members(&mut self, from: usize, to: usize, qual: &str, ns: &str, parent_key: &str) {
        let mut j = from;
        while j < to {
            if self.is_ident(j) && !self.is_modifier(j) {
                let next_punct = j + 1 < to && self.toks[j + 1].kind == Kind::Punct
                    && matches!(self.text(j + 1), "=" | "," | "}");
                if next_punct {
                    let name = self.text(j).to_string();
                    let q = format!("{qual}.{name}");
                    self.add_symbol("enum_member", &name, &q, ns, self.line(j), self.line(j), name.clone(), String::new(), -1, qual);
                    self.add_ref(parent_key, &name, "declares", "", self.line(j), ns, -1);
                }
            }
            j += 1;
        }
    }

    fn collect_modifiers(&self, from: usize) -> String {
        let mut out: Vec<&str> = Vec::new();
        let mut j = from;
        while j > 0 {
            j -= 1;
            if self.toks[j].kind == Kind::Punct && matches!(self.text(j), ";" | "{" | "}") {
                break;
            }
            if self.is_modifier(j) {
                out.push(self.text(j));
            }
            if out.len() >= 8 {
                break;
            }
        }
        out.reverse();
        out.join(" ")
    }

    fn parse_member(&mut self, from: usize, end: usize, ns: &str, type_qual: &str) -> usize {
        if from > 0 {
            let prev = self.text(from - 1);
            if self.toks[from - 1].kind == Kind::Punct
                && matches!(prev, "=" | "=>" | "," | "(" | "[" | ":" | "return")
            {
                return from;
            }
        }
        let mut i = from;
        while i < end && self.is_punct(i, "[") {
            i = self.match_close(i) + 1;
        }
        let mut start = i;
        while start < end && self.is_modifier(start) {
            start += 1;
        }
        if start >= end || !self.is_ident(start) {
            return from;
        }
        let head = self.text(start);
        if NOT_CALLS.contains(&head) || head == "var" || head == "const" {
            return from;
        }
        let mut depth = 0i32;
        let mut j = start;
        let mut term = "";
        let mut term_idx = None;
        let mut eq_idx: Option<usize> = None;
        while j < end {
            if self.toks[j].kind == Kind::Punct {
                match self.text(j) {
                    "<" => depth += 1,
                    ">" => {
                        if depth > 0 {
                            depth -= 1;
                        }
                    }
                    "=" if depth == 0 => {
                        if eq_idx.is_none() {
                            eq_idx = Some(j);
                        }
                    }
                    "(" | "{" | ";" | "=>" | "}" if depth == 0 => {
                        if self.text(j) == "(" && eq_idx.is_some() {
                            // initializer call, not a parameter list
                        } else {
                            term = self.text(j);
                            term_idx = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        let Some(term_idx) = term_idx else { return from };
        if term.is_empty() || term == "}" {
            return from;
        }
        let name_end = eq_idx.filter(|e| *e < term_idx).unwrap_or(term_idx);
        let mut name_idx = None;
        let mut k = name_end;
        while k > start {
            k -= 1;
            if self.is_ident(k) {
                name_idx = Some(k);
                break;
            }
        }
        let Some(name_idx) = name_idx else { return from };
        let name = self.text(name_idx).to_string();
        if name.is_empty() || matches!(name.as_str(), "get" | "set" | "init" | "add" | "remove") {
            return from;
        }
        let is_ctor = name_idx == start && term == "(" && name == last_type_name(type_qual);
        if name_idx == start && !is_ctor {
            return from;
        }
        let type_text = self.raw_text(start, name_idx);
        if type_text.is_empty() && !is_ctor {
            return from;
        }

        let line = self.line(start);
        let mods = self.collect_modifiers(start);
        let qual = format!("{type_qual}.{name}");
        let mut param_count = -1i32;
        let mut params_open: Option<usize> = None;
        let mut params_close: Option<usize> = None;
        let mut body_open: Option<usize> = None;
        let mut body_close: Option<usize> = None;
        let kind = match term {
            "(" => {
                let close = self.match_close(term_idx);
                params_open = Some(term_idx);
                params_close = Some(close);
                param_count = count_params(&self.toks, self.src, term_idx, close);
                let mut k = close + 1;
                while k < end {
                    if self.is_punct(k, "{") {
                        body_open = Some(k);
                        body_close = Some(self.match_close(k));
                        break;
                    }
                    if self.is_punct(k, "=>") {
                        body_open = Some(k);
                        body_close = Some(find_semicolon(&self.toks, self.src, k, end));
                        break;
                    }
                    if self.is_punct(k, ";") || self.is_punct(k, "}") {
                        break;
                    }
                    k += 1;
                }
                if name == last_type_name(type_qual) { "constructor" } else { "method" }
            }
            "{" => {
                body_open = Some(term_idx);
                body_close = Some(self.match_close(term_idx));
                "property"
            }
            "=>" => {
                body_open = Some(term_idx);
                body_close = Some(find_semicolon(&self.toks, self.src, term_idx, end));
                "property"
            }
            ";" => {
                if mods.contains("event") { "event" } else { "field" }
            }
            _ => "field",
        };
        let mut end_line = self.line(name_idx);
        if let Some(c) = body_close {
            end_line = self.line(c);
        }
        let sig_end = body_open.unwrap_or(name_end);
        let sig = self.raw_text(start, sig_end);
        self.add_symbol(kind, &name, &qual, ns, line, end_line, sig, mods.clone(), param_count, type_qual);
        let src_key = symbol_key(&qual, kind, line);
        self.members.insert(format!("{type_qual}|{name}"), type_text.clone());

        // type dependencies
        let raw_decl = self.raw_text(start, name_idx);
        self.add_type_uses(&src_key, ns, &raw_decl, line);
        if let (Some(po), Some(pc)) = (params_open, params_close) {
            let raw_params = self.raw_text(po + 1, pc);
            self.add_type_uses(&src_key, ns, &raw_params, line);
        }

        let param_types = match (params_open, params_close) {
            (Some(po), Some(pc)) => self.param_types(po, pc),
            _ => HashMap::new(),
        };
        if let (Some(bo), Some(bc)) = (body_open, body_close) {
            if bc > bo {
                self.scan_refs(bo + 1, bc, &src_key, ns, type_qual, &param_types);
            }
        }
        let next = body_close.map(|c| c + 1).unwrap_or(term_idx + 1);
        if next <= from { from + 1 } else { next }
    }

    fn add_type_uses(&mut self, src_key: &str, ns: &str, text: &str, line: i32) {
        if !self.include_type_deps || text.is_empty() {
            return;
        }
        for name in type_names(text) {
            if BUILTIN.contains(&name.as_str()) {
                continue;
            }
            let first = name.as_bytes()[0];
            if !first.is_ascii_uppercase() {
                continue;
            }
            let k = format!("{src_key}|{name}");
            if !self.type_uses.insert(k) {
                continue;
            }
            self.add_ref(src_key, &name, "type_uses", &name, line, ns, -1);
        }
    }

    fn param_types(&self, open: usize, close: usize) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let mut depth = 0i32;
        let mut seg = open + 1;
        let mut j = open + 1;
        while j <= close {
            let at_end = j == close;
            let mut is_sep = false;
            if !at_end && self.toks[j].kind == Kind::Punct {
                match self.text(j) {
                    "(" | "[" | "{" | "<" => depth += 1,
                    ")" | "]" | "}" | ">" => depth -= 1,
                    "," if depth == 0 => is_sep = true,
                    _ => {}
                }
            }
            if at_end || is_sep {
                let mut idents: Vec<&str> = Vec::new();
                for x in seg..j {
                    if self.is_ident(x) {
                        let t = self.text(x);
                        if !matches!(t, "ref" | "out" | "in" | "params" | "this" | "scoped") {
                            idents.push(t);
                        }
                    }
                }
                if idents.len() >= 2 {
                    out.insert(
                        idents[idents.len() - 1].to_string(),
                        idents[idents.len() - 2].to_string(),
                    );
                }
                seg = j + 1;
            }
            j += 1;
        }
        out
    }

    fn infer_receiver(&self, recv: &str, vars: &HashMap<String, String>, owner: &str) -> String {
        let recv = recv.trim();
        if recv.is_empty() {
            return owner.to_string();
        }
        if recv == "this" || recv == "base" {
            return owner.to_string();
        }
        let head = recv.split('.').next().unwrap_or(recv).trim();
        if let Some(t) = vars.get(head) {
            return t.clone();
        }
        if let Some(t) = self.members.get(&format!("{owner}|{head}")) {
            return t.clone();
        }
        if head.as_bytes().first().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
            return head.to_string();
        }
        String::new()
    }

    fn scan_refs(&mut self, from: usize, to: usize, src_key: &str, ns: &str, owner: &str, param_types: &HashMap<String, String>) {
        let mut vars: HashMap<String, String> = param_types.clone();
        // local declarations: T x = / T x ; / var x = new T
        let mut j = from;
        while j + 2 < to {
            if self.is_ident(j) && self.text(j) == "var" && self.is_ident(j + 1) {
                if self.is_punct(j + 2, "=") && j + 3 < to && self.is_ident(j + 3) && self.text(j + 3) == "new" {
                    let mut k = j + 4;
                    let mut parts = String::new();
                    while k < to && (self.is_ident(k) || self.is_punct(k, ".")) {
                        parts.push_str(self.text(k));
                        k += 1;
                    }
                    vars.insert(self.text(j + 1).to_string(), last_type_name(&parts));
                }
            } else if self.is_ident(j) && self.is_ident(j + 1) {
                if self.is_punct(j + 2, "=") || self.is_punct(j + 2, ";") || self.is_punct(j + 2, ",") {
                    let t = self.text(j);
                    if !NOT_CALLS.contains(&t) && t != "var" && t != "is" && t != "as" {
                        vars.insert(self.text(j + 1).to_string(), t.to_string());
                    }
                }
            }
            j += 1;
        }

        let mut k = from;
        while k < to {
            if !self.is_ident(k) {
                k += 1;
                continue;
            }
            let t = self.text(k).to_string();
            if t == "new" {
                let mut parts = String::new();
                let mut x = k + 1;
                while x < to && (self.is_ident(x) || self.is_punct(x, ".")) {
                    parts.push_str(self.text(x));
                    x += 1;
                }
                if !parts.is_empty() {
                    self.add_ref(src_key, &last_type_name(&parts), "constructs", "", self.line(k), ns, -1);
                }
                k += 1;
                continue;
            }
            if NOT_CALLS.contains(&t.as_str()) {
                k += 1;
                continue;
            }
            let next = k + 1;
            if self.is_punct(next, "<") {
                if let Some(gt) = self.match_angle(next, to) {
                    if self.is_punct(gt + 1, "(") {
                        let (recv, rtype) = self.receiver_before(k, from, &vars, owner);
                        let _ = recv;
                        let arity = count_params(&self.toks, self.src, gt + 1, self.match_close(gt + 1));
                        self.add_ref(src_key, &t, "calls", &rtype, self.line(k), ns, arity);
                        k = gt + 1;
                        continue;
                    }
                }
            }
            if self.is_punct(next, "(") {
                let (_recv, rtype) = self.receiver_before(k, from, &vars, owner);
                let arity = count_params(&self.toks, self.src, next, self.match_close(next));
                self.add_ref(src_key, &t, "calls", &rtype, self.line(k), ns, arity);
                k += 1;
                continue;
            }
            if (self.is_punct(next, ".") || self.is_punct(next, "?.")) && self.is_ident(next + 1) {
                if self.is_punct(next + 2, "(") || self.is_punct(next + 2, "<") {
                    k += 1;
                    continue;
                }
                let rtype = self.infer_receiver(&t, &vars, owner);
                let member = self.text(next + 1).to_string();
                let line = self.line(k);
                self.add_ref(src_key, &member, "references", &rtype, line, ns, -1);
            }
            k += 1;
        }
    }

    fn receiver_before(&self, name_idx: usize, from: usize, vars: &HashMap<String, String>, owner: &str) -> (String, String) {
        if name_idx <= from || !(self.is_punct(name_idx - 1, ".") || self.is_punct(name_idx - 1, "?.")) {
            return (String::new(), String::new());
        }
        if name_idx < from + 2 {
            return (String::new(), String::new());
        }
        let end = name_idx - 2;
        if !self.is_ident(end) {
            return (String::new(), String::new());
        }
        let mut recv = self.text(end).to_string();
        let mut e = end;
        while e >= from + 2 && (self.is_punct(e - 1, ".") || self.is_punct(e - 1, "?.")) && self.is_ident(e - 2) {
            recv = format!("{}.{}", self.text(e - 2), recv);
            e -= 2;
        }
        let head = recv.split('.').next().unwrap_or(&recv).to_string();
        let rtype = if let Some(t) = vars.get(&head) {
            t.clone()
        } else if let Some(t) = self.members.get(&format!("{owner}|{head}")) {
            t.clone()
        } else if head == "this" || head == "base" {
            owner.to_string()
        } else if head.as_bytes().first().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
            head.clone()
        } else {
            String::new()
        };
        (recv, rtype)
    }
}

fn type_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || (ch as u32) >= 0x80 {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn last_type_name(s: &str) -> String {
    let s = s.trim();
    let s = match s.find('<') {
        Some(i) => &s[..i],
        None => s,
    };
    let s = match s.rfind('.') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    let s = match s.find(|c| c == '?' || c == '[') {
        Some(i) => &s[..i],
        None => s,
    };
    s.to_string()
}

fn count_params(toks: &[Tok], src: &[u8], open: usize, close: usize) -> i32 {
    if close <= open + 1 {
        return 0;
    }
    let mut depth = 0i32;
    let mut count = 1i32;
    let mut has_ident = false;
    for j in open + 1..close {
        if toks[j].kind == Kind::Ident {
            has_ident = true;
        }
        if toks[j].kind != Kind::Punct {
            continue;
        }
        let t = std::str::from_utf8(&src[toks[j].start..toks[j].end]).unwrap_or("");
        match t {
            "(" | "{" | "[" | "<" => depth += 1,
            ")" | "}" | "]" | ">" => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            "," if depth == 0 => count += 1,
            _ => {}
        }
    }
    if has_ident { count } else { 0 }
}

fn find_semicolon(toks: &[Tok], src: &[u8], from: usize, end: usize) -> usize {
    let mut depth = 0i32;
    for j in from..end {
        if toks[j].kind != Kind::Punct {
            continue;
        }
        let t = std::str::from_utf8(&src[toks[j].start..toks[j].end]).unwrap_or("");
        match t {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            "}" => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return j;
                }
            }
            ";" if depth == 0 => return j,
            _ => {}
        }
    }
    end.saturating_sub(1)
}
