//! Lexer-driven frontends for game-development languages without a maintained
//! tree-sitter grammar: Haxe, GameMaker Language (GML) and the shader
//! dialects HLSL, GLSL, Metal and WGSL.
//!
//! Like every other frontend here these are static navigation aids, not
//! compilers: they record declarations, obvious calls and member accesses so
//! symbols and call edges can be searched. Dynamic dispatch is never guessed.

use std::collections::{HashMap, HashSet};

use crate::lexer::{Cursor, Options};
use crate::model::{symbol_key, FileParse, Ref, Symbol};

const MAX_SOURCE: usize = 16 * 1024 * 1024;

pub fn parse(rel: &str, root: &str, lang: &str, source: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        module: crate::tree_langs::module_of(rel),
        ..Default::default()
    };
    if source.len() > MAX_SOURCE {
        res.errors.push("source exceeds 16 MiB".into());
        return res;
    }
    let options = Options {
        nested_comments: true,
        ..Default::default()
    };
    let mut front = Front {
        c: Cursor::new(source, options),
        res,
        seen: HashSet::new(),
        namespace: String::new(),
        owner: String::new(),
        source: String::new(),
        aliases: HashMap::new(),
        vars: HashMap::new(),
    };
    match lang {
        "haxe" => haxe(&mut front),
        "gml" => gml(&mut front, rel),
        "hlsl" | "glsl" | "metal" | "wgsl" => shader(&mut front),
        other => front
            .res
            .errors
            .push(format!("unsupported language {other}")),
    }
    front.res
}

struct Front<'a> {
    c: Cursor<'a>,
    res: FileParse,
    seen: HashSet<String>,
    namespace: String,
    owner: String,
    source: String,
    aliases: HashMap<String, String>,
    vars: HashMap<String, String>,
}

impl<'a> Front<'a> {
    fn type_qual(&self, name: &str) -> String {
        if self.owner.is_empty() {
            if self.namespace.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", self.namespace, name)
            }
        } else {
            format!("{}.{}", self.owner, name)
        }
    }

    fn add_symbol(
        &mut self,
        kind: &str,
        name: &str,
        qual: &str,
        from: usize,
        to: usize,
        params: i32,
        container: &str,
    ) -> String {
        let line = self.c.line(from) + 1;
        let key = symbol_key(qual, kind, line);
        if !self.seen.insert(key.clone()) {
            return key;
        }
        let sig = self.signature(from, to);
        self.res.symbols.push(Symbol {
            key: key.clone(),
            lang: self.res.lang.clone(),
            kind: kind.to_string(),
            name: name.to_string(),
            qualname: qual.to_string(),
            namespace: self.namespace.clone(),
            start_line: line,
            end_line: self.c.line(to.saturating_sub(1)) + 1,
            signature: sig,
            modifiers: String::new(),
            param_count: params,
            container: container.to_string(),
            module: self.res.module.clone(),
        });
        key
    }

    fn signature(&self, from: usize, to: usize) -> String {
        let text = self.c.raw_text(from, to);
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed.chars().take(200).collect()
    }

    fn add_ref(&mut self, name: &str, receiver: &str, kind: &str, line: i32, arity: i32) {
        if name.is_empty() || self.source.is_empty() {
            return;
        }
        let receiver_type = resolve_receiver(self, receiver);
        let dedupe = format!("{}|{}|{}|{}", self.source, name, kind, line);
        if !self.seen.insert(dedupe) {
            return;
        }
        self.res.refs.push(Ref {
            src_key: self.source.clone(),
            name: name.to_string(),
            kind: kind.to_string(),
            receiver_type,
            namespace_hint: String::new(),
            line: line + 1,
            arity,
        });
    }
}

