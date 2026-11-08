//! Tree-sitter frontends for game-development languages that have a maintained
//! grammar: GDScript, Kotlin, Swift, Objective-C and Dart.
//!
//! The frontends are deliberately shallow static navigation aids, in line with
//! the other source frontends: they record declarations, inheritance, member
//! fields and literal call targets with a best-effort receiver type. Dynamic
//! dispatch is never guessed; unresolved references stay unresolved.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use tree_sitter::{Language, Node, Parser};

use crate::model::{symbol_key, FileParse, Ref, Symbol};

const MAX_SOURCE: usize = 16 * 1024 * 1024;
const MAX_NODES: usize = 2_000_000;
const MAX_DEPTH: usize = 256;
const MAX_DIAGNOSTICS: usize = 32;

/// Default file module name used for the `module` column and symbol module.
pub fn module_of(rel: &str) -> String {
    let p = Path::new(rel);
    let mut parts: Vec<String> = p
        .iter()
        .map(|c| c.to_string_lossy().to_string())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = Path::new(last.as_str())
            .file_stem()
            .and_then(|s| s.to_str())
        {
            *last = stem.to_string();
        }
    }
    if parts.len() > 1 && parts.last().map(|s| s == "index").unwrap_or(false) {
        parts.pop();
    }
    parts.join(".")
}

pub fn parse(rel: &str, root: &str, lang: &str, source: &[u8]) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        module: module_of(rel),
        ..Default::default()
    };
    if source.len() > MAX_SOURCE {
        res.errors.push("source exceeds 16 MiB".into());
        return res;
    }
    let language: Language = match lang {
        "gdscript" => tree_sitter_gdscript::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "objc" => tree_sitter_objc::LANGUAGE.into(),
        "dart" => tree_sitter_dart::LANGUAGE.into(),
        other => {
            res.errors.push(format!("unsupported language {other}"));
            return res;
        }
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        res.errors.push(format!("failed to load the {lang} grammar"));
        return res;
    }
    #[allow(deprecated)]
    parser.set_timeout_micros(2_000_000);
    let tree = match parser.parse(source, None) {
        Some(tree) => tree,
        None => {
            res.errors
                .push("parser timed out after 2 s; no partial graph".into());
            return res;
        }
    };
    let root_node = tree.root_node();
    let mut builder = Builder {
        lang,
        src: source,
        res,
        seen: HashSet::new(),
        classes: HashSet::new(),
        nodes: 0,
        stopped_nodes: false,
        stopped_depth: false,
    };
    match lang {
        "gdscript" => walk_gdscript(&mut builder, root_node),
        "kotlin" => walk_kotlin(&mut builder, root_node),
        "swift" => walk_swift(&mut builder, root_node),
        "objc" => walk_objc(&mut builder, root_node),
        "dart" => walk_dart(&mut builder, root_node),
        _ => {}
    }
    if root_node.has_error() {
        let mut found = 0usize;
        collect_errors(root_node, &mut builder.res.errors, &mut found, 0);
        if builder.res.errors.len() > MAX_DIAGNOSTICS {
            builder.res.errors.truncate(MAX_DIAGNOSTICS);
            builder
                .res
                .errors
                .push("further syntax diagnostics were dropped".into());
        }
        if builder.res.errors.is_empty() {
            builder
                .res
                .errors
                .push("AST contains recovery nodes; partial graph".into());
        }
    }
    builder.res
}

fn collect_errors(node: Node, out: &mut Vec<String>, found: &mut usize, depth: usize) {
    if *found >= MAX_DIAGNOSTICS || depth > MAX_DEPTH {
        return;
    }
    if node.is_error() || node.is_missing() {
        *found += 1;
        let p = node.start_position();
        out.push(format!("syntax recovery at {}:{} ({})", p.row, p.column, node.kind()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_errors(child, out, found, depth + 1);
    }
}

// ---------------------------------------------------------------- utilities

fn text_of<'a>(node: Node<'_>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

fn trim_text<'a>(node: Node<'_>, src: &'a [u8]) -> &'a str {
    text_of(node, src).trim()
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn all_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn descendant<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

fn descendants_of<'t>(node: Node<'t>, kind: &str, out: &mut Vec<Node<'t>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            out.push(child);
        }
        descendants_of(child, kind, out, depth + 1);
    }
}

fn body_of<'t>(node: Node<'t>) -> Option<Node<'t>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    for child in all_children(node) {
        match child.kind() {
            "body" | "function_body" | "block" | "statements" | "compound_statement"
            | "class_body" | "enum_class_body" | "protocol_body" | "enumerator_list"
            | "field_declaration_list" => return Some(child),
            _ => {}
        }
    }
    None
}

fn is_upper(name: &str) -> bool {
    name.chars()
        .next()
        .map(|c| c.is_ascii_uppercase())
        .unwrap_or(false)
}

fn first_identifier(node: Node) -> Option<Node> {
    for child in named_children(node) {
        if child.kind() == "identifier" {
            return Some(child);
        }
    }
    descendant(node, "identifier")
}

fn strip_call(raw: &str) -> String {
    let t = raw.trim();
    match t.find('(') {
        Some(idx) => t[..idx].trim().to_string(),
        None => t.to_string(),
    }
}

#[derive(Clone, Default)]
struct Ctx {
    namespace: String,
    owner: String,
    source: String,
    vars: HashMap<String, String>,
    aliases: HashMap<String, String>,
}

impl Ctx {
    fn qualified(&self, name: &str) -> String {
        if !self.owner.is_empty() {
            format!("{}.{}", self.owner, name)
        } else if !self.namespace.is_empty() {
            format!("{}.{}", self.namespace, name)
        } else {
            name.to_string()
        }
    }
}

struct Builder<'a> {
    lang: &'a str,
    src: &'a [u8],
    res: FileParse,
    seen: HashSet<String>,
    classes: HashSet<String>,
    nodes: usize,
    stopped_nodes: bool,
    stopped_depth: bool,
}

impl<'a> Builder<'a> {
    fn tick(&mut self, depth: usize) -> bool {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            if !self.stopped_nodes {
                self.stopped_nodes = true;
                self.res
                    .errors
                    .push("node budget exceeded; graph truncated".into());
            }
            return false;
        }
        if depth > MAX_DEPTH {
            if !self.stopped_depth {
                self.stopped_depth = true;
                self.res
                    .errors
                    .push("tree depth limit reached; graph truncated".into());
            }
            return false;
        }
        true
    }

    fn add_symbol(
        &mut self,
        node: Node,
        kind: &str,
        name: &str,
        ctx: &Ctx,
        param_count: i32,
    ) -> String {
        let name = name.trim().trim_matches('"').to_string();
        if name.is_empty() {
            return String::new();
        }
        let qual = ctx.qualified(&name);
        let line = node.start_position().row as i32 + 1;
        let key = symbol_key(&qual, kind, line);
        if !self.seen.insert(key.clone()) {
            return key;
        }
        let start = node.start_byte();
        let sig_end = body_of(node)
            .map(|b| b.start_byte())
            .unwrap_or(node.end_byte())
            .max(start)
            .min(start + 1024);
        let signature = String::from_utf8_lossy(&self.src[start..sig_end])
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        self.res.symbols.push(Symbol {
            key: key.clone(),
            lang: self.lang.to_string(),
            kind: kind.to_string(),
            name,
            qualname: qual,
            namespace: ctx.namespace.clone(),
            start_line: line,
            end_line: node.end_position().row as i32 + 1,
            signature,
            modifiers: String::new(),
            param_count,
            container: ctx.owner.clone(),
            module: self.res.module.clone(),
        });
        key
    }

    fn add_class(&mut self, node: Node, kind: &str, name: &str, ctx: &Ctx) -> Option<String> {
        let qual = ctx.qualified(name.trim());
        if qual.is_empty() || self.classes.contains(&qual) {
            return None;
        }
        self.classes.insert(qual);
        Some(self.add_symbol(node, kind, name, ctx, -1))
    }

    fn add_ref(
        &mut self,
        node: Node,
        ctx: &Ctx,
        name: &str,
        receiver: &str,
        kind: &str,
        arity: i32,
    ) {
        let name = strip_call(name);
        if name.is_empty() || ctx.source.is_empty() {
            return;
        }
        let receiver = self.receiver_type(ctx, receiver);
        self.res.refs.push(Ref {
            src_key: ctx.source.clone(),
            name,
            kind: kind.to_string(),
            receiver_type: receiver,
            namespace_hint: String::new(),
            line: node.start_position().row as i32 + 1,
            arity,
        });
    }

    fn receiver_type(&self, ctx: &Ctx, raw: &str) -> String {
        let raw = strip_call(raw);
        let raw = raw.trim().trim_start_matches('!');
        if raw.is_empty() {
            return String::new();
        }
        if matches!(raw, "self" | "this" | "Self" | "$this") {
            return ctx.owner.clone();
        }
        let parts: Vec<&str> = raw.split('.').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return String::new();
        }
        if matches!(parts[0], "self" | "this") {
            if parts.len() >= 2 {
                return ctx
                    .vars
                    .get(parts[1])
                    .cloned()
                    .unwrap_or_else(|| ctx.owner.clone());
            }
            return ctx.owner.clone();
        }
        if let Some(t) = ctx.vars.get(raw) {
            return t.clone();
        }
        if let Some(t) = ctx.vars.get(parts[0]) {
            return t.clone();
        }
        if let Some(alias) = ctx.aliases.get(parts[0]) {
            if alias.is_empty() {
                if parts.len() > 1 {
                    return self.receiver_type(ctx, &parts[1..].join("."));
                }
                return String::new();
            }
            if parts.len() > 1 {
                return format!("{}.{}", alias, parts[1..].join("."));
            }
            return alias.clone();
        }
        if is_upper(parts[0]) {
            return raw.to_string();
        }
        String::new()
    }
}

