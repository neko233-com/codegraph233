//! Additional source frontends backed by bounded tree-sitter parses. These are
//! static declaration/reference graphs, not language runtimes or type checkers.
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Language, Node, Parser};

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}
fn child<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut c = node.walk();
    let found = node.named_children(&mut c).find(|n| n.kind() == kind);
    found
}
fn descendant<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut stack = vec![node];
    let mut scanned = 0;
    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            return Some(node);
        }
        scanned += 1;
        if scanned > 100000 {
            return None;
        }
        let mut c = node.walk();
        let children: Vec<_> = node.named_children(&mut c).collect();
        stack.extend(children.into_iter().rev());
    }
    None
}
fn declaration_name<'a>(node: Node<'a>) -> Option<Node<'a>> {
    if let Some(name) = node.child_by_field_name("name") {
        return Some(name);
    }
    match node.kind() {
        "identifier"
        | "field_identifier"
        | "qualified_identifier"
        | "destructor_name"
        | "operator_name" => Some(node),
        _ => node
            .child_by_field_name("declarator")
            .and_then(declaration_name),
    }
}
fn module(rel: &str) -> String {
    let stem = std::path::Path::new(rel)
        .with_extension("")
        .to_string_lossy()
        .replace(['\\', '/'], ".");
    stem.strip_suffix(".__init__").unwrap_or(&stem).to_string()
}
fn qualified(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        name.into()
    } else {
        format!("{scope}.{name}")
    }
}