fn resolve_receiver(f: &Front, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if matches!(raw, "self" | "this" | "Self") {
        return f.owner.clone();
    }
    let parts: Vec<&str> = raw.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return String::new();
    }
    if matches!(parts[0], "self" | "this") {
        if parts.len() > 1 {
            return f
                .vars
                .get(parts[1])
                .cloned()
                .unwrap_or_else(|| f.owner.clone());
        }
        return f.owner.clone();
    }
    if let Some(t) = f.vars.get(raw) {
        return t.clone();
    }
    if let Some(t) = f.vars.get(parts[0]) {
        return t.clone();
    }
    if let Some(alias) = f.aliases.get(parts[0]) {
        if alias.is_empty() {
            return String::new();
        }
        if parts.len() > 1 {
            return format!("{}.{}", alias, parts[1..].join("."));
        }
        return alias.clone();
    }
    if starts_upper(parts[0]) {
        return parts[0].to_string();
    }
    String::new()
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
}

fn is_call_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for" | "foreach" | "while" | "do" | "switch" | "case" | "default"
            | "return" | "else" | "catch" | "try" | "throw" | "new" | "delete"
            | "function" | "var" | "const" | "final" | "static" | "public" | "private"
            | "protected" | "override" | "inline" | "extern" | "import" | "package"
            | "class" | "interface" | "enum" | "typedef" | "abstract" | "extends"
            | "implements" | "using" | "with" | "repeat" | "until" | "exit" | "struct"
            | "sizeof" | "cast" | "typeof" | "defined" | "not" | "and" | "or" | "xor"
            | "discard" | "break" | "continue" | "true" | "false" | "null" | "this"
            | "super" | "fn" | "let" | "loop" | "guard" | "defer"
    )
}

fn is_builtin_type(name: &str) -> bool {
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(
        base,
        "float" | "double" | "half" | "int" | "uint" | "bool" | "void" | "vec"
            | "ivec" | "uvec" | "bvec" | "mat" | "dmat" | "sampler" | "texture"
            | "array" | "atomic" | "ptr" | "packed" | "struct" | "float16"
    ) || name.starts_with("float")
        || name.starts_with("half")
        || name.starts_with("vec")
        || name.starts_with("ivec")
        || name.starts_with("uvec")
        || name.starts_with("bvec")
        || name.starts_with("mat")
        || name.starts_with("dmat")
        || name.starts_with("int")
        || name.starts_with("uint")
        || name.starts_with("sampler")
}

fn count_args(c: &Cursor, open: usize) -> (i32, usize) {
    let close = c.match_close(open);
    if close <= open || close >= c.len() {
        return (0, close);
    }
    // Strings and comments are skipped by the tokenizer, so count from the raw
    // source between the parentheses instead of from the tokens.
    let seg = c.gap(open, close);
    let mut count = 0i32;
    let mut depth = 0i32;
    let mut seen = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in seg.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                seen = true;
            }
            '(' | '[' | '{' => {
                depth += 1;
                seen = true;
            }
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                count += 1;
                seen = true;
            }
            _ if !ch.is_whitespace() => seen = true,
            _ => {}
        }
    }
    let arity = if !seen {
        0
    } else if count == 0 {
        1
    } else {
        count + 1
    };
    (arity, close)
}

fn skip_preprocessor(c: &Cursor, i: usize, to: usize) -> usize {
    let line = c.line(i);
    let mut j = i + 1;
    while j < to && c.line(j) == line {
        j += 1;
    }
    j
}

fn recv_chain(c: &Cursor, i: usize) -> String {
    if i < 2 || !c.is_punct(i - 1, ".") || !c.is_ident(i - 2) {
        return String::new();
    }
    let mut start = i - 2;
    while start >= 2 && c.is_punct(start - 1, ".") && c.is_ident(start - 2) {
        start -= 2;
    }
    c.raw_text(start, i - 1)
}

/// Scan a token range for call sites (`f(...)`, `obj.m(...)`, `new T(...)`).
struct CallSite {
    name: String,
    receiver: String,
    kind: &'static str,
    line: i32,
    arity: i32,
}