// ---------------------------------------------------------------- GDScript

fn walk_gdscript(b: &mut Builder, root: Node) {
    let src = b.src;
    let rel = b.res.rel_path.clone();
    let mut ctx = Ctx::default();
    let mut class_seen = false;
    for child in named_children(root) {
        if child.kind() != "class_name_statement" {
            continue;
        }
        if let Some(name_node) = child.child_by_field_name("name") {
            let name = trim_text(name_node, src).to_string();
            if let Some(key) = b.add_class(child, "class", &name, &ctx) {
                ctx.owner = name.clone();
                if let Some(extends) = child.child_by_field_name("extends") {
                    if let Some(base) = descendant(extends, "identifier") {
                        let base = trim_text(base, src).to_string();
                        let mut c = ctx.clone();
                        c.source = key;
                        b.add_ref(extends, &c, &base, "", "inherits", 0);
                    }
                }
            }
            class_seen = true;
        }
        break;
    }
    if !class_seen {
        let stem = Path::new(&rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("script")
            .to_string();
        if let Some(_key) = b.add_class(root, "class", &stem, &ctx) {
            ctx.owner = stem;
        }
    }
    for child in named_children(root) {
        if child.kind() == "class_name_statement" {
            continue;
        }
        gd_node(b, child, &mut ctx, 0);
    }
}

fn gd_node(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    let src = b.src;
    match node.kind() {
        "function_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let mut params: Vec<(String, String)> = Vec::new();
            if let Some(list) = node.child_by_field_name("parameters") {
                for param in named_children(list) {
                    gd_param(param, src, &mut params);
                }
            }
            let kind = if ctx.owner.is_empty() {
                "function"
            } else {
                "method"
            };
            let key = b.add_symbol(node, kind, &name, ctx, params.len() as i32);
            let mut body_ctx = ctx.clone();
            body_ctx.source = key;
            for (name, ty) in params {
                body_ctx.vars.entry(name).or_insert(ty);
            }
            if let Some(body) = node.child_by_field_name("body") {
                for stmt in named_children(body) {
                    gd_node(b, stmt, &mut body_ctx, depth + 1);
                }
            }
        }
        "variable_statement" | "const_statement" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let ty_node = node.child_by_field_name("type");
            let ty = match ty_node {
                Some(t) if t.kind() != "inferred_type" => gd_type_name(t, src),
                _ => String::new(),
            };
            let value = node.child_by_field_name("value");
            let inferred = if ty.is_empty() {
                value.map(|v| gd_infer(v, ctx, src)).unwrap_or_default()
            } else {
                ty
            };
            if ctx.source.is_empty() {
                let kind = if node.kind() == "const_statement" {
                    "constant"
                } else {
                    "property"
                };
                b.add_symbol(node, kind, &name, ctx, -1);
            }
            if !inferred.is_empty() {
                ctx.vars.insert(name, inferred);
            }
            if let Some(v) = value {
                gd_node(b, v, ctx, depth + 1);
            }
        }
        "class_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            if let Some(key) = b.add_class(node, "class", &name, ctx) {
                let qual = ctx.qualified(&name);
                if let Some(extends) = node.child_by_field_name("extends") {
                    if let Some(base) = descendant(extends, "identifier") {
                        let base = trim_text(base, src).to_string();
                        let mut c = ctx.clone();
                        c.source = key;
                        b.add_ref(extends, &c, &base, "", "inherits", 0);
                    }
                }
                let mut inner = ctx.clone();
                inner.owner = qual;
                if let Some(body) = node.child_by_field_name("body") {
                    for member in named_children(body) {
                        gd_node(b, member, &mut inner, depth + 1);
                    }
                }
            }
        }
        "enum_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            if let Some(key) = b.add_class(node, "enum", &name, ctx) {
                let qual = ctx.qualified(&name);
                let mut inner = ctx.clone();
                inner.owner = qual;
                inner.source = key;
                if let Some(body) = node.child_by_field_name("body") {
                    for entry in named_children(body) {
                        if entry.kind() != "enumerator" {
                            continue;
                        }
                        let ename = entry
                            .child_by_field_name("left")
                            .or_else(|| first_identifier(entry))
                            .map(|n| trim_text(n, src).to_string())
                            .unwrap_or_default();
                        b.add_symbol(entry, "enum_member", &ename, &inner, -1);
                    }
                }
            }
        }
        "signal_statement" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let arity = node
                .child_by_field_name("parameters")
                .map(|p| named_children(p).len() as i32)
                .unwrap_or(0);
            b.add_symbol(node, "signal", &name, ctx, arity);
        }
        "call" => gd_call(b, node, ctx, depth),
        "attribute" => gd_attribute(b, node, ctx, depth, false),
        "annotations" | "class_name_statement" | "extends_statement" | "comment" => {}
        "ERROR" | "body" | "block" | "if_statement" | "for_statement" | "while_statement"
        | "match_statement" | "return_statement" | "expression_statement"
        | "with_statement" | "assignment" | "augmented_assignment" => {
            for child in named_children(node) {
                gd_node(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                gd_node(b, child, ctx, depth + 1);
            }
        }
    }
}

fn gd_param(node: Node, src: &[u8], out: &mut Vec<(String, String)>) {
    match node.kind() {
        "typed_parameter" => {
            let name = first_identifier(node)
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let ty = node
                .child_by_field_name("type")
                .map(|t| gd_type_name(t, src))
                .unwrap_or_default();
            if !name.is_empty() {
                out.push((name, ty));
            }
        }
        _ => {
            let name = first_identifier(node)
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            if !name.is_empty() {
                out.push((name, String::new()));
            }
        }
    }
}

fn gd_type_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "inferred_type" => String::new(),
        "type" => descendant(node, "identifier")
            .map(|n| trim_text(n, src).to_string())
            .unwrap_or_else(|| trim_text(node, src).to_string()),
        _ => trim_text(node, src).to_string(),
    }
}

