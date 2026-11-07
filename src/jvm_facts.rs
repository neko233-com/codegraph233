//! Import versioned, provenance-bearing ASM facts. No class loading, decryption,
//! Java execution, inferred runtime targets, or fabricated external declarations.
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use serde_json::Value;
use std::collections::HashMap;

fn string<'a>(row: &'a Value, key: &str) -> Result<&'a str, String> {
    row[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 131072)
        .ok_or_else(|| format!("missing/invalid {key}"))
}

fn sha(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn field_type(bytes: &[u8], at: &mut usize, void: bool) -> Result<(), String> {
    let mut array = false;
    while bytes.get(*at) == Some(&b'[') {
        array = true;
        *at += 1;
    }
    match bytes.get(*at) {
        Some(b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => *at += 1,
        Some(b'V') if void && !array => *at += 1,
        Some(b'L') => {
            *at += 1;
            let start = *at;
            while bytes.get(*at).is_some_and(|b| *b != b';') {
                *at += 1;
            }
            if *at == start || bytes.get(*at) != Some(&b';') {
                return Err("invalid object descriptor".into());
            }
            *at += 1;
        }
        _ => return Err("invalid JVM descriptor".into()),
    }
    Ok(())
}

fn parameters(descriptor: &str) -> Result<i32, String> {
    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err("method descriptor must start with (".into());
    }
    let mut at = 1;
    let mut count = 0;
    while bytes.get(at) != Some(&b')') {
        field_type(bytes, &mut at, false)?;
        count += 1;
    }
    at += 1;
    field_type(bytes, &mut at, true)?;
    if at != bytes.len() {
        return Err("trailing method descriptor bytes".into());
    }
    Ok(count)
}

fn field_id(owner: &str, name: &str, descriptor: &str) -> Result<String, String> {
    let mut at = 0;
    field_type(descriptor.as_bytes(), &mut at, false)?;
    if at != descriptor.len() {
        return Err("trailing field descriptor bytes".into());
    }
    Ok(format!("{owner}#{name}:{descriptor}"))
}

fn symbol(
    owner: &str,
    name: &str,
    qual: String,
    kind: &str,
    line: i32,
    signature: String,
    arity: i32,
) -> Symbol {
    Symbol {
        key: symbol_key(&qual, kind, line),
        lang: "jvm".into(),
        kind: kind.into(),
        name: name.into(),
        namespace: owner
            .rsplit_once('/')
            .map(|(p, _)| p.replace('/', "."))
            .unwrap_or_default(),
        container: if kind == "class" {
            String::new()
        } else {
            owner.into()
        },
        qualname: qual,
        start_line: line,
        end_line: line,
        signature,
        modifiers: "bytecode-facts; lines refer to JSONL records, not recovered Java source".into(),
        param_count: arity,
        ..Default::default()
    }
}

fn checked(rel: &str, root: &str, data: &[u8]) -> Result<FileParse, String> {
    if data.len() > 64 * 1024 * 1024 {
        return Err("JVM fact input exceeds 64 MiB".into());
    }
    let text = std::str::from_utf8(data).map_err(|e| e.to_string())?;
    let mut rows: Vec<(i32, Value)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > 1024 * 1024 {
            return Err(format!("record {} exceeds 1 MiB", n + 1));
        }
        let row = serde_json::from_str(line).map_err(|e| format!("record {}: {e}", n + 1))?;
        rows.push(((n + 1) as i32, row));
        if rows.len() > 1_000_000 {
            return Err("too many JVM records".into());
        }
    }
    let manifest = &rows.first().ok_or("missing manifest")?.1;
    if manifest["kind"] != "manifest"
        || manifest["format"] != "codegraph233.jvm-facts"
        || manifest["version"] != 1
    {
        return Err("expected codegraph233.jvm-facts version 1 manifest".into());
    }
    let artifact = string(manifest, "artifactSha256")?;
    if !sha(artifact) {
        return Err("invalid artifact SHA-256".into());
    }
    let summary = &rows.last().ok_or("missing summary")?.1;
    if summary["kind"] != "summary" || summary["failedClasses"] != 0 {
        return Err("missing successful final summary; partial facts rejected".into());
    }
    let mut result = FileParse {
        rel_path: rel.into(),
        root: root.into(),
        lang: "jvm".into(),
        module: artifact.into(),
        ..Default::default()
    };
    let mut keys = HashMap::new();
    let mut class_entries = HashMap::new();
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for (line, row) in rows.iter().skip(1).take(rows.len().saturating_sub(2)) {
        let kind = string(row, "kind")?;
        *counts.entry(kind).or_default() += 1;
        if kind == "class" {
            let owner = string(row, "name")?;
            let entry = string(row, "entry")?;
            if !sha(string(row, "sha256")?) || row["major"].as_u64().unwrap_or(0) < 45 {
                return Err("invalid class identity/version".into());
            }
            if class_entries
                .insert(owner.to_string(), entry.to_string())
                .is_some()
            {
                return Err(format!(
                    "duplicate class {owner}; select a multi-release variant before export"
                ));
            }
            let name = owner.rsplit('/').next().unwrap_or(owner);
            let s = symbol(
                owner,
                name,
                owner.into(),
                "class",
                *line,
                format!(
                    "class {owner}; entry={entry}; sha256={}",
                    string(row, "sha256")?
                ),
                -1,
            );
            keys.insert(s.qualname.clone(), s.key.clone());
            result.symbols.push(s);
        } else if kind == "method" || kind == "field" {
            let owner = string(row, "owner")?;
            let name = string(row, "name")?;
            let desc = string(row, "descriptor")?;
            if class_entries.get(owner).map(String::as_str) != Some(string(row, "entry")?) {
                return Err(format!("declaration owner/entry not established: {owner}"));
            }
            let (qual, arity, symbol_kind) = if kind == "method" {
                let qual = format!("{owner}#{name}{desc}");
                if string(row, "id")? != qual {
                    return Err("method id differs from descriptor".into());
                }
                (
                    qual,
                    parameters(desc)?,
                    if name == "<init>" {
                        "constructor"
                    } else {
                        "method"
                    },
                )
            } else {
                (field_id(owner, name, desc)?, -1, "field")
            };
            let s = symbol(
                owner,
                name,
                qual,
                symbol_kind,
                *line,
                format!("{owner}#{name}{desc}; access={}", row["access"]),
                arity,
            );
            if keys.insert(s.qualname.clone(), s.key.clone()).is_some() {
                return Err("duplicate JVM declaration".into());
            }
            result.symbols.push(s);
        }
    }
    for (kind, expected) in [
        ("class", "classes"),
        ("method", "methods"),
        ("field", "fields"),
        ("invoke", "directCalls"),
        ("invokedynamic", "invokedynamicSites"),
    ] {
        if summary[expected].as_u64() != Some(*counts.get(kind).unwrap_or(&0)) {
            return Err(format!("summary {expected} differs from records"));
        }
    }
    if summary.get("constantDynamicSites").is_some()
        && summary["constantDynamicSites"].as_u64()
            != Some(*counts.get("constant-dynamic").unwrap_or(&0))
    {
        return Err("summary constantDynamicSites differs from records".into());
    }
    for (line, row) in rows.iter().skip(1).take(rows.len().saturating_sub(2)) {
        let kind = string(row, "kind")?;
        let (src, target, edge) = match kind {
            "class" => {
                let Some(target) = row["super"].as_str().filter(|s| !s.is_empty()) else {
                    continue;
                };
                (
                    string(row, "name")?.to_string(),
                    target.to_string(),
                    "inherits",
                )
            }
            "implements" => (
                string(row, "owner")?.to_string(),
                string(row, "target")?.to_string(),
                "inherits",
            ),
            "invoke" => {
                let owner = string(row, "owner")?;
                let name = string(row, "name")?;
                let desc = string(row, "descriptor")?;
                parameters(desc)?;
                let target = format!("{owner}#{name}{desc}");
                if string(row, "target")? != target {
                    return Err("invoke target differs from descriptor".into());
                }
                (string(row, "caller")?.to_string(), target, "calls")
            }
            "field-access" => (
                string(row, "caller")?.to_string(),
                field_id(
                    string(row, "owner")?,
                    string(row, "name")?,
                    string(row, "descriptor")?,
                )?,
                "field-access",
            ),
            "bootstrap" | "bootstrap-handle" | "constant-handle" => {
                let desc = string(row, "descriptor")?;
                let owner = string(row, "owner")?;
                let name = string(row, "name")?;
                let tag = row["tag"].as_u64().ok_or("missing handle tag")?;
                let target = if (1..=4).contains(&tag) {
                    field_id(owner, name, desc)?
                } else if (5..=9).contains(&tag) {
                    parameters(desc)?;
                    format!("{owner}#{name}{desc}")
                } else {
                    return Err("invalid method-handle tag".into());
                };
                (string(row, "caller")?.to_string(), target, "bootstrap")
            }
            "invokedynamic" | "constant-dynamic" => {
                if row["runtimeTargetResolved"] != false {
                    return Err("v1 never claims a resolved dynamic target".into());
                }
                if kind == "invokedynamic" {
                    parameters(string(row, "descriptor")?)?;
                } else {
                    field_id("constant", string(row, "name")?, string(row, "descriptor")?)?;
                }
                (
                    string(row, "caller")?.to_string(),
                    format!(
                        "invokedynamic:{}{}@{line}",
                        string(row, "name")?,
                        string(row, "descriptor")?
                    ),
                    "dynamic-call",
                )
            }
            "method" | "field" | "string" | "raw-string" | "long-string" | "type-constant" => {
                continue
            }
            _ => return Err(format!("unsupported JVM record {kind}")),
        };
        let src_key = keys
            .get(&src)
            .ok_or_else(|| format!("reference source absent: {src}"))?
            .clone();
        result.refs.push(Ref {
            src_key,
            name: target,
            kind: edge.into(),
            line: *line,
            arity: -1,
            ..Default::default()
        });
    }
    Ok(result)
}