fn collect_calls(c: &Cursor, from: usize, to: usize, out: &mut Vec<CallSite>) {
    let mut i = from;
    while i < to {
        if c.is_punct(i, "#") {
            i = skip_preprocessor(c, i, to);
            continue;
        }
        if !c.is_ident(i) {
            i += 1;
            continue;
        }
        let name = c.text(i).to_string();
        if name == "new" {
            if i + 1 < to && c.is_ident(i + 1) {
                let ty = c.text(i + 1).to_string();
                let arity = if i + 2 < to && c.is_punct(i + 2, "(") {
                    count_args(c, i + 2).0
                } else {
                    0
                };
                out.push(CallSite {
                    name: ty,
                    receiver: String::new(),
                    kind: "constructs",
                    line: c.line(i),
                    arity,
                });
            }
            i += 2;
            continue;
        }
        let next_call = i + 1 < to && c.is_punct(i + 1, "(");
        let member_call = i >= 2 && c.is_punct(i - 1, ".") && next_call;
        if member_call && !is_call_keyword(&name) {
            let receiver = recv_chain(c, i);
            let (arity, close) = count_args(c, i + 1);
            out.push(CallSite {
                name,
                receiver,
                kind: "calls",
                line: c.line(i),
                arity,
            });
            collect_calls(c, i + 2, close, out);
            i = close + 1;
            continue;
        }
        if next_call && !is_call_keyword(&name) {
            let (arity, close) = count_args(c, i + 1);
            let kind = if starts_upper(&name) && !is_builtin_type(&name) {
                "constructs"
            } else {
                "calls"
            };
            out.push(CallSite {
                name,
                receiver: String::new(),
                kind,
                line: c.line(i),
                arity,
            });
            collect_calls(c, i + 2, close, out);
            i = close + 1;
            continue;
        }
        if i >= 2 && c.is_punct(i - 1, ".") && starts_upper(&name) {
            let receiver = recv_chain(c, i);
            if !receiver.is_empty() {
                out.push(CallSite {
                    name,
                    receiver,
                    kind: "references",
                    line: c.line(i),
                    arity: 0,
                });
            }
        }
        i += 1;
    }
}

fn scan_calls(f: &mut Front, from: usize, to: usize) {
    let mut sites = Vec::new();
    collect_calls(&f.c, from, to, &mut sites);
    for site in sites {
        f.add_ref(&site.name, &site.receiver, site.kind, site.line, site.arity);
    }
}

fn collect_locals(f: &mut Front, from: usize, to: usize) {
    let c = &f.c;
    let mut found: Vec<(String, String)> = Vec::new();
    let mut i = from;
    while i < to {
        if c.is_ident(i)
            && matches!(c.text(i), "var" | "final" | "let" | "const")
            && i + 1 < to
            && c.is_ident(i + 1)
        {
            let name = c.text(i + 1).to_string();
            let mut ty = String::new();
            if i + 2 < to && c.is_punct(i + 2, ":") && i + 3 < to && c.is_ident(i + 3) {
                ty = c.text(i + 3).to_string();
            } else if i + 2 < to && c.is_punct(i + 2, "=") && i + 3 < to && c.is_ident(i + 3) {
                let rhs = c.text(i + 3).to_string();
                if rhs == "new" && i + 4 < to && c.is_ident(i + 4) {
                    ty = c.text(i + 4).to_string();
                } else if starts_upper(&rhs) {
                    ty = rhs;
                }
            }
            if !ty.is_empty() {
                found.push((name, ty));
            }
        }
        i += 1;
    }
    for (name, ty) in found {
        f.vars.insert(name, ty);
    }
}

fn params_haxe(c: &Cursor, open: usize) -> (Vec<(String, String)>, i32, usize) {
    let close = c.match_close(open);
    let mut out = Vec::new();
    let mut seg_start = open + 1;
    let mut depth = 0i32;
    for k in open + 1..=close {
        let boundary = k == close || (depth == 0 && c.is_punct(k, ","));
        if boundary {
            if seg_start < k {
                let mut name = String::new();
                let mut ty = String::new();
                let mut j = seg_start;
                while j < k && !c.is_ident(j) {
                    j += 1;
                }
                if j < k {
                    name = c.text(j).to_string();
                    if j + 1 < k && c.is_punct(j + 1, ":") {
                        let mut t = Vec::new();
                        let mut m = j + 2;
                        while m < k && !c.is_punct(m, "=") {
                            if c.is_ident(m) {
                                t.push(c.text(m).to_string());
                            }
                            m += 1;
                        }
                        ty = t.join("");
                    }
                }
                if !name.is_empty() && name != "_" {
                    out.push((name, ty));
                }
            }
            seg_start = k + 1;
            continue;
        }
        if c.is_punct(k, "(") || c.is_punct(k, "[") || c.is_punct(k, "{") {
            depth += 1;
        } else if c.is_punct(k, ")") || c.is_punct(k, "]") || c.is_punct(k, "}") {
            depth -= 1;
        }
    }
    let count = out.len() as i32;
    (out, count, close)
}