struct Context {
    namespace: String,
    owner: String,
    source: String,
    lexical: String,
    variables: HashMap<String, String>,
}
struct Extractor<'a> {
    source: &'a [u8],
    language: &'a str,
    result: FileParse,
    aliases: HashMap<String, String>,
    seen: HashSet<String>,
    nodes: usize,
}
impl Extractor<'_> {
    fn symbol(
        &mut self,
        node: Node<'_>,
        kind: &str,
        name: &str,
        qual: &str,
        scope: &Context,
        params: i32,
    ) -> String {
        let line = node.start_position().row as i32 + 1;
        let key = symbol_key(qual, kind, line);
        if self.seen.insert(key.clone()) {
            let start = node.start_byte();
            let end = node
                .child_by_field_name("body")
                .map(|n| n.start_byte())
                .unwrap_or(node.end_byte())
                .min(start + 1024);
            let sig = String::from_utf8_lossy(&self.source[start..end])
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            self.result.symbols.push(Symbol {
                key: key.clone(),
                lang: self.language.into(),
                kind: kind.into(),
                name: name.into(),
                qualname: qual.into(),
                namespace: scope.namespace.clone(),
                container: scope.owner.clone(),
                start_line: line,
                end_line: node.end_position().row as i32 + 1,
                signature: sig,
                param_count: params,
                module: self.result.module.clone(),
                ..Default::default()
            });
        }
        key
    }
    fn reference(
        &mut self,
        node: Node<'_>,
        scope: &Context,
        name: &str,
        receiver: &str,
        kind: &str,
        arity: i32,
    ) {
        if name.is_empty() {
            return;
        }
        let receiver = self
            .aliases
            .get(receiver)
            .cloned()
            .unwrap_or_else(|| receiver.into());
        let (name, receiver) = if receiver.is_empty() {
            if let Some(target) = self.aliases.get(name) {
                if let Some((parent, name)) = target.rsplit_once('.') {
                    (name.to_string(), parent.to_string())
                } else {
                    (name.into(), receiver)
                }
            } else {
                (name.into(), receiver)
            }
        } else {
            (name.into(), receiver)
        };
        self.result.refs.push(Ref {
            src_key: scope.source.clone(),
            name,
            kind: kind.into(),
            receiver_type: receiver,
            namespace_hint: scope.namespace.clone(),
            line: node.start_position().row as i32 + 1,
            arity,
            ..Default::default()
        });
    }
    fn walk(
        &mut self,
        node: Node<'_>,
        scope: &mut Context,
        definition_declarator: bool,
        depth: usize,
    ) {
        if depth > 256 {
            if self.result.errors.len() < 32 {
                self.result
                    .errors
                    .push("AST nesting budget exceeded; partial graph".into());
            }
            return;
        }
        self.nodes += 1;
        if self.nodes > 2_000_000 {
            return;
        }
        let kind = node.kind();
        let mut nested = None;
        if kind == "namespace_definition" {
            if let Some(name) = node.child_by_field_name("name") {
                nested = Some(Context {
                    namespace: qualified(
                        &scope.namespace,
                        &text(name, self.source).replace("::", "."),
                    ),
                    lexical: qualified(
                        &scope.namespace,
                        &text(name, self.source).replace("::", "."),
                    ),
                    owner: String::new(),
                    source: scope.source.clone(),
                    variables: scope.variables.clone(),
                });
            }
        }
        let type_kind = match kind {
            "class_declaration" | "class_definition" | "class_specifier" | "class_statement" => {
                Some("class")
            }
            "interface_declaration" | "annotation_type_declaration" => Some("interface"),
            "struct_specifier" | "union_specifier" => Some("struct"),
            "enum_declaration" | "enum_specifier" | "enum_statement" => Some("enum"),
            "record_declaration" => Some("record"),
            _ => None,
        };
        if let Some(type_kind) = type_kind {
            let name = if matches!(kind, "class_statement" | "enum_statement") {
                child(node, "simple_name")
            } else {
                node.child_by_field_name("name")
            };
            if let Some(name) = name {
                let name = text(name, self.source);
                let base = &scope.lexical;
                let qual = qualified(base, name);
                let key = self.symbol(node, type_kind, name, &qual, scope, -1);
                nested = Some(Context {
                    namespace: scope.namespace.clone(),
                    owner: qual.clone(),
                    lexical: qual.clone(),
                    source: key.clone(),
                    variables: scope.variables.clone(),
                });
                if let Some(base) = node.child_by_field_name("superclass") {
                    if let Some(base) = descendant(base, "type_identifier") {
                        self.reference(
                            base,
                            nested.as_ref().unwrap(),
                            text(base, self.source),
                            "",
                            "inherits",
                            -1,
                        );
                    }
                }
            }
        }
        let callable = match kind {
            "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration" => true,
            "function_definition" if self.language == "python" => true,
            "function_definition" | "function_declarator"
                if matches!(self.language, "c" | "cpp") =>
            {
                !definition_declarator
            }
            "function_statement" | "class_method_definition" => true,
            _ => false,
        };
        if callable {
            let name_node = match kind {
                "function_statement" => child(node, "function_name"),
                "class_method_definition" => child(node, "simple_name"),
                _ => declaration_name(node),
            };
            if let Some(name_node) = name_node {
                let name = text(name_node, self.source).replace("::", ".");
                let base = &scope.lexical;
                let qual = if name.contains('.') {
                    name.clone()
                } else {
                    qualified(base, &name)
                };
                let short = name.rsplit('.').next().unwrap_or(&name);
                let parameters = node
                    .child_by_field_name("parameters")
                    .or_else(|| {
                        node.child_by_field_name("declarator")
                            .and_then(|n| descendant(n, "parameter_list"))
                    })
                    .or_else(|| descendant(node, "class_method_parameter_list"))
                    .or_else(|| descendant(node, "function_parameter_declaration"))
                    .or_else(|| descendant(node, "param_block"));
                let mut arity = parameters
                    .map(|n| {
                        let target = descendant(n, "parameter_list").unwrap_or(n);
                        let mut c = target.walk();
                        target
                            .named_children(&mut c)
                            .filter(|n| !matches!(n.kind(), "comment" | "type_parameter"))
                            .count() as i32
                    })
                    .unwrap_or(0);
                if matches!(self.language, "c" | "cpp")
                    && parameters.is_some_and(|p| text(p, self.source).trim() == "(void)")
                {
                    arity = 0;
                }
                if self.language == "python" && !scope.owner.is_empty() {
                    if let Some(parameters) = parameters {
                        if let Some(first) = parameters.named_child(0) {
                            let name = text(first, self.source)
                                .split([':', '='])
                                .next()
                                .unwrap_or("")
                                .trim();
                            let static_method = node.parent().is_some_and(|p| {
                                p.kind() == "decorated_definition"
                                    && text(p, self.source).contains("@staticmethod")
                            });
                            if matches!(name, "self" | "cls") && !static_method {
                                arity = (arity - 1).max(0);
                            }
                        }
                    }
                }
                let symbol_kind = if kind.contains("constructor") {
                    "constructor"
                } else if scope.owner.is_empty() {
                    "function"
                } else {
                    "method"
                };
                let key = self.symbol(node, symbol_kind, short, &qual, scope, arity);
                let mut variables = scope.variables.clone();
                if !scope.owner.is_empty() {
                    variables.insert("self".into(), scope.owner.clone());
                    variables.insert("this".into(), scope.owner.clone());
                    variables.insert("$this".into(), scope.owner.clone());
                }
                nested = Some(Context {
                    namespace: scope.namespace.clone(),
                    owner: scope.owner.clone(),
                    lexical: qual,
                    source: key,
                    variables,
                });
            }
        }
        let current = nested.as_mut().unwrap_or(scope);
        if matches!(kind, "enumerator" | "enum_constant" | "enum_member") {
            if let Some(name) = node
                .child_by_field_name("name")
                .or_else(|| child(node, "simple_name"))
            {
                let name = text(name, self.source);
                let qual = qualified(&current.lexical, name);
                self.symbol(node, "enum_member", name, &qual, current, -1);
            }
        }
        let is_field = matches!(kind, "field_declaration" | "class_property_definition")
            || matches!(kind, "declaration" | "assignment")
                && !current.source.contains("#function@")
                && !current.source.contains("#method@");
        if is_field && descendant(node, "function_declarator").is_none() {
            let mut names = Vec::new();
            if kind == "class_property_definition" {
                if let Some(name) = child(node, "variable") {
                    names.push(name);
                }
            } else if kind == "assignment" {
                if let Some(name) = node
                    .child_by_field_name("left")
                    .filter(|n| n.kind() == "identifier")
                {
                    names.push(name);
                }
            } else {
                let mut c = node.walk();
                for declaration in node.named_children(&mut c) {
                    if matches!(
                        declaration.kind(),
                        "variable_declarator"
                            | "init_declarator"
                            | "field_identifier"
                            | "pointer_declarator"
                            | "identifier"
                    ) {
                        if let Some(name) = declaration_name(declaration) {
                            names.push(name);
                        }
                    }
                }
            }
            for name_node in names {
                let name = text(name_node, self.source).trim_start_matches('$');
                let qual = qualified(&current.lexical, name);
                self.symbol(name_node, "field", name, &qual, current, -1);
            }
        }
        if matches!(
            kind,
            "formal_parameter"
                | "parameter_declaration"
                | "local_variable_declaration"
                | "declaration"
        ) {
            if let Some(t) = node.child_by_field_name("type") {
                let name = node.child_by_field_name("name").or_else(|| {
                    node.child_by_field_name("declarator")
                        .and_then(declaration_name)
                });
                if let Some(name) = name {
                    current.variables.insert(
                        text(name, self.source).into(),
                        text(t, self.source).replace("::", "."),
                    );
                }
            }
        }
        match kind {
            "method_invocation" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let receiver = node
                        .child_by_field_name("object")
                        .map(|n| text(n, self.source))
                        .unwrap_or("");
                    let receiver = current
                        .variables
                        .get(receiver)
                        .map(String::as_str)
                        .unwrap_or(receiver)
                        .to_string();
                    self.reference(
                        node,
                        current,
                        text(name, self.source),
                        &receiver,
                        "calls",
                        argument_count(node),
                    );
                }
            }
            "object_creation_expression" => {
                if let Some(t) = node.child_by_field_name("type") {
                    self.reference(
                        node,
                        current,
                        text(t, self.source),
                        "",
                        "constructs",
                        argument_count(node),
                    );
                }
            }
            "call" | "call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    let (name, receiver) = if let Some(name) = function
                        .child_by_field_name("attribute")
                        .or_else(|| function.child_by_field_name("field"))
                    {
                        let receiver = function
                            .child_by_field_name("object")
                            .or_else(|| function.child_by_field_name("argument"))
                            .map(|n| text(n, self.source))
                            .unwrap_or("");
                        (
                            text(name, self.source).to_string(),
                            current
                                .variables
                                .get(receiver)
                                .cloned()
                                .unwrap_or_else(|| receiver.replace("::", ".")),
                        )
                    } else {
                        let full = text(function, self.source).replace("::", ".");
                        full.rsplit_once('.')
                            .map(|(parent, name)| (name.into(), parent.into()))
                            .unwrap_or((full, String::new()))
                    };
                    let dynamic = !matches!(
                        function.kind(),
                        "identifier"
                            | "qualified_identifier"
                            | "attribute"
                            | "field_expression"
                            | "template_function"
                    );
                    self.reference(
                        node,
                        current,
                        &name,
                        &receiver,
                        if dynamic { "dynamic-call" } else { "calls" },
                        argument_count(node),
                    );
                }
            }
            "command" if self.language == "powershell" => {
                if let Some(name) = node.child_by_field_name("command_name") {
                    let name = text(name, self.source).trim_matches(['\'', '"']);
                    let dynamic = name.starts_with('$') || name.contains("$(");
                    self.reference(
                        node,
                        current,
                        name,
                        "",
                        if dynamic { "dynamic-call" } else { "calls" },
                        -1,
                    );
                }
            }
            "invokation_expression" if self.language == "powershell" => {
                if let Some(name) = child(node, "member_name") {
                    let name = text(name, self.source);
                    self.reference(
                        node,
                        current,
                        name,
                        "",
                        if name.starts_with('$') {
                            "dynamic-call"
                        } else {
                            "calls"
                        },
                        -1,
                    );
                }
            }
            _ => {}
        }
        let inside_declarator = matches!(self.language, "c" | "cpp")
            && (kind == "function_definition"
                || definition_declarator && kind != "compound_statement");
        let mut c = node.walk();
        for next in node.named_children(&mut c) {
            if next.is_error() || next.is_missing() {
                if self.result.errors.len() < 32 {
                    self.result.errors.push(format!(
                        "syntax recovery at {}:{} ({})",
                        next.start_position().row + 1,
                        next.start_position().column + 1,
                        next.kind()
                    ));
                }
            }
            self.walk(
                next,
                current,
                inside_declarator && next.kind() != "compound_statement",
                depth + 1,
            );
        }
    }
}