pub fn parse(rel: &str, root: &str, data: &[u8]) -> FileParse {
    match checked(rel, root, data) {
        Ok(result) => result,
        Err(error) => FileParse {
            rel_path: rel.into(),
            root: root.into(),
            lang: "jvm".into(),
            errors: vec![error],
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn fixture() -> Vec<Value> {
        vec![
            json!({"kind":"manifest","format":"codegraph233.jvm-facts","version":1,"artifactSha256":"a".repeat(64)}),
            json!({"kind":"class","entry":"a/A.class","name":"a/A","sha256":"b".repeat(64),"major":52,"super":"java/lang/Object"}),
            json!({"kind":"method","entry":"a/A.class","owner":"a/A","name":"call","id":"a/A#call()V","descriptor":"()V","access":9}),
            json!({"kind":"method","entry":"a/A.class","owner":"a/A","name":"pick","id":"a/A#pick(I)I","descriptor":"(I)I","access":9}),
            json!({"kind":"method","entry":"a/A.class","owner":"a/A","name":"pick","id":"a/A#pick(Ljava/lang/String;)I","descriptor":"(Ljava/lang/String;)I","access":9}),
            json!({"kind":"invoke","caller":"a/A#call()V","owner":"a/A","name":"pick","descriptor":"(Ljava/lang/String;)I","target":"a/A#pick(Ljava/lang/String;)I"}),
            json!({"kind":"invokedynamic","caller":"a/A#call()V","name":"run","descriptor":"()Ljava/lang/Runnable;","runtimeTargetResolved":false}),
            json!({"kind":"summary","classes":1,"methods":3,"fields":0,"directCalls":1,"invokedynamicSites":1,"failedClasses":0}),
        ]
    }
    fn parse_rows(rows: &[Value]) -> FileParse {
        let source = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        parse("artifact.cg-jvm.jsonl", "jar", source.as_bytes())
    }
    #[test]
    fn jvm_descriptors_keep_overloads_and_dynamic_sites_distinct() {
        let parsed = parse_rows(&fixture());
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.symbols.len(), 4);
        let names: HashSet<_> = parsed.symbols.iter().map(|s| s.qualname.as_str()).collect();
        assert!(names.contains("a/A#pick(I)I"));
        assert!(names.contains("a/A#pick(Ljava/lang/String;)I"));
        assert_eq!(parsed.refs.iter().filter(|r| r.kind == "calls").count(), 1);
        assert_eq!(
            parsed
                .refs
                .iter()
                .filter(|r| r.kind == "dynamic-call")
                .count(),
            1
        );
        assert_eq!(
            parameters("([[I[Ljava/lang/String;J)Ljava/lang/Object;").unwrap(),
            3
        );
        for invalid in ["(V)V", "([V)V", "(L;)V", "(I)", "()Vx", "I"] {
            assert!(parameters(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn jvm_partial_tampered_duplicate_and_unknown_facts_are_rejected() {
        let baseline = fixture();
        let mut rows = baseline.clone();
        rows.pop();
        assert!(!parse_rows(&rows).errors.is_empty());
        let mut rows = baseline.clone();
        rows.last_mut().unwrap()["methods"] = json!(4);
        assert!(!parse_rows(&rows).errors.is_empty());
        let mut rows = baseline.clone();
        rows[4] = rows[3].clone();
        assert!(!parse_rows(&rows).errors.is_empty());
        let mut rows = baseline.clone();
        rows[5]["target"] = json!("a/A#pick(I)I");
        assert!(!parse_rows(&rows).errors.is_empty());
        let mut rows = baseline.clone();
        rows[0]["version"] = json!(2);
        assert!(!parse_rows(&rows).errors.is_empty());
        let mut rows = baseline;
        rows[6]["kind"] = json!("class-error");
        rows.last_mut().unwrap()["invokedynamicSites"] = json!(0);
        assert!(!parse_rows(&rows).errors.is_empty());
    }
    #[test]
    fn jvm_binding_never_guesses_a_same_arity_overload_or_dynamic_target() {
        use crate::binder::Binder;
        use crate::model::{RefRow, SymbolRow};
        let parsed = parse_rows(&fixture());
        let rows: Vec<SymbolRow> = parsed
            .symbols
            .iter()
            .enumerate()
            .map(|(i, s)| SymbolRow {
                id: (i + 1) as i64,
                root: "jar".into(),
                lang: s.lang.clone(),
                kind: s.kind.clone(),
                name: s.name.clone(),
                qualname: s.qualname.clone(),
                namespace: s.namespace.clone(),
                container: s.container.clone(),
                param_count: s.param_count,
            })
            .collect();
        let binder = Binder::new(rows.clone());
        let source = &rows[1];
        let reference = RefRow {
            name: "a/A#pick(Ljava/lang/String;)I".into(),
            kind: "calls".into(),
            arity: 1,
            ..Default::default()
        };
        let resolved = binder.resolve(&reference, source).unwrap();
        assert_eq!(binder.row(resolved.row).qualname, reference.name);
        assert_eq!(resolved.confidence, "exact");
        let mut missing = reference.clone();
        missing.name = "other/A#pick(Ljava/lang/String;)I".into();
        assert!(binder.resolve(&missing, source).is_none());
        let mut dynamic = reference.clone();
        dynamic.kind = "dynamic-call".into();
        assert!(binder.resolve(&dynamic, source).is_none());
        let mut duplicates = rows.clone();
        let mut duplicate = rows[3].clone();
        duplicate.id = 50;
        duplicates.push(duplicate);
        assert!(Binder::new(duplicates)
            .resolve(&reference, source)
            .is_none());
    }
}