fn params_gml(c: &Cursor, open: usize) -> (Vec<(String, String)>, i32, usize) {
    let close = c.match_close(open);
    let mut out = Vec::new();
    let mut seg_start = open + 1;
    let mut depth = 0i32;
    for k in open + 1..=close {
        let boundary = k == close || (depth == 0 && c.is_punct(k, ","));
        if boundary {
            let mut j = seg_start;
            while j < k && !c.is_ident(j) {
                j += 1;
            }
            if j < k {
                out.push((c.text(j).to_string(), String::new()));
            }
            seg_start = k + 1;
            continue;
        }
        if c.is_punct(k, "(") || c.is_punct(k, "[") {
            depth += 1;
        } else if c.is_punct(k, ")") || c.is_punct(k, "]") {
            depth -= 1;
        }
    }
    let count = out.len() as i32;
    (out, count, close)
}

// --------------------------------------------------------------------- Haxe

fn haxe(f: &mut Front) {
    let n = f.c.len();
    let mut i = 0;
    while i < n {
        if !f.c.is_ident(i) {
            i += 1;
            continue;
        }
        let word = f.c.text(i).to_string();
        match word.as_str() {
            "package" => {
                let (path, next) = dotted_until(f, i + 1, ";");
                f.namespace = path;
                i = next;
            }
            "import" => {
                let (path, next) = dotted_until(f, i + 1, ";");
                if !path.is_empty() {
                    let alias = path.rsplit('.').next().unwrap_or(&path).to_string();
                    f.aliases.insert(alias, path);
                }
                i = next;
            }
            "class" | "interface" | "enum" | "typedef" | "abstract" => {
                i = haxe_type(f, i);
            }
            "function" => {
                i = haxe_function(f, i);
            }
            _ => i += 1,
        }
    }
}

fn dotted_until(f: &Front, from: usize, end: &str) -> (String, usize) {
    let c = &f.c;
    let n = c.len();
    let mut parts = Vec::new();
    let mut i = from;
    while i < n && !c.is_punct(i, end) {
        if c.is_ident(i) {
            parts.push(c.text(i).to_string());
        }
        i += 1;
    }
    (parts.join("."), if i < n { i + 1 } else { n })
}

fn haxe_type(f: &mut Front, i: usize) -> usize {
    let c = &f.c;
    let n = c.len();
    let keyword = c.text(i).to_string();
    let kind = match keyword.as_str() {
        "interface" => "interface",
        "enum" => "enum",
        "typedef" => "type",
        "abstract" => "class",
        _ => "class",
    };
    let mut j = i + 1;
    while j < n
        && c.is_ident(j)
        && matches!(c.text(j), "abstract" | "final" | "private" | "public" | "extern")
    {
        j += 1;
    }
    if j >= n || !c.is_ident(j) {
        return i + 1;
    }
    let name = c.text(j).to_string();
    let name_tok = j;
    let qual = f.type_qual(&name);
    let mut k = j + 1;
    let mut body_open = None;
    let mut semicolon = None;
    while k < n {
        if c.is_punct(k, "{") {
            body_open = Some(k);
            break;
        }
        if c.is_punct(k, ";") {
            semicolon = Some(k);
            break;
        }
        k += 1;
    }
    let Some(open) = body_open else {
        let key = f.add_symbol(kind, &name, &qual, name_tok, semicolon.unwrap_or(k).max(name_tok + 1), -1, "");
        let _ = key;
        return semicolon.map(|s| s + 1).unwrap_or(k);
    };
    let close = c.match_close(open);
    let mut members: Vec<(String, usize)> = Vec::new();
    let mut depth = 0i32;
    let mut m = open + 1;
    while m < close {
        if c.is_punct(m, "{") {
            depth += 1;
            m += 1;
            continue;
        }
        if c.is_punct(m, "}") {
            depth -= 1;
            m += 1;
            continue;
        }
        if depth == 0 && c.is_ident(m) {
            let word = c.text(m).to_string();
            if matches!(
                word.as_str(),
                "var" | "final" | "function" | "class" | "interface" | "enum" | "typedef"
            ) {
                members.push((word, m));
            }
        }
        m += 1;
    }
    let key = f.add_symbol(kind, &name, &qual, name_tok, open, -1, "");
    let saved = (f.owner.clone(), f.source.clone(), f.vars.clone());
    f.owner = qual.clone();
    f.source = key;
    for (word, m) in members {
        match word.as_str() {
            "var" | "final" => {
                haxe_field(f, m, close);
            }
            "function" => {
                haxe_function(f, m);
            }
            _ => {
                haxe_type(f, m);
            }
        }
    }
    f.owner = saved.0;
    f.source = saved.1;
    f.vars = saved.2;
    close + 1
}