fn argument_count(node: Node<'_>) -> i32 {
    node.child_by_field_name("arguments")
        .map(|args| {
            let mut c = args.walk();
            args.named_children(&mut c)
                .filter(|n| n.kind() != "comment")
                .count() as i32
        })
        .unwrap_or(-1)
}

pub fn parse(rel: &str, root: &str, lang: &str, source: &[u8]) -> FileParse {
    let mut result = FileParse {
        rel_path: rel.into(),
        root: root.into(),
        lang: lang.into(),
        module: module(rel),
        ..Default::default()
    };
    if source.len() > 16 * 1024 * 1024 {
        result.errors.push("source exceeds 16 MiB".into());
        return result;
    }
    let language: Language = match lang {
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "powershell" => tree_sitter_powershell::LANGUAGE.into(),
        _ => {
            result.errors.push("unsupported AST language".into());
            return result;
        }
    };
    let mut parser = Parser::new();
    if let Err(error) = parser.set_language(&language) {
        result.errors.push(error.to_string());
        return result;
    }
    #[allow(deprecated)]
    parser.set_timeout_micros(2_000_000);
    // Known JNI/Windows annotation macros are not declarations or callable
    // names. Mask whole identifier tokens only; preserve all byte positions,
    // comments, string contents and the original source used for extraction.
    let mut normalized = source.to_vec();
    let mut annotations = HashSet::new();
    if matches!(lang, "c" | "cpp") {
        for token in crate::lexer::tokenize(
            source,
            crate::lexer::Options {
                hash_directives: true,
                ..Default::default()
            },
        ) {
            let name = &source[token.start..token.end];
            if matches!(
                name,
                b"JNICALL"
                    | b"JNIEXPORT"
                    | b"JNIIMPORT"
                    | b"WINAPI"
                    | b"APIENTRY"
                    | b"CALLBACK"
                    | b"NTAPI"
            ) {
                annotations.insert(String::from_utf8_lossy(name).into_owned());
                normalized[token.start..token.end].fill(b' ');
            }
        }
    }
    let Some(tree) = parser.parse(&normalized, None) else {
        result.errors.push("AST parse deadline exceeded".into());
        return result;
    };
    let node = tree.root_node();
    let mut namespace = if matches!(lang, "java" | "c" | "cpp") {
        String::new()
    } else {
        result.module.clone()
    };
    if lang == "java" {
        if let Some(package) = child(node, "package_declaration") {
            namespace = text(package, source)
                .trim()
                .trim_start_matches("package")
                .trim()
                .trim_end_matches(';')
                .trim()
                .into();
        }
    }
    let mut extractor = Extractor {
        source,
        language: lang,
        result,
        aliases: HashMap::new(),
        seen: HashSet::new(),
        nodes: 0,
    };
    if lang == "python" {
        let mut c = node.walk();
        for import in node
            .named_children(&mut c)
            .filter(|n| matches!(n.kind(), "import_statement" | "import_from_statement"))
        {
            let from = import.kind() == "import_from_statement";
            let module_node = import.child_by_field_name("module_name");
            let raw_module = module_node.map(|n| text(n, source)).unwrap_or("");
            let dots = raw_module.bytes().take_while(|b| *b == b'.').count();
            let resolved = if dots > 0 {
                let mut package: Vec<&str> = extractor.result.module.split('.').collect();
                package.pop();
                if dots > package.len() + 1 {
                    continue;
                }
                for _ in 1..dots {
                    package.pop();
                }
                let tail = raw_module.trim_start_matches('.');
                if !tail.is_empty() {
                    package.push(tail);
                }
                package.join(".")
            } else {
                raw_module.to_string()
            };
            let mut children = import.walk();
            for name_node in import.named_children(&mut children) {
                if Some(name_node) == module_node
                    || !matches!(name_node.kind(), "aliased_import" | "dotted_name")
                {
                    continue;
                }
                let actual = name_node
                    .child_by_field_name("name")
                    .map(|n| text(n, source))
                    .unwrap_or_else(|| text(name_node, source));
                let alias = name_node
                    .child_by_field_name("alias")
                    .map(|n| text(n, source));
                let local = alias.unwrap_or_else(|| {
                    if from {
                        actual
                    } else {
                        actual.split('.').next().unwrap_or(actual)
                    }
                });
                let target = if from {
                    qualified(&resolved, actual)
                } else if alias.is_some() {
                    actual.to_string()
                } else {
                    local.to_string()
                };
                extractor.aliases.insert(local.into(), target);
            }
        }
    }
    // Explicit Java imports can establish an owning type; wildcard/dynamic
    // imports remain unresolved rather than generating an assumed type.
    if lang == "java" {
        let mut c = node.walk();
        for import in node
            .named_children(&mut c)
            .filter(|n| n.kind() == "import_declaration")
        {
            let target = text(import, source)
                .trim()
                .trim_start_matches("import")
                .trim()
                .trim_start_matches("static")
                .trim()
                .trim_end_matches(';')
                .trim();
            if !target.ends_with(".*") {
                if let Some(name) = target.rsplit('.').next() {
                    extractor.aliases.insert(name.into(), target.into());
                }
            }
        }
    }
    let mut scope = Context {
        lexical: namespace.clone(),
        namespace,
        owner: String::new(),
        source: String::new(),
        variables: HashMap::new(),
    };
    let module_key = extractor.symbol(
        node,
        "module",
        "<module>",
        &format!("{}::<module>", extractor.result.module.clone()),
        &scope,
        -1,
    );
    scope.source = module_key;
    extractor.walk(node, &mut scope, false, 0);
    if extractor.nodes > 2_000_000 {
        extractor
            .result
            .errors
            .push("AST node budget exceeded; partial graph".into());
    }
    if node.has_error() && extractor.result.errors.is_empty() {
        extractor
            .result
            .errors
            .push("AST contains recovery nodes; partial graph".into());
    }
    if lang == "powershell" && !extractor.result.errors.is_empty() {
        match crate::powershell_ast::parse(rel, root, &extractor.result.module, source) {
            Ok(parsed) => return parsed,
            Err(error) => extractor.result.errors.push(error),
        }
    }
    if !annotations.is_empty() {
        let mut annotations: Vec<_> = annotations.into_iter().collect();
        annotations.sort();
        let note = format!(
            "AST annotation normalization: {}; no preprocessor expansion",
            annotations.join(",")
        );
        for symbol in &mut extractor.result.symbols {
            symbol.modifiers = note.clone();
        }
    }
    extractor.result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_frontends_capture_real_declarations_and_calls() {
        let cases = [
            ("java","A.java","package demo; class A { int run() { return work(1); } int work(int v) { return v; } }","demo.A.run","work"),
            ("cpp","a.cpp","namespace demo { struct A { int work(int v) { return v; } int run() { return work(1); } }; }","demo.A.run","work"),
            ("c","a.c","int work(int v) { return v; } int run(void) { return work(1); }","run","work"),
            ("python","a.py","class A:\n    def work(self, v):\n        return v\n    def run(self):\n        return self.work(1)\n","a.A.run","work"),
            ("powershell","a.ps1","function Get-Value { param([int] $Value); return $Value }; function Start-Work { Get-Value 1 }","a.Start-Work","Get-Value"),
        ];
        for (lang, path, source, qual, call) in cases {
            let parsed = parse(path, "code", lang, source.as_bytes());
            assert!(parsed.errors.is_empty(), "{lang}: {:?}", parsed.errors);
            assert!(
                parsed.symbols.iter().any(|s| s.qualname == qual),
                "{lang}: {:?}",
                parsed.symbols
            );
            assert!(
                parsed
                    .refs
                    .iter()
                    .any(|r| r.name == call && r.kind == "calls"),
                "{lang}: {:?}",
                parsed.refs
            );
        }
    }
    #[test]
    fn syntax_errors_are_visible_and_dynamic_python_calls_are_not_literal_targets() {
        let broken = parse("bad.py", "code", "python", b"def broken(:\n");
        assert!(!broken.errors.is_empty());
        let dynamic = parse(
            "a.py",
            "code",
            "python",
            b"def run(factory):\n    return factory()(1)\n",
        );
        assert!(dynamic.refs.iter().any(|r| r.kind == "dynamic-call"));
    }
    #[test]
    fn jni_annotations_keep_original_positions_and_calls() {
        let source = b"#define JNICALL __stdcall\n// JNICALL stays in this comment\nstatic void JNICALL onClass(void* env) { work(env); }\nextern \"C\" JNIEXPORT int JNICALL Agent_OnLoad(void* vm) { onClass(vm); return 0; }\nconst char* note = \"JNICALL\";\n";
        let parsed = parse("observer.cpp", "tool", "cpp", source);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let observer = parsed.symbols.iter().find(|s| s.name == "onClass").unwrap();
        assert_eq!(observer.start_line, 3);
        assert!(observer.signature.contains("JNICALL"));
        assert!(observer.modifiers.contains("no preprocessor expansion"));
        assert!(parsed.symbols.iter().any(|s| s.name == "Agent_OnLoad"));
        assert!(parsed
            .refs
            .iter()
            .any(|r| r.name == "onClass" && r.line == 4));
    }
    #[cfg(windows)]
    #[test]
    fn powershell_native_recovery_handles_valid_source_without_executing_it() {
        let sentinel = std::env::temp_dir().join(format!(
            "cg233-never-execute-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = format!("function Get-Value {{ param([int] $Value) return $Value }}; function Start-Work {{ Get-Value 1 }}; [IO.File]::WriteAllText('{}','must not execute')",sentinel.display());
        let parsed = parse("a.ps1", "code", "powershell", source.as_bytes());
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert!(parsed.symbols.iter().any(|s| s.name == "Get-Value"));
        assert!(!sentinel.exists(), "analyzed PowerShell source executed");
    }
}