fn gd_infer(node: Node, ctx: &Ctx, src: &[u8]) -> String {
    match node.kind() {
        "attribute" => {
            let children = named_children(node);
            let base = children.first().map(|c| trim_text(*c, src).to_string());
            let call = children.get(1).and_then(|c| first_identifier(*c));
            let method = call.map(|c| trim_text(c, src)).unwrap_or_default();
            if method == "new" {
                return base.unwrap_or_default();
            }
            base.and_then(|b| ctx.vars.get(&b).cloned())
                .unwrap_or_default()
        }
        "call" => {
            let callee = named_children(node)
                .first()
                .map(|c| trim_text(*c, src).to_string())
                .unwrap_or_default();
            ctx.vars.get(&callee).cloned().unwrap_or_default()
        }
        "identifier" => ctx
            .vars
            .get(trim_text(node, src))
            .cloned()
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn gd_call(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let args = node.child_by_field_name("arguments");
    let arity = args.map(|a| named_children(a).len() as i32).unwrap_or(0);
    if let Some(callee) = named_children(node).first().copied() {
        match callee.kind() {
            "identifier" => {
                let name = trim_text(callee, src).to_string();
                let kind = if is_upper(&name) {
                    "constructs"
                } else {
                    "calls"
                };
                b.add_ref(callee, ctx, &name, "", kind, arity);
            }
            "attribute" => gd_attribute(b, callee, ctx, depth, true),
            _ => {}
        }
    }
    if let Some(args) = args {
        for arg in named_children(args) {
            gd_node(b, arg, ctx, depth + 1);
        }
    }
}

fn gd_attribute(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize, is_callee: bool) {
    let src = b.src;
    let children = named_children(node);
    let Some(base) = children.first().copied() else {
        return;
    };
    let Some(acall) = children.get(1).copied() else {
        return;
    };
    if acall.kind() != "attribute_call" {
        return;
    }
    let name = first_identifier(acall)
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    let args = acall.child_by_field_name("arguments");
    let arity = args.map(|a| named_children(a).len() as i32).unwrap_or(0);
    let base_text = trim_text(base, src).to_string();
    if name == "new" {
        b.add_ref(acall, ctx, &base_text, "", "constructs", arity);
    } else if args.is_some() {
        b.add_ref(acall, ctx, &name, &base_text, "calls", arity);
    } else if !is_callee {
        b.add_ref(acall, ctx, &name, &base_text, "references", 0);
    }
    if matches!(base.kind(), "attribute" | "call") {
        gd_node(b, base, ctx, depth + 1);
    }
    if let Some(args) = args {
        for arg in named_children(args) {
            gd_node(b, arg, ctx, depth + 1);
        }
    }
}

// ------------------------------------------------------------------ Kotlin

fn walk_kotlin(b: &mut Builder, root: Node) {
    let src = b.src;
    let mut ctx = Ctx::default();
    for child in named_children(root) {
        if child.kind() == "package_header" {
            let text = trim_text(child, src);
            ctx.namespace = text
                .trim_start_matches("package")
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string();
        }
    }
    for child in named_children(root) {
        if child.kind() == "import" {
            kt_import(b, child, &mut ctx);
        }
    }
    for child in named_children(root) {
        match child.kind() {
            "package_header" | "import" | "file_annotation" | "shebang_line" => {}
            _ => kt_member(b, child, &mut ctx, 0),
        }
    }
}

fn kt_import(b: &mut Builder, node: Node, ctx: &mut Ctx) {
    let src = b.src;
    let text = trim_text(node, src);
    let body = text
        .trim_start_matches("import")
        .trim()
        .trim_end_matches(';')
        .trim();
    if body.is_empty() {
        return;
    }
    let (target, alias) = match body.find(" as ") {
        Some(idx) => (
            body[..idx].trim().to_string(),
            body[idx + 4..].trim().to_string(),
        ),
        None => (body.to_string(), String::new()),
    };
    if target.ends_with(".*") {
        return;
    }
    let alias = if alias.is_empty() {
        target.rsplit('.').next().unwrap_or(&target).to_string()
    } else {
        alias
    };
    ctx.aliases.insert(alias, target);
}

fn kt_member(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    match node.kind() {
        "class_declaration" | "object_declaration" => kt_class(b, node, ctx, depth),
        "function_declaration" => kt_function(b, node, ctx, depth),
        "property_declaration" => kt_property(b, node, ctx, depth),
        "companion_object" => {
            if let Some(body) = node.child_by_field_name("body") {
                for member in named_children(body) {
                    kt_member(b, member, ctx, depth + 1);
                }
            }
        }
        "secondary_constructor" => {
            if let Some(body) = node.child_by_field_name("body") {
                let mut fctx = ctx.clone();
                let name = ctx
                    .owner
                    .rsplit('.')
                    .next()
                    .unwrap_or("init")
                    .to_string();
                let key = b.add_symbol(node, "constructor", &name, ctx, -1);
                fctx.source = key;
                for stmt in named_children(body) {
                    kt_member(b, stmt, &mut fctx, depth + 1);
                }
            }
        }
        "enum_entry" => {}
        "call_expression" => kt_call(b, node, ctx, depth),
        "navigation_expression" => kt_navigation(b, node, ctx, depth),
        "ERROR" | "block" | "statements" | "function_body" | "init_block"
        | "assignment" | "binary_expression" | "if_expression" | "when_expression"
        | "for_statement" | "while_statement" | "do_while_statement" | "try_expression"
        | "catch_block" | "finally_block" | "lambda_literal" | "value_argument"
        | "return_expression" | "control_structure_body" => {
            for child in named_children(node) {
                kt_member(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                kt_member(b, child, ctx, depth + 1);
            }
        }
    }
}

fn kt_class(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = trim_text(name_node, src).to_string();
    let kind = if node.kind() == "object_declaration" {
        "class"
    } else {
        let mut kind = "class";
        for child in all_children(node) {
            if child.kind() == "interface" {
                kind = "interface";
            }
            if child.kind() == "modifiers" {
                for m in all_children(child) {
                    if m.kind() == "class_modifier" {
                        match trim_text(m, src) {
                            "enum" => kind = "enum",
                            "annotation" => kind = "interface",
                            _ => {}
                        }
                    }
                }
            }
        }
        kind
    };
    let key = match b.add_class(node, kind, &name, ctx) {
        Some(key) => key,
        None => return,
    };
    let qual = ctx.qualified(&name);
    let mut member_ctx = ctx.clone();
    member_ctx.owner = qual.clone();
    if let Some(params) = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "primary_constructor")
    {
        if let Some(list) = descendant(params, "class_parameters") {
            for p in named_children(list) {
                if p.kind() != "class_parameter" {
                    continue;
                }
                let pname = first_identifier(p)
                    .map(|n| trim_text(n, src).to_string())
                    .unwrap_or_default();
                let ptype = descendant(p, "user_type")
                    .map(|n| trim_text(n, src).to_string())
                    .unwrap_or_default();
                if !pname.is_empty() {
                    b.add_symbol(p, "property", &pname, &member_ctx, -1);
                    if !ptype.is_empty() {
                        member_ctx.vars.insert(pname, ptype);
                    }
                }
            }
        }
    }
    if let Some(specs) = descendant(node, "delegation_specifiers") {
        let mut class_ctx = ctx.clone();
        class_ctx.source = key.clone();
        for spec in named_children(specs) {
            let candidate = descendant(spec, "user_type")
                .or_else(|| descendant(spec, "constructor_invocation"));
            if let Some(t) = candidate {
                let base = trim_text(t, src).split('(').next().unwrap_or("").trim().to_string();
                if !base.is_empty() {
                    b.add_ref(spec, &class_ctx, &base, "", "inherits", 0);
                }
            }
        }
    }
    let body = named_children(node)
        .into_iter()
        .find(|c| matches!(c.kind(), "class_body" | "enum_class_body"));
    let Some(body) = body else {
        return;
    };
    if body.kind() == "enum_class_body" {
        let mut enum_ctx = member_ctx.clone();
        enum_ctx.source = key.clone();
        for entry in named_children(body) {
            if entry.kind() == "enum_entry" {
                let ename = first_identifier(entry)
                    .map(|n| trim_text(n, src).to_string())
                    .unwrap_or_default();
                b.add_symbol(entry, "enum_member", &ename, &enum_ctx, -1);
            } else {
                kt_member(b, entry, &mut member_ctx, depth + 1);
            }
        }
        return;
    }
    for member in named_children(body) {
        kt_member(b, member, &mut member_ctx, depth + 1);
    }
}

fn kt_function(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    let params_node = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "function_value_parameters");
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(list) = params_node {
        for p in named_children(list) {
            if p.kind() != "parameter" && p.kind() != "class_parameter" {
                continue;
            }
            let pname = first_identifier(p)
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let ptype = descendant(p, "user_type")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            if !pname.is_empty() {
                params.push((pname, ptype));
            }
        }
    }
    let kind = if ctx.owner.is_empty() {
        "function"
    } else {
        "method"
    };
    let key = b.add_symbol(node, kind, &name, ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, ptype) in params {
        if !ptype.is_empty() {
            body_ctx.vars.insert(pname, ptype);
        }
    }
    let mut walked_body = false;
    if let Some(body) = node.child_by_field_name("body") {
        walked_body = true;
        for stmt in named_children(body) {
            kt_member(b, stmt, &mut body_ctx, depth + 1);
        }
    }
    if !walked_body {
        for child in named_children(node) {
            match child.kind() {
                "modifiers" | "function_value_parameters" | "user_type" | "type_parameters"
                | "name" => {}
                _ => kt_member(b, child, &mut body_ctx, depth + 1),
            }
        }
    }
}

fn kt_property(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let var_decl = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "variable_declaration");
    let Some(var_decl) = var_decl else {
        return;
    };
    let name = first_identifier(var_decl)
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let declared = descendant(var_decl, "user_type")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    let mut value_expr = None;
    for child in named_children(node) {
        if child.id() == var_decl.id() || child.kind() == "user_type" {
            continue;
        }
        value_expr = Some(child);
    }
    let inferred = if declared.is_empty() {
        value_expr
            .map(|v| kt_infer(b, v, ctx))
            .unwrap_or_default()
    } else {
        declared
    };
    if ctx.source.is_empty() {
        b.add_symbol(node, "property", &name, ctx, -1);
    }
    if !inferred.is_empty() {
        ctx.vars.insert(name, inferred);
    }
    if let Some(value) = value_expr {
        kt_member(b, value, ctx, depth + 1);
    }
}