fn haxe_field(f: &mut Front, i: usize, to: usize) -> usize {
    let c = &f.c;
    let j = i + 1;
    if j >= to || !c.is_ident(j) {
        return i + 1;
    }
    let name = c.text(j).to_string();
    let mut ty = String::new();
    if j + 1 < to && c.is_punct(j + 1, ":") && j + 2 < to && c.is_ident(j + 2) {
        ty = c.text(j + 2).to_string();
    }
    let mut k = j + 1;
    while k < to && !c.is_punct(k, ";") {
        k += 1;
    }
    let qual = format!("{}.{}", f.owner, name);
    f.add_symbol(
        "property",
        &name,
        &qual,
        i,
        k.max(i + 1),
        -1,
        &f.owner.clone(),
    );
    if !ty.is_empty() {
        f.vars.insert(name, ty);
    }
    if k < to {
        k + 1
    } else {
        to
    }
}

fn haxe_function(f: &mut Front, i: usize) -> usize {
    let c = &f.c;
    let n = c.len();
    let mut j = i + 1;
    if j >= n || !c.is_ident(j) {
        return i + 1;
    }
    let name = c.text(j).to_string();
    j += 1;
    if j >= n || !c.is_punct(j, "(") {
        return j;
    }
    let (params, param_count, close_params) = params_haxe(c, j);
    let mut k = close_params + 1;
    while k < n && !c.is_punct(k, "{") && !c.is_punct(k, ";") {
        k += 1;
    }
    if k >= n || c.is_punct(k, ";") {
        let kind = if f.owner.is_empty() { "function" } else { "method" };
        let qual = if f.owner.is_empty() {
            f.type_qual(&name)
        } else {
            format!("{}.{}", f.owner, name)
        };
        f.add_symbol(kind, &name, &qual, i, k.max(i + 1), param_count, &f.owner.clone());
        return k + 1;
    }
    let body_close = c.match_close(k);
    let kind = if name == "new" {
        "constructor"
    } else if f.owner.is_empty() {
        "function"
    } else {
        "method"
    };
    let qual = if f.owner.is_empty() {
        f.type_qual(&name)
    } else {
        format!("{}.{}", f.owner, name)
    };
    let key = f.add_symbol(kind, &name, &qual, i, k, param_count, &f.owner.clone());
    let saved = (f.source.clone(), f.vars.clone());
    f.source = key;
    for (pname, pty) in params {
        if !pty.is_empty() {
            f.vars.insert(pname, pty);
        }
    }
    collect_locals(f, k + 1, body_close);
    scan_calls(f, k + 1, body_close);
    f.source = saved.0;
    f.vars = saved.1;
    body_close + 1
}

// ---------------------------------------------------------------------- GML

