#[derive(Debug, Clone, Default)]
pub struct Symbol {
    pub key: String,
    pub root: String,
    pub lang: String,
    pub kind: String,
    pub name: String,
    pub qualname: String,
    pub namespace: String,
    pub start_line: i32,
    pub end_line: i32,
    pub signature: String,
    pub modifiers: String,
    pub param_count: i32,
    pub container: String,
    pub module: String,
}

#[derive(Debug, Clone, Default)]
pub struct Ref {
    pub src_key: String,
    pub root: String,
    pub name: String,
    pub kind: String,
    pub receiver_type: String,
    pub namespace_hint: String,
    pub line: i32,
    pub arity: i32,
}

#[derive(Debug, Clone, Default)]
pub struct FileParse {
    pub rel_path: String,
    pub root: String,
    pub lang: String,
    pub module: String,
    pub symbols: Vec<Symbol>,
    pub refs: Vec<Ref>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolRow {
    pub id: i64,
    pub root: String,
    pub lang: String,
    pub kind: String,
    pub name: String,
    pub qualname: String,
    pub namespace: String,
    pub container: String,
    pub param_count: i32,
}

#[derive(Debug, Clone, Default)]
pub struct RefRow {
    pub id: i64,
    pub src_id: i64,
    pub root: String,
    pub name: String,
    pub receiver_type: String,
    pub kind: String,
    pub namespace_hint: String,
    pub line: i32,
    pub arity: i32,
    pub target: i64,
}

pub fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "struct" | "interface" | "enum" | "delegate" | "record" | "type"
    )
}

pub fn symbol_key(qual: &str, kind: &str, line: i32) -> String {
    format!("{qual}#{kind}@{line}")
}