fn kt_infer(b: &Builder, node: Node, ctx: &Ctx) -> String {
    let src = b.src;
    match node.kind() {
        "call_expression" => {
            let callee = named_children(node).first().copied();
            let name = callee
                .map(|c| trim_text(c, src).to_string())
                .unwrap_or_default();
            if is_upper(&name) {
                name
            } else if let Some(t) = ctx.vars.get(&name) {
                t.clone()
            } else {
                String::new()
            }
        }
        "identifier" => ctx
            .vars
            .get(trim_text(node, src))
            .cloned()
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn kt_call(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let children = named_children(node);
    let callee = children.first().copied();
    let args = children.get(1).copied().filter(|c| c.kind() == "value_arguments");
    let arity = args
        .map(|a| {
            named_children(a)
                .iter()
                .filter(|c| c.kind() == "value_argument")
                .count() as i32
        })
        .unwrap_or(0);
    if let Some(callee) = callee {
        match callee.kind() {
            "identifier" => {
                let name = trim_text(callee, src).to_string();
                let kind = if is_upper(&name) {
                    "constructs"
                } else {
                    "calls"
                };
                b.add_ref(callee, ctx, &name, "", kind, arity);
            }
            "navigation_expression" => {
                let text = trim_text(callee, src);
                if let Some(idx) = text.rfind('.') {
                    let receiver = &text[..idx];
                    let name = &text[idx + 1..];
                    b.add_ref(callee, ctx, name, receiver, "calls", arity);
                }
            }
            _ => {}
        }
    }
    if let Some(args) = args {
        for arg in named_children(args) {
            kt_member(b, arg, ctx, depth + 1);
        }
    }
}

fn kt_navigation(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let text = trim_text(node, src);
    if let Some(idx) = text.rfind('.') {
        let receiver = &text[..idx];
        let name = &text[idx + 1..];
        if is_upper(name) || is_upper(receiver.split('.').next().unwrap_or("")) {
            b.add_ref(node, ctx, name, receiver, "references", 0);
        }
    }
    for child in named_children(node) {
        if matches!(child.kind(), "call_expression" | "lambda_literal") {
            kt_member(b, child, ctx, depth + 1);
        }
    }
}

// ------------------------------------------------------------------- Swift

fn walk_swift(b: &mut Builder, root: Node) {
    let mut ctx = Ctx::default();
    for child in named_children(root) {
        swift_member(b, child, &mut ctx, 0);
    }
}

fn swift_member(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    match node.kind() {
        "class_declaration" => swift_class(b, node, ctx, depth),
        "protocol_declaration" => swift_protocol(b, node, ctx, depth),
        "function_declaration" => swift_function(b, node, ctx, depth),
        "property_declaration" => swift_property(b, node, ctx, depth),
        "init_declaration" => swift_init(b, node, ctx, depth),
        "deinit_declaration" => {
            let key = b.add_symbol(node, "destructor", "deinit", ctx, 0);
            let mut body_ctx = ctx.clone();
            body_ctx.source = key;
            if let Some(body) = node.child_by_field_name("body") {
                for stmt in named_children(body) {
                    swift_member(b, stmt, &mut body_ctx, depth + 1);
                }
            }
        }
        "import_declaration" | "comment" | "line_comment" | "block_comment" => {}
        "enum_entry" => {}
        "call_expression" => swift_call(b, node, ctx, depth),
        "navigation_expression" => swift_navigation(b, node, ctx, depth),
        "ERROR" | "function_body" | "statements" | "block" | "assignment"
        | "expression_statement" | "if_statement" | "for_statement" | "while_statement"
        | "guard_statement" | "switch_statement" | "do_statement" | "return_statement"
        | "value_argument" => {
            for child in named_children(node) {
                swift_member(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                swift_member(b, child, ctx, depth + 1);
            }
        }
    }
}

fn swift_class(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = trim_text(name_node, src).to_string();
    let decl_kind = node
        .child_by_field_name("declaration_kind")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_else(|| "class".to_string());
    if decl_kind == "extension" {
        let base = name.clone();
        let mut inner = ctx.clone();
        inner.owner = if ctx.namespace.is_empty() {
            base
        } else {
            format!("{}.{}", ctx.namespace, base)
        };
        if let Some(body) = node.child_by_field_name("body") {
            for member in named_children(body) {
                swift_member(b, member, &mut inner, depth + 1);
            }
        }
        return;
    }
    let kind = match decl_kind.as_str() {
        "struct" => "struct",
        "enum" => "enum",
        "protocol" => "interface",
        _ => "class",
    };
    let key = match b.add_class(node, kind, &name, ctx) {
        Some(key) => key,
        None => return,
    };
    let qual = ctx.qualified(&name);
    let mut member_ctx = ctx.clone();
    member_ctx.owner = qual.clone();
    let mut class_ctx = ctx.clone();
    class_ctx.source = key.clone();
    for spec in named_children(node) {
        if spec.kind() != "inheritance_specifier" {
            continue;
        }
        if let Some(base) = spec.child_by_field_name("inherits_from") {
            let base = trim_text(base, src).to_string();
            if !base.is_empty() {
                b.add_ref(spec, &class_ctx, &base, "", "inherits", 0);
            }
        }
    }
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    if body.kind() == "enum_class_body" {
        let mut enum_ctx = member_ctx.clone();
        enum_ctx.source = key;
        for entry in named_children(body) {
            if entry.kind() != "enum_entry" {
                swift_member(b, entry, &mut member_ctx, depth + 1);
                continue;
            }
            for case in named_children(entry) {
                if case.kind() != "simple_identifier" {
                    continue;
                }
                let cname = trim_text(case, src).to_string();
                b.add_symbol(case, "enum_member", &cname, &enum_ctx, -1);
            }
        }
        return;
    }
    for member in named_children(body) {
        swift_member(b, member, &mut member_ctx, depth + 1);
    }
}

fn swift_protocol(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = trim_text(name_node, src).to_string();
    if b.add_class(node, "interface", &name, ctx).is_none() {
        return;
    }
    let qual = ctx.qualified(&name);
    let mut member_ctx = ctx.clone();
    member_ctx.owner = qual;
    if let Some(body) = node.child_by_field_name("body") {
        for member in named_children(body) {
            swift_member(b, member, &mut member_ctx, depth + 1);
        }
    }
}

fn swift_param(node: Node, src: &[u8]) -> Option<(String, String)> {
    let mut name = String::new();
    let mut ty = String::new();
    for child in named_children(node) {
        match child.kind() {
            "simple_identifier" => name = trim_text(child, src).to_string(),
            "user_type" => ty = trim_text(child, src).to_string(),
            _ => {}
        }
    }
    if name == "_" || name.is_empty() {
        return None;
    }
    Some((name, ty))
}

fn swift_function(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    let mut params: Vec<(String, String)> = Vec::new();
    for child in named_children(node) {
        if child.kind() == "parameter" {
            if let Some(p) = swift_param(child, src) {
                params.push(p);
            }
        }
    }
    let kind = if ctx.owner.is_empty() {
        "function"
    } else {
        "method"
    };
    let key = b.add_symbol(node, kind, &name, ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, ptype) in params {
        if !ptype.is_empty() {
            body_ctx.vars.insert(pname, ptype);
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        for stmt in named_children(body) {
            swift_member(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn swift_property(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("name")
        .map(|n| {
            descendant(n, "bound_identifier")
                .map(|b| trim_text(b, src).to_string())
                .unwrap_or_else(|| trim_text(n, src).to_string())
        })
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let declared = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "type_annotation")
        .and_then(|t| descendant(t, "user_type"))
        .map(|t| trim_text(t, src).to_string())
        .unwrap_or_default();
    let value = node.child_by_field_name("value");
    let inferred = if declared.is_empty() {
        value
            .map(|v| swift_infer(b, v, ctx))
            .unwrap_or_default()
    } else {
        declared
    };
    if ctx.source.is_empty() {
        b.add_symbol(node, "property", &name, ctx, -1);
    }
    if !inferred.is_empty() {
        ctx.vars.insert(name, inferred);
    }
    if let Some(value) = value {
        swift_member(b, value, ctx, depth + 1);
    }
}

fn swift_init(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let mut params: Vec<(String, String)> = Vec::new();
    for child in named_children(node) {
        if child.kind() == "parameter" {
            if let Some(p) = swift_param(child, src) {
                params.push(p);
            }
        }
    }
    let key = b.add_symbol(node, "constructor", "init", ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, ptype) in params {
        if !ptype.is_empty() {
            body_ctx.vars.insert(pname, ptype);
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        for stmt in named_children(body) {
            swift_member(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn swift_infer(b: &Builder, node: Node, ctx: &Ctx) -> String {
    let src = b.src;
    match node.kind() {
        "call_expression" => {
            let callee = named_children(node).first().copied();
            let name = callee
                .map(|c| trim_text(c, src).to_string())
                .unwrap_or_default();
            if is_upper(&name) {
                name
            } else if let Some(t) = ctx.vars.get(&name) {
                t.clone()
            } else {
                String::new()
            }
        }
        "simple_identifier" => ctx
            .vars
            .get(trim_text(node, src))
            .cloned()
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn swift_navigation(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let text = trim_text(node, src);
    if let Some(idx) = text.rfind('.') {
        let receiver = &text[..idx];
        let name = &text[idx + 1..];
        if is_upper(name) || is_upper(receiver.split('.').next().unwrap_or("")) {
            b.add_ref(node, ctx, name, receiver, "references", 0);
        }
    }
    for child in named_children(node) {
        if matches!(child.kind(), "call_expression") {
            swift_member(b, child, ctx, depth + 1);
        }
    }
}

fn swift_call(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let children = named_children(node);
    let callee = children.first().copied();
    let args = descendant(node, "value_arguments");
    let arity = args
        .map(|a| {
            named_children(a)
                .iter()
                .filter(|c| c.kind() == "value_argument")
                .count() as i32
        })
        .unwrap_or(0);
    if let Some(callee) = callee {
        match callee.kind() {
            "simple_identifier" => {
                let name = trim_text(callee, src).to_string();
                let kind = if is_upper(&name) {
                    "constructs"
                } else {
                    "calls"
                };
                b.add_ref(callee, ctx, &name, "", kind, arity);
            }
            "navigation_expression" => {
                let text = trim_text(callee, src);
                if let Some(idx) = text.rfind('.') {
                    b.add_ref(callee, ctx, &text[idx + 1..], &text[..idx], "calls", arity);
                }
            }
            _ => {}
        }
    }
    if let Some(args) = args {
        for arg in named_children(args) {
            swift_member(b, arg, ctx, depth + 1);
        }
    }
}

// ------------------------------------------------------------- Objective-C

fn walk_objc(b: &mut Builder, root: Node) {
    let mut ctx = Ctx::default();
    for child in named_children(root) {
        objc_top(b, child, &mut ctx, 0);
    }
}

fn objc_top(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    match node.kind() {
        "class_interface" | "class_implementation" => objc_class(b, node, ctx, depth),
        "category_interface" | "category_implementation" => objc_class(b, node, ctx, depth),
        "protocol_declaration" => objc_protocol(b, node, ctx, depth),
        "function_definition" => objc_function(b, node, ctx, depth),
        "struct_specifier" => objc_struct(b, node, ctx, depth),
        "enum_specifier" => objc_enum(b, node, ctx, depth),
        "preproc_include" | "preproc_def" | "preproc_function_def" | "comment" => {}
        "ERROR" => {
            for child in named_children(node) {
                objc_top(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                objc_top(b, child, ctx, depth + 1);
            }
        }
    }
}

fn objc_class_name(node: Node, src: &[u8]) -> Option<String> {
    for child in named_children(node) {
        if child.kind() == "identifier" {
            return Some(trim_text(child, src).to_string());
        }
    }
    None
}

fn objc_class(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name) = objc_class_name(node, src) else {
        return;
    };
    let is_interface = node.kind() == "class_interface" || node.kind() == "protocol_declaration";
    let is_category = node.kind().starts_with("category_");
    if !is_category {
        b.add_class(node, "class", &name, ctx);
    }
    let qual = ctx.qualified(&name);
    let mut inner = ctx.clone();
    inner.owner = qual.clone();
    if is_interface {
        if let Some(base) = node.child_by_field_name("superclass") {
            let base = trim_text(base, src).to_string();
            if !base.is_empty() {
                let mut c = ctx.clone();
                c.source = String::new();
                // inherit edges are anchored on the class symbol when present
                if let Some(class) = b
                    .res
                    .symbols
                    .iter()
                    .find(|s| s.qualname == qual && s.kind == "class")
                {
                    c.source = class.key.clone();
                }
                if !c.source.is_empty() {
                    b.add_ref(node, &c, &base, "", "inherits", 0);
                }
            }
        }
        for child in named_children(node) {
            match child.kind() {
                "method_declaration" => objc_method(b, child, &mut inner, depth + 1, false),
                "property_declaration" => objc_property(b, child, &mut inner, depth + 1),
                _ => {}
            }
        }
        return;
    }
    for child in named_children(node) {
        if child.kind() == "implementation_definition" {
            for member in named_children(child) {
                if member.kind() == "method_definition" {
                    objc_method(b, member, &mut inner, depth + 1, true);
                }
            }
        } else if child.kind() == "property_declaration" {
            objc_property(b, child, &mut inner, depth + 1);
        }
    }
}

fn objc_protocol(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name) = objc_class_name(node, src) else {
        return;
    };
    if b.add_class(node, "interface", &name, ctx).is_none() {
        return;
    }
    let qual = ctx.qualified(&name);
    let mut inner = ctx.clone();
    inner.owner = qual;
    for child in named_children(node) {
        if child.kind() == "method_declaration" {
            objc_method(b, child, &mut inner, depth + 1, false);
        }
    }
}

fn objc_selector<'a>(node: Node<'a>, src: &'a [u8]) -> (String, Vec<Node<'a>>) {
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<Node> = Vec::new();
    for child in named_children(node) {
        match child.kind() {
            "identifier" => parts.push(trim_text(child, src).to_string()),
            "method_parameter" => params.push(child),
            _ => {}
        }
    }
    let mut name = parts.join(":");
    if !params.is_empty() {
        name.push(':');
    }
    (name, params)
}

fn objc_method(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize, has_body: bool) {
    let src = b.src;
    let (name, params) = objc_selector(node, src);
    if name.is_empty() {
        return;
    }
    let mut vars: Vec<(String, String)> = Vec::new();
    for param in &params {
        let ty = descendant(*param, "type_identifier")
            .or_else(|| descendant(*param, "primitive_type"))
            .map(|n| trim_text(n, src).to_string())
            .unwrap_or_default();
        let pname = direct_identifier(*param, src).unwrap_or_default();
        if !pname.is_empty() {
            vars.push((pname, ty));
        }
    }
    let qual = ctx.qualified(&name);
    let known = b.classes.contains(&format!("{qual}|method"));
    let key = if known {
        b.res
            .symbols
            .iter()
            .find(|s| s.qualname == qual)
            .map(|s| s.key.clone())
            .unwrap_or_default()
    } else {
        b.classes.insert(format!("{qual}|method"));
        b.add_symbol(node, "method", &name, ctx, params.len() as i32)
    };
    if !has_body {
        return;
    }
    let mut body_ctx = ctx.clone();
    if !key.is_empty() {
        body_ctx.source = key;
    }
    for (pname, pty) in vars {
        if !pty.is_empty() {
            body_ctx.vars.insert(pname, pty);
        }
    }
    if let Some(body) = descendant(node, "compound_statement") {
        for stmt in named_children(body) {
            objc_stmt(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn direct_identifier(node: Node, src: &[u8]) -> Option<String> {
    for child in named_children(node) {
        if child.kind() == "identifier" {
            return Some(trim_text(child, src).to_string());
        }
    }
    None
}

fn objc_property(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let decl = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "struct_declaration");
    let Some(decl) = decl else {
        return;
    };
    let ty = descendant(decl, "type_identifier")
        .map(|n| trim_text(n, src).to_string())
        .or_else(|| descendant(decl, "primitive_type").map(|n| trim_text(n, src).to_string()))
        .unwrap_or_default();
    for child in named_children(decl) {
        if child.kind() != "struct_declarator" {
            continue;
        }
        let name = direct_identifier(child, src).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        b.add_symbol(child, "property", &name, ctx, -1);
        if !ty.is_empty() {
            ctx.vars.insert(name, ty.clone());
        }
    }
    let _ = depth;
}

fn objc_function(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("declarator")
        .and_then(|d| descendant(d, "identifier"))
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(decl) = node.child_by_field_name("declarator") {
        let mut param_nodes = Vec::new();
        descendants_of(decl, "parameter_declaration", &mut param_nodes, 0);
        for p in param_nodes {
            let ty = descendant(p, "type_identifier")
                .or_else(|| descendant(p, "primitive_type"))
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let pname = descendant(p, "identifier")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            if !pname.is_empty() {
                params.push((pname, ty));
            }
        }
    }
    let key = b.add_symbol(node, "function", &name, ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, pty) in params {
        if !pty.is_empty() {
            body_ctx.vars.insert(pname, pty);
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        for stmt in named_children(body) {
            objc_stmt(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn objc_struct(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    if b.add_class(node, "struct", &name, ctx).is_none() {
        return;
    }
    let qual = ctx.qualified(&name);
    let mut inner = ctx.clone();
    inner.owner = qual;
    if let Some(body) = node.child_by_field_name("body") {
        let mut fields = Vec::new();
        descendants_of(body, "field_declaration", &mut fields, 0);
        for field in fields {
            let ty = descendant(field, "type_identifier")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let mut decls = Vec::new();
            descendants_of(field, "field_identifier", &mut decls, 0);
            for decl in decls {
                let fname = trim_text(decl, src).to_string();
                b.add_symbol(decl, "field", &fname, &inner, -1);
                if !ty.is_empty() {
                    inner.vars.insert(fname, ty.clone());
                }
            }
        }
    }
    let _ = depth;
}

fn objc_enum(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let name = node
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    if b.add_class(node, "enum", &name, ctx).is_none() {
        return;
    }
    let qual = ctx.qualified(&name);
    let mut inner = ctx.clone();
    inner.owner = qual;
    if let Some(body) = node.child_by_field_name("body") {
        for entry in named_children(body) {
            if entry.kind() != "enumerator" {
                continue;
            }
            let ename = direct_identifier(entry, src)
                .or_else(|| descendant(entry, "identifier").map(|n| trim_text(n, src).to_string()))
                .unwrap_or_default();
            b.add_symbol(entry, "enum_member", &ename, &inner, -1);
        }
    }
    let _ = depth;
}

fn objc_stmt(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    let src = b.src;
    match node.kind() {
        "message_expression" => objc_message(b, node, ctx, depth),
        "call_expression" => {
            let name = node
                .child_by_field_name("function")
                .map(|f| {
                    let t = trim_text(f, src);
                    t.rsplit('.').next().unwrap_or(t).to_string()
                })
                .unwrap_or_default();
            let args = node.child_by_field_name("arguments");
            let arity = args.map(|a| named_children(a).len() as i32).unwrap_or(0);
            b.add_ref(node, ctx, &name, "", "calls", arity);
            if let Some(args) = args {
                for arg in named_children(args) {
                    objc_stmt(b, arg, ctx, depth + 1);
                }
            }
        }
        "property_declaration" => objc_property(b, node, ctx, depth),
        "declaration" => {
            if let Some(ty) = descendant(node, "type_identifier") {
                let ty = trim_text(ty, src).to_string();
                let mut decls = Vec::new();
                descendants_of(node, "identifier", &mut decls, 0);
                if let Some(name) = decls.last() {
                    let name = trim_text(*name, src).to_string();
                    if !name.is_empty() {
                        ctx.vars.insert(name, ty);
                    }
                }
            }
            for child in named_children(node) {
                objc_stmt(b, child, ctx, depth + 1);
            }
        }
        "assignment_expression" | "field_expression" | "compound_statement"
        | "expression_statement" | "if_statement" | "for_statement" | "while_statement"
        | "switch_statement" | "return_statement" | "do_statement" => {
            for child in named_children(node) {
                objc_stmt(b, child, ctx, depth + 1);
            }
        }
        "ERROR" => {
            for child in named_children(node) {
                objc_stmt(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                objc_stmt(b, child, ctx, depth + 1);
            }
        }
    }
}

fn objc_message(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let mut receiver = String::new();
    let mut parts: Vec<String> = Vec::new();
    let mut args: Vec<Node> = Vec::new();
    for (i, child) in all_children(node).iter().enumerate() {
        if !child.is_named() {
            continue;
        }
        match node.field_name_for_child(i as u32) {
            Some("receiver") => receiver = trim_text(*child, src).to_string(),
            Some("method") => parts.push(trim_text(*child, src).to_string()),
            Some(_) => {}
            None => args.push(*child),
        }
    }
    if parts.is_empty() {
        return;
    }
    let mut name = parts.join(":");
    if !args.is_empty() {
        name.push(':');
    }
    let arity = parts.len() as i32;
    b.add_ref(node, ctx, &name, &receiver, "calls", arity);
    for arg in args {
        objc_stmt(b, arg, ctx, depth + 1);
    }
}

// -------------------------------------------------------------------- Dart

fn walk_dart(b: &mut Builder, root: Node) {
    let src = b.src;
    let mut ctx = Ctx::default();
    for child in named_children(root) {
        if child.kind() != "import_or_export" {
            continue;
        }
        let alias = child
            .child_by_field_name("alias")
            .map(|n| trim_text(n, src).to_string());
        if let Some(alias) = alias {
            ctx.aliases.insert(alias, String::new());
        }
    }
    for child in named_children(root) {
        dart_member(b, child, &mut ctx, 0);
    }
}

fn dart_member(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    if !b.tick(depth) {
        return;
    }
    match node.kind() {
        "class_declaration" => dart_class(b, node, ctx, depth),
        "enum_declaration" => dart_enum(b, node, ctx, depth),
        "function_declaration" => dart_function(b, node, ctx, depth),
        "method_declaration" => dart_method(b, node, ctx, depth),
        "declaration" => dart_declaration(b, node, ctx, depth),
        "call_expression" => dart_call(b, node, ctx, depth),
        "member_expression" => {
            let src = b.src;
            let text = trim_text(node, src);
            if let Some(idx) = text.rfind('.') {
                let receiver = &text[..idx];
                let name = &text[idx + 1..];
                if is_upper(name) || is_upper(receiver.split('.').next().unwrap_or("")) {
                    b.add_ref(node, ctx, name, receiver, "references", 0);
                }
            }
        }
        "local_variable_declaration" => {
            let src = b.src;
            if let Some(init) = named_children(node)
                .into_iter()
                .find(|c| c.kind() == "initialized_variable_declaration")
            {
                let name = init
                    .child_by_field_name("name")
                    .map(|n| trim_text(n, src).to_string())
                    .unwrap_or_default();
                let value = init.child_by_field_name("value");
                if !name.is_empty() {
                    if let Some(value) = value {
                        let inferred = dart_infer(b, value, ctx);
                        if !inferred.is_empty() {
                            ctx.vars.insert(name, inferred);
                        }
                        dart_member(b, value, ctx, depth + 1);
                    }
                }
            }
        }
        "import_or_export" | "comment" | "part_directive" | "part_of_directive"
        | "library_directive" => {}
        "ERROR" | "class_body" | "block" | "function_body" | "member_declaration"
        | "assignment_expression" | "assignment" | "expression_statement"
        | "if_statement" | "for_statement" | "while_statement" | "do_statement"
        | "switch_statement" | "return_statement" | "try_statement" | "catch_clause"
        | "initialized_variable_declaration" => {
            for child in named_children(node) {
                dart_member(b, child, ctx, depth + 1);
            }
        }
        _ => {
            for child in named_children(node) {
                dart_member(b, child, ctx, depth + 1);
            }
        }
    }
}

fn dart_class(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = trim_text(name_node, src).to_string();
    let key = match b.add_class(node, "class", &name, ctx) {
        Some(key) => key,
        None => return,
    };
    let qual = ctx.qualified(&name);
    let mut member_ctx = ctx.clone();
    member_ctx.owner = qual.clone();
    let mut class_ctx = ctx.clone();
    class_ctx.source = key.clone();
    if let Some(superclass) = node.child_by_field_name("superclass") {
        let base = descendant(superclass, "type_identifier")
            .map(|n| trim_text(n, src).to_string())
            .unwrap_or_default();
        if !base.is_empty() {
            b.add_ref(superclass, &class_ctx, &base, "", "inherits", 0);
        }
    }
    let Some(body) = node.child_by_field_name("body") else {
        return;
    };
    for member in named_children(body) {
        dart_member(b, member, &mut member_ctx, depth + 1);
    }
}

fn dart_enum(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let name = trim_text(name_node, src).to_string();
    let key = match b.add_class(node, "enum", &name, ctx) {
        Some(key) => key,
        None => return,
    };
    let qual = ctx.qualified(&name);
    let mut inner = ctx.clone();
    inner.owner = qual;
    inner.source = key;
    if let Some(body) = node.child_by_field_name("body") {
        for entry in named_children(body) {
            if entry.kind() != "enum_entry" {
                continue;
            }
            let ename = entry
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            b.add_symbol(entry, "enum_member", &ename, &inner, -1);
        }
    }
    let _ = depth;
}

fn dart_function(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let signature = node
        .child_by_field_name("signature")
        .or_else(|| named_children(node).into_iter().find(|c| c.kind() == "function_signature"));
    let Some(signature) = signature else {
        return;
    };
    let name = signature
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let params = dart_params(signature, src);
    let kind = if ctx.owner.is_empty() {
        "function"
    } else {
        "method"
    };
    let key = b.add_symbol(node, kind, &name, ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, ptype) in params {
        if !ptype.is_empty() {
            body_ctx.vars.insert(pname, ptype);
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        for stmt in named_children(body) {
            dart_member(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn dart_params(signature: Node, src: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let list = signature
        .child_by_field_name("parameters")
        .or_else(|| descendant(signature, "formal_parameter_list"));
    let Some(list) = list else {
        return out;
    };
    for param in named_children(list) {
        if param.kind() != "formal_parameter" {
            continue;
        }
        let pname = param
            .child_by_field_name("name")
            .map(|n| trim_text(n, src).to_string())
            .or_else(|| direct_identifier(param, src))
            .unwrap_or_default();
        let ptype = descendant(param, "type_identifier")
            .map(|n| trim_text(n, src).to_string())
            .unwrap_or_default();
        if !pname.is_empty() {
            out.push((pname, ptype));
        }
    }
    out
}

fn dart_declaration(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    if let Some(ctor) = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "constructor_signature")
    {
        let name = ctor
            .child_by_field_name("name")
            .map(|n| trim_text(n, src).to_string())
            .unwrap_or_default();
        let params = dart_params(ctor, src);
        let key = b.add_symbol(ctor, "constructor", &name, ctx, params.len() as i32);
        let mut body_ctx = ctx.clone();
        body_ctx.source = key;
        for (pname, ptype) in params {
            if !ptype.is_empty() {
                body_ctx.vars.insert(pname, ptype);
            }
        }
        let mut body = None;
        let mut parent = ctor.parent();
        while let Some(p) = parent {
            if p.kind() == "member_declaration" {
                body = p.child_by_field_name("body");
                break;
            }
            parent = p.parent();
        }
        if let Some(body) = body {
            for stmt in named_children(body) {
                dart_member(b, stmt, &mut body_ctx, depth + 1);
            }
        }
        return;
    }
    let ty = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "type")
        .and_then(|t| descendant(t, "type_identifier"))
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if let Some(list) = named_children(node)
        .into_iter()
        .find(|c| c.kind() == "initialized_identifier_list")
    {
        for ident in named_children(list) {
            if ident.kind() != "initialized_identifier" {
                continue;
            }
            let name = ident
                .child_by_field_name("name")
                .map(|n| trim_text(n, src).to_string())
                .unwrap_or_default();
            let value = ident.child_by_field_name("value");
            let inferred = if ty.is_empty() {
                value
                    .map(|v| dart_infer(b, v, ctx))
                    .unwrap_or_default()
            } else {
                ty.clone()
            };
            if !name.is_empty() {
                if ctx.source.is_empty() {
                    b.add_symbol(ident, "property", &name, ctx, -1);
                }
                if !inferred.is_empty() {
                    ctx.vars.insert(name, inferred);
                }
            }
            if let Some(value) = value {
                dart_member(b, value, ctx, depth + 1);
            }
        }
    }
}

fn dart_method(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let signature = node
        .child_by_field_name("signature")
        .or_else(|| descendant(node, "method_signature"));
    let Some(signature) = signature else {
        return;
    };
    let fn_sig = descendant(signature, "function_signature").unwrap_or(signature);
    let name = fn_sig
        .child_by_field_name("name")
        .map(|n| trim_text(n, src).to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }
    let params = dart_params(fn_sig, src);
    let key = b.add_symbol(node, "method", &name, ctx, params.len() as i32);
    let mut body_ctx = ctx.clone();
    body_ctx.source = key;
    for (pname, ptype) in params {
        if !ptype.is_empty() {
            body_ctx.vars.insert(pname, ptype);
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        for stmt in named_children(body) {
            dart_member(b, stmt, &mut body_ctx, depth + 1);
        }
    }
}

fn dart_infer(b: &Builder, node: Node, ctx: &Ctx) -> String {
    let src = b.src;
    match node.kind() {
        "call_expression" => {
            let callee = named_children(node)
                .into_iter()
                .find(|c| c.kind() == "identifier")
                .map(|c| trim_text(c, src).to_string())
                .unwrap_or_default();
            if is_upper(&callee) {
                callee
            } else {
                ctx.vars.get(&callee).cloned().unwrap_or_default()
            }
        }
        "identifier" => ctx
            .vars
            .get(trim_text(node, src))
            .cloned()
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn dart_call(b: &mut Builder, node: Node, ctx: &mut Ctx, depth: usize) {
    let src = b.src;
    let function = node.child_by_field_name("function");
    let args = node.child_by_field_name("arguments");
    let arity = args.map(|a| named_children(a).len() as i32).unwrap_or(0);
    if let Some(function) = function {
        match function.kind() {
            "identifier" => {
                let name = trim_text(function, src).to_string();
                let kind = if is_upper(&name) {
                    "constructs"
                } else {
                    "calls"
                };
                b.add_ref(function, ctx, &name, "", kind, arity);
            }
            "member_expression" => {
                let text = trim_text(function, src);
                if let Some(idx) = text.rfind('.') {
                    b.add_ref(function, ctx, &text[idx + 1..], &text[..idx], "calls", arity);
                }
            }
            _ => {}
        }
    }
    if let Some(args) = args {
        for arg in named_children(args) {
            dart_member(b, arg, ctx, depth + 1);
        }
    }
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
    fn gdscript_extracts_classes_signals_and_calls() {
        let src = r#"
class_name Player
extends CharacterBody2D

signal died
enum State { IDLE, RUN }
const SPEED := 10.0
var health: int = 100
var state: State = State.IDLE

func _ready() -> void:
	take_damage(10)

func take_damage(amount: int) -> void:
	health -= amount
	emit_signal("died")

class Bag:
	func add_item(item):
		print(item)
"#;
        let fp = parse("scripts/Player.gd", "app", "gdscript", src.as_bytes());
        assert!(fp.errors.is_empty(), "{:?}", fp.errors);
        assert_eq!(fp.lang, "gdscript");
        assert!(has_symbol(&fp, "Player"));
        assert!(has_symbol(&fp, "Player.take_damage"));
        assert!(has_symbol(&fp, "Player.health"));
        assert!(has_symbol(&fp, "Player.State"));
        assert!(has_symbol(&fp, "Player.Bag.add_item"));
        assert!(has_ref(&fp, "take_damage", "calls"));
        assert!(has_ref(&fp, "emit_signal", "calls"));
    }

    #[test]
    fn kotlin_extracts_packages_methods_and_calls() {
        let src = r#"
package demo.game

import demo.util.Logger

class Player(val name: String) : Character() {
    var health: Int = 100

    fun takeDamage(amount: Int) {
        health -= amount
        log("hit")
    }

    fun log(msg: String) {
        Logger.log(msg)
    }
}

fun main() {
    val p = Player("hero")
    p.takeDamage(3)
    Logger.log("start")
}
"#;
        let fp = parse("src/Player.kt", "app", "kotlin", src.as_bytes());
        assert_eq!(fp.lang, "kotlin");
        assert!(has_symbol(&fp, "demo.game.Player"));
        assert!(has_symbol(&fp, "demo.game.Player.takeDamage"));
        assert!(has_symbol(&fp, "demo.game.main"));
        assert!(has_ref(&fp, "takeDamage", "calls"));
        assert!(has_ref(&fp, "log", "calls"));
        let call = fp
            .refs
            .iter()
            .find(|r| r.name == "takeDamage")
            .expect("call ref");
        assert_eq!(call.receiver_type, "Player");
        assert_eq!(call.arity, 1);
    }

    #[test]
    fn swift_extracts_structs_enums_and_calls() {
        let src = r#"
class Player: Character {
    var health: Int = 100

    init(name: String) {
        health = 100
    }

    func takeDamage(amount: Int) {
        health -= amount
        log(msg: "hit")
    }

    func log(msg: String) {}
}

struct Point {
    var x: Int
}

enum State {
    case idle, run
}

func top() {
    let p = Player(name: "hero")
    p.takeDamage(amount: 3)
}
"#;
        let fp = parse("Sources/Player.swift", "app", "swift", src.as_bytes());
        assert_eq!(fp.lang, "swift");
        assert!(has_symbol(&fp, "Player"));
        assert!(has_symbol(&fp, "Player.takeDamage"));
        assert!(has_symbol(&fp, "Player.init"));
        assert!(has_symbol(&fp, "Point"));
        assert!(has_symbol(&fp, "State.idle"));
        assert!(has_symbol(&fp, "State.run"));
        assert!(has_ref(&fp, "takeDamage", "calls"));
        let call = fp
            .refs
            .iter()
            .find(|r| r.name == "takeDamage")
            .expect("call ref");
        assert_eq!(call.receiver_type, "Player");
        assert!(has_ref(&fp, "Player", "constructs"));
    }

    #[test]
    fn objc_extracts_selectors_and_messages() {
        let src = r#"
@interface Player : NSObject
@property NSInteger health;
- (void)takeDamage:(NSInteger)amount;
@end

@implementation Player
- (void)takeDamage:(NSInteger)amount {
    health -= amount;
    [self logHit:amount];
}

- (void)logHit:(NSInteger)amount {
}
@end
"#;
        let fp = parse("Player.m", "app", "objc", src.as_bytes());
        assert_eq!(fp.lang, "objc");
        assert!(has_symbol(&fp, "Player"));
        assert!(has_symbol(&fp, "Player.takeDamage:"));
        assert!(has_symbol(&fp, "Player.logHit:"));
        assert!(has_symbol(&fp, "Player.health"));
        assert!(has_ref(&fp, "logHit:", "calls"));
        let call = fp
            .refs
            .iter()
            .find(|r| r.name == "logHit:")
            .expect("message ref");
        assert_eq!(call.receiver_type, "Player");
        assert_eq!(call.arity, 1);
    }

    #[test]
    fn dart_extracts_classes_methods_and_calls() {
        let src = r#"
import 'dart:math' as math;

class Player extends Character {
  int health = 100;

  Player(String name);

  void takeDamage(int amount) {
    health -= amount;
    log('hit');
  }

  void log(String msg) {}
}

enum State { idle, run }

void main() {
  final p = Player('hero');
  p.takeDamage(3);
}
"#;
        let fp = parse("lib/player.dart", "app", "dart", src.as_bytes());
        assert_eq!(fp.lang, "dart");
        assert!(has_symbol(&fp, "Player"));
        assert!(has_symbol(&fp, "Player.takeDamage"));
        assert!(has_symbol(&fp, "Player.health"));
        assert!(has_symbol(&fp, "State.idle"));
        assert!(has_symbol(&fp, "main"));
        assert!(has_ref(&fp, "takeDamage", "calls"));
        let call = fp
            .refs
            .iter()
            .find(|r| r.name == "takeDamage")
            .expect("call ref");
        assert_eq!(call.receiver_type, "Player");
        assert!(has_ref(&fp, "Player", "constructs"));
    }

    #[test]
    fn kotlin_recovery_nodes_are_reported_without_panicking() {
        let src = "package demo\ninterface Updatable { fun update(dt: Float) }\n";
        let fp = parse("More.kt", "app", "kotlin", src.as_bytes());
        assert!(!fp.errors.is_empty());
        assert!(fp.errors.iter().any(|e| e.contains("recovery")));
    }
}