fn gml(f: &mut Front, rel: &str) {
    let n = f.c.len();
    let mut stem = std::path::Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if stem.is_empty() {
        stem = "script".to_string();
    }
    let mut i = 0;
    let mut saw_function = false;
    while i < n {
        if f.c.is_punct(i, "#") {
            i = skip_preprocessor(&f.c, i, n);
            continue;
        }
        if !f.c.is_ident(i) {
            i += 1;
            continue;
        }
        if f.c.text(i) == "function" && i + 1 < n && f.c.is_ident(i + 1) {
            i = gml_function(f, i, false);
            saw_function = true;
            continue;
        }
        i += 1;
    }
    if !saw_function && n > 0 {
        let key = f.add_symbol("function", &stem, &stem, 0, n, 0, "");
        f.source = key;
        scan_calls(f, 0, n);
    }
}

fn gml_function(f: &mut Front, i: usize, in_constructor: bool) -> usize {
    let c = &f.c;
    let n = c.len();
    let name_tok = i + 1;
    let name = c.text(name_tok).to_string();
    let Some(open) = (name_tok + 1..n).find(|&k| c.is_punct(k, "(")) else {
        return name_tok + 1;
    };
    let (params, param_count, close_params) = params_gml(c, open);
    let mut k = close_params + 1;
    while k < n && !c.is_punct(k, "{") && !c.is_punct(k, ";") {
        k += 1;
    }
    if k >= n || c.is_punct(k, ";") {
        return k + 1;
    }
    let body_close = c.match_close(k);
    let is_constructor = !in_constructor
        && (close_params + 1..k).any(|t| c.is_ident(t) && c.text(t) == "constructor");
    if is_constructor {
        let key = f.add_symbol("class", &name, &name, i, k, param_count, "");
        let saved = (f.owner.clone(), f.source.clone(), f.vars.clone());
        f.owner = name.clone();
        f.source = key;
        for (pname, _) in params {
            f.vars.insert(pname, name.clone());
        }
        gml_constructor_body(f, k + 1, body_close);
        f.owner = saved.0;
        f.source = saved.1;
        f.vars = saved.2;
        return body_close + 1;
    }
    let key = f.add_symbol("function", &name, &name, i, k, param_count, "");
    let saved = (f.source.clone(), f.vars.clone());
    f.source = key;
    for (pname, _) in params {
        f.vars.insert(pname, String::new());
    }
    collect_locals(f, k + 1, body_close);
    scan_calls(f, k + 1, body_close);
    f.source = saved.0;
    f.vars = saved.1;
    body_close + 1
}

fn gml_constructor_body(f: &mut Front, from: usize, to: usize) {
    let owner = f.owner.clone();
    let mut i = from;
    while i < to {
        if !f.c.is_ident(i) {
            i += 1;
            continue;
        }
        let word = f.c.text(i).to_string();
        let mut name_tok = None;
        if word == "static" && i + 2 < to && f.c.is_ident(i + 1) && f.c.is_punct(i + 2, "=") {
            name_tok = Some(i + 1);
        } else if f.c.is_punct(i + 1, "=") {
            name_tok = Some(i);
        }
        if let Some(nt) = name_tok {
            let is_method = nt + 3 < to
                && f.c.is_ident(nt + 2)
                && f.c.text(nt + 2) == "function"
                && f.c.is_punct(nt + 3, "(");
            if is_method {
                i = gml_method(f, nt, nt + 2);
                continue;
            }
            let name = f.c.text(nt).to_string();
            let sig_end = (nt + 1..to)
                .find(|&k| f.c.is_punct(k, ";"))
                .unwrap_or(nt + 1);
            let qual = format!("{}.{}", owner, name);
            f.add_symbol("property", &name, &qual, nt, sig_end, -1, &owner);
        }
        i += 1;
    }
}

fn gml_method(f: &mut Front, name_tok: usize, fn_tok: usize) -> usize {
    let c = &f.c;
    let n = c.len();
    let name = c.text(name_tok).to_string();
    let open = fn_tok + 1;
    let (params, param_count, close_params) = params_gml(c, open);
    let mut k = close_params + 1;
    while k < n && !c.is_punct(k, "{") {
        k += 1;
    }
    if k >= n {
        return k;
    }
    let body_close = c.match_close(k);
    let trailing = body_close + 1 < n && c.is_punct(body_close + 1, ";");
    let owner = f.owner.clone();
    let qual = format!("{}.{}", owner, name);
    let key = f.add_symbol("method", &name, &qual, name_tok, k, param_count, &owner);
    let saved = (f.source.clone(), f.vars.clone());
    f.source = key;
    for (pname, _) in params {
        f.vars.insert(pname, String::new());
    }
    collect_locals(f, k + 1, body_close);
    scan_calls(f, k + 1, body_close);
    f.source = saved.0;
    f.vars = saved.1;
    body_close + 1 + usize::from(trailing)
}

// ------------------------------------------------------------------ Shaders

fn shader(f: &mut Front) {
    let n = f.c.len();
    let mut i = 0;
    while i < n {
        if f.c.is_punct(i, "#") {
            i = skip_preprocessor(&f.c, i, n);
            continue;
        }
        if !f.c.is_ident(i) {
            i += 1;
            continue;
        }
        let word = f.c.text(i).to_string();
        if word == "struct" && i + 1 < n && f.c.is_ident(i + 1) {
            i = shader_struct(f, i);
            continue;
        }
        if word == "fn" && i + 1 < n && f.c.is_ident(i + 1) && i + 2 < n && f.c.is_punct(i + 2, "(") {
            i = shader_function(f, i + 1, true);
            continue;
        }
        if i + 1 < n && f.c.is_punct(i + 1, "(") && !is_call_keyword(&word) && i > 0 && f.c.is_ident(i - 1) && !is_call_keyword(f.c.text(i - 1)) {
            i = shader_function(f, i, false);
            continue;
        }
        i += 1;
    }
}

fn shader_struct(f: &mut Front, i: usize) -> usize {
    let c = &f.c;
    let n = c.len();
    let name = c.text(i + 1).to_string();
    let Some(open) = (i + 2..n).find(|&k| c.is_punct(k, "{")) else {
        return i + 2;
    };
    let close = c.match_close(open);
    let mut fields: Vec<(String, usize, usize)> = Vec::new();
    let mut m = open + 1;
    while m < close {
        if c.is_ident(m) && m + 1 < close && c.is_ident(m + 1) {
            let field = c.text(m + 1).to_string();
            let mut end = m + 2;
            while end < close && !c.is_punct(end, ";") && !c.is_punct(end, "}") {
                end += 1;
            }
            fields.push((field, m, end.max(m + 1)));
            m = end;
        }
        m += 1;
    }
    let key = f.add_symbol("struct", &name, &name, i, open, -1, "");
    let saved = f.source.clone();
    f.source = key;
    for (field, start, end) in fields {
        let qual = format!("{}.{}", name, field);
        f.add_symbol("property", &field, &qual, start, end, -1, &name);
    }
    f.source = saved;
    close + 1
}

fn shader_function(f: &mut Front, name_tok: usize, wgsl_fn: bool) -> usize {
    let c = &f.c;
    let n = c.len();
    let name = c.text(name_tok).to_string();
    let open = name_tok + 1;
    let (arity, close_params) = count_args(c, open);
    let mut k = close_params + 1;
    while k < n && !c.is_punct(k, "{") && !c.is_punct(k, ";") {
        k += 1;
    }
    let from = if wgsl_fn { name_tok.saturating_sub(1) } else if name_tok > 0 { name_tok - 1 } else { name_tok };
    if k >= n || c.is_punct(k, ";") {
        f.add_symbol("function", &name, &name, from, k.max(from + 1), arity, "");
        return k + 1;
    }
    let body_close = c.match_close(k);
    let key = f.add_symbol("function", &name, &name, from, k, arity, "");
    let saved = (f.source.clone(), f.vars.clone());
    f.source = key;
    scan_calls(f, k + 1, body_close);
    f.source = saved.0;
    f.vars = saved.1;
    body_close + 1
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn has_symbol(fp: &crate::model::FileParse, qual: &str) -> bool {
        fp.symbols.iter().any(|s| s.qualname == qual)
    }

    fn has_ref(fp: &crate::model::FileParse, name: &str, kind: &str) -> bool {
        fp.refs.iter().any(|r| r.name == name && r.kind == kind)
    }

    #[test]
    fn haxe_extracts_packages_types_methods_and_calls() {
        let src = r#"
package demo.game;

import demo.util.Logger;

class Player extends Character implements IUpdatable {
    var health:Int = 100;

    public function new(name:String) {
        this.name = name;
    }

    public function takeDamage(amount:Int):Void {
        health -= amount;
        log("hit");
    }

    function log(msg:String):Void {
        Logger.log(msg);
    }
}

enum abstract State(Int) from Int to Int {
    var Idle = 0;
    var Run = 1;
}
"#;
        let fp = parse("src/Player.hx", "app", "haxe", src.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert!(has_symbol(&fp, "demo.game.Player"));
        assert!(has_symbol(&fp, "demo.game.Player.takeDamage"));
        assert!(has_symbol(&fp, "demo.game.Player.health"));
        assert!(has_symbol(&fp, "demo.game.State"));
        assert!(has_ref(&fp, "log", "calls"));
        assert!(has_ref(&fp, "Logger.log", "calls") || has_ref(&fp, "log", "calls"));
    }

    #[test]
    fn gml_extracts_constructors_methods_and_calls() {
        let src = r#"
function Player(_name) constructor {
    name = _name;
    hp = 100;

    take_damage = function(amount) {
        hp -= amount;
        log_hit("hit");
    };

    static log_hit = function(msg) {
        show_debug_message(msg);
    };
}

function scr_damage(_amount) {
    Player("hero").take_damage(10);
}
"#;
        let fp = parse("scripts/Player.gml", "app", "gml", src.as_bytes());
        assert!(has_symbol(&fp, "Player"));
        assert!(has_symbol(&fp, "Player.take_damage"));
        assert!(has_symbol(&fp, "Player.log_hit"));
        assert!(has_symbol(&fp, "scr_damage"));
        assert!(has_ref(&fp, "take_damage", "calls"));
        assert!(has_ref(&fp, "log_hit", "calls"));
    }

    #[test]
    fn hlsl_extracts_structs_functions_and_calls() {
        let src = r#"
struct VSInput {
    float4 position : POSITION;
    float2 uv : TEXCOORD0;
};

float lighting(float3 normal) {
    return max(dot(normal, normalize(light_dir)), 0.0);
}

float4 main(VSInput input) : SV_Target {
    float3 n = normalize(input.normal);
    return float4(lighting(n), 1.0);
}
"#;
        let fp = parse("shaders/Phong.hlsl", "app", "hlsl", src.as_bytes());
        assert!(has_symbol(&fp, "VSInput"));
        assert!(has_symbol(&fp, "lighting"));
        assert!(has_symbol(&fp, "main"));
        assert!(has_ref(&fp, "lighting", "calls"));
        assert!(has_ref(&fp, "normalize", "calls"));
    }

    #[test]
    fn wgsl_and_metal_frontends_capture_entry_points() {
        let wgsl = r#"
struct VertexOut {
    @builtin(position) position : vec4f,
    @location(0) uv : vec2f,
};

@vertex
fn vs_main(@location(0) pos : vec3f) -> VertexOut {
    var out : VertexOut;
    out.position = vec4f(pos, 1.0);
    return out;
}
"#;
        let fp = parse("shaders/basic.wgsl", "app", "wgsl", wgsl.as_bytes());
        assert!(has_symbol(&fp, "VertexOut"));
        assert!(has_symbol(&fp, "vs_main"));

        let metal = r#"
#include <metal_stdlib>
using namespace metal;

vertex float4 vertex_main(const device float3 *positions [[buffer(0)]]) {
    return float4(positions[0], 1.0);
}

fragment half4 fragment_main() {
    return half4(1.0);
}
"#;
        let fp = parse("shaders/basic.metal", "app", "metal", metal.as_bytes());
        assert!(has_symbol(&fp, "vertex_main"));
        assert!(has_symbol(&fp, "fragment_main"));
    }
}
