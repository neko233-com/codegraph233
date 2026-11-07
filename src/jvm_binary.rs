//! Bounded readers for ordinary, unencrypted CLASS/JAR inputs. Analyzed Java
//! is never loaded/executed. Unknown/private containers fail without decoding.
use crate::model::{Asset, FileParse};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};

const CLASS_LIMIT: usize = 16 * 1024 * 1024;
const TOTAL_LIMIT: usize = 256 * 1024 * 1024;

fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("JVM length overflow")?;
        let value = self
            .data
            .get(self.at..end)
            .ok_or_else(|| format!("truncated JVM data at byte {}", self.at))?;
        self.at = end;
        Ok(value)
    }
    fn u1(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u2(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u4(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn done(&self) -> Result<(), String> {
        if self.at == self.data.len() {
            Ok(())
        } else {
            Err("trailing JVM attribute bytes".into())
        }
    }
}

#[derive(Clone)]
enum Cp {
    Empty,
    Utf8(Vec<u8>),
    Class(u16),
    String(u16),
    Ref(u8, u16, u16),
    Nat(u16, u16),
    Handle(u8, u16),
    MethodType(u16),
    Dynamic(bool, u16, u16),
    Other,
}
struct Pool(Vec<Cp>);
impl Pool {
    fn get(&self, id: u16) -> Result<&Cp, String> {
        self.0
            .get(id as usize)
            .ok_or_else(|| format!("invalid constant-pool index {id}"))
    }
    fn utf8(&self, id: u16) -> Result<String, String> {
        if let Cp::Utf8(data) = self.get(id)? {
            cesu8::from_java_cesu8(data)
                .map(|s| s.into_owned())
                .map_err(|_| "invalid modified-UTF8 identifier".into())
        } else {
            Err("expected UTF8 constant".into())
        }
    }
    fn class(&self, id: u16) -> Result<String, String> {
        if let Cp::Class(name) = self.get(id)? {
            self.utf8(*name)
        } else {
            Err("expected class constant".into())
        }
    }
    fn nat(&self, id: u16) -> Result<(String, String), String> {
        if let Cp::Nat(name, desc) = self.get(id)? {
            Ok((self.utf8(*name)?, self.utf8(*desc)?))
        } else {
            Err("expected name/type constant".into())
        }
    }
    fn member(&self, id: u16) -> Result<(u8, String, String, String), String> {
        if let Cp::Ref(tag, owner, nat) = self.get(id)? {
            let (name, desc) = self.nat(*nat)?;
            Ok((*tag, self.class(*owner)?, name, desc))
        } else {
            Err("expected member reference constant".into())
        }
    }
    fn handle(&self, id: u16) -> Result<(u8, String, String, String, bool), String> {
        if let Cp::Handle(kind, target) = self.get(id)? {
            let (tag, owner, name, desc) = self.member(*target)?;
            if !(1..=9).contains(kind) || ((*kind <= 4) != (tag == 9)) {
                return Err("invalid method-handle reference".into());
            }
            Ok((*kind, owner, name, desc, tag == 11))
        } else {
            Err("expected method handle".into())
        }
    }
}

fn pool(r: &mut Reader<'_>) -> Result<Pool, String> {
    let count = r.u2()? as usize;
    if count == 0 {
        return Err("empty constant pool".into());
    }
    let mut values = vec![Cp::Empty];
    while values.len() < count {
        let tag = r.u1()?;
        let value = match tag {
            1 => {
                let n = r.u2()? as usize;
                Cp::Utf8(r.take(n)?.to_vec())
            }
            3 | 4 => {
                r.take(4)?;
                Cp::Other
            }
            5 | 6 => {
                r.take(8)?;
                values.push(Cp::Other);
                Cp::Empty
            }
            7 => Cp::Class(r.u2()?),
            8 => Cp::String(r.u2()?),
            9..=11 => Cp::Ref(tag, r.u2()?, r.u2()?),
            12 => Cp::Nat(r.u2()?, r.u2()?),
            15 => Cp::Handle(r.u1()?, r.u2()?),
            16 => Cp::MethodType(r.u2()?),
            17 | 18 => Cp::Dynamic(tag == 17, r.u2()?, r.u2()?),
            19 | 20 => {
                r.u2()?;
                Cp::Other
            }
            _ => return Err(format!("unsupported constant-pool tag {tag}")),
        };
        values.push(value);
    }
    if values.len() != count {
        return Err("invalid two-slot constant pool entry".into());
    }
    Ok(Pool(values))
}

fn attributes<'a>(r: &mut Reader<'a>, cp: &Pool) -> Result<Vec<(String, &'a [u8])>, String> {
    let mut out = Vec::new();
    for _ in 0..r.u2()? {
        let name = cp.utf8(r.u2()?)?;
        let len = r.u4()? as usize;
        out.push((name, r.take(len)?));
    }
    Ok(out)
}

struct Code<'a> {
    bytes: &'a [u8],
    lines: Vec<(u16, u16)>,
}
fn code<'a>(data: &'a [u8], cp: &Pool) -> Result<Code<'a>, String> {
    let mut r = Reader::new(data);
    r.u2()?;
    r.u2()?;
    let size = r.u4()? as usize;
    if size == 0 || size > 65535 {
        return Err("invalid method code length".into());
    }
    let bytes = r.take(size)?;
    let exceptions = r.u2()? as usize;
    r.take(exceptions * 8)?;
    let mut lines = Vec::new();
    for (name, attr) in attributes(&mut r, cp)? {
        if name == "LineNumberTable" {
            let mut a = Reader::new(attr);
            for _ in 0..a.u2()? {
                let pc = a.u2()?;
                let line = a.u2()?;
                if pc as usize >= size {
                    return Err("line table offset outside code".into());
                }
                lines.push((pc, line));
            }
            a.done()?;
        }
    }
    r.done()?;
    Ok(Code { bytes, lines })
}

fn add(rows: &mut Vec<Value>, entry: &str, mut row: Value) {
    row["entry"] = json!(entry);
    rows.push(row);
}
fn add_handle(
    rows: &mut Vec<Value>,
    entry: &str,
    caller: &str,
    site: usize,
    id: u16,
    kind: &str,
    cp: &Pool,
) -> Result<(), String> {
    let (tag, owner, name, desc, face) = cp.handle(id)?;
    add(
        rows,
        entry,
        json!({"kind":kind,"caller":caller,"site":site,"owner":owner,"name":name,"descriptor":desc,"tag":tag,"interface":face}),
    );
    Ok(())
}
fn dynamic(
    rows: &mut Vec<Value>,
    entry: &str,
    caller: &str,
    site: usize,
    line: u16,
    id: u16,
    cp: &Pool,
    bootstraps: &[(u16, Vec<u16>)],
    constant: bool,
) -> Result<(), String> {
    let Cp::Dynamic(is_constant, index, nat) = cp.get(id)? else {
        return Err("expected dynamic constant".into());
    };
    if *is_constant != constant {
        return Err("wrong dynamic constant kind".into());
    }
    let (name, desc) = cp.nat(*nat)?;
    add(
        rows,
        entry,
        json!({"kind":if constant {"constant-dynamic"} else {"invokedynamic"},"caller":caller,"site":site,"sourceLine":line,"name":name,"descriptor":desc,"runtimeTargetResolved":false}),
    );
    let (method, args) = bootstraps
        .get(*index as usize)
        .ok_or("bootstrap index outside table")?;
    add_handle(rows, entry, caller, site, *method, "bootstrap", cp)?;
    for arg in args {
        if matches!(cp.get(*arg)?, Cp::Handle(..)) {
            add_handle(rows, entry, caller, site, *arg, "bootstrap-handle", cp)?;
        }
    }
    Ok(())
}

fn scan_code(
    rows: &mut Vec<Value>,
    entry: &str,
    caller: &str,
    code: &Code<'_>,
    cp: &Pool,
    bootstrap: &[(u16, Vec<u16>)],
) -> Result<(), String> {
    let bytes = code.bytes;
    let mut pc = 0;
    let mut site = 0;
    while pc < bytes.len() {
        let opcode = bytes[pc];
        let len = match opcode {
            0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
            0x11
            | 0x13
            | 0x14
            | 0x84
            | 0x99..=0xa8
            | 0xb2..=0xb8
            | 0xbb
            | 0xbd
            | 0xc0
            | 0xc1
            | 0xc6
            | 0xc7 => 3,
            0xc5 => 4,
            0xb9 | 0xba | 0xc8 | 0xc9 => 5,
            0xc4 => match bytes.get(pc + 1) {
                Some(0x84) => 6,
                Some(0x15..=0x19 | 0x36..=0x3a | 0xa9) => 4,
                _ => return Err("invalid wide instruction".into()),
            },
            0xaa | 0xab => {
                let aligned = (pc + 4) & !3;
                let mut r = Reader {
                    data: bytes,
                    at: aligned,
                };
                r.u4()?;
                let entries = if opcode == 0xaa {
                    let low = r.u4()? as i32;
                    let high = r.u4()? as i32;
                    if high < low {
                        return Err("invalid tableswitch bounds".into());
                    }
                    (high as i64 - low as i64 + 1)
                        .checked_mul(4)
                        .ok_or("switch overflow")?
                } else {
                    let pairs = r.u4()? as i32;
                    if pairs < 0 {
                        return Err("negative lookupswitch count".into());
                    }
                    pairs as i64 * 8
                };
                if entries > bytes.len() as i64 {
                    return Err("switch table exceeds method".into());
                }
                r.take(entries as usize)?;
                r.at - pc
            }
            0x00..=0xc3 => 1,
            _ => return Err(format!("reserved/unsupported opcode {opcode:#x}")),
        };
        let instruction = bytes.get(pc..pc + len).ok_or("truncated instruction")?;
        let line = code
            .lines
            .iter()
            .filter(|(start, _)| *start as usize <= pc)
            .max_by_key(|(start, _)| *start)
            .map(|(_, line)| *line)
            .unwrap_or(0);
        let index = || u16::from_be_bytes([instruction[1], instruction[2]]);
        if (0xb2..=0xb5).contains(&opcode) {
            let (tag, owner, name, desc) = cp.member(index())?;
            if tag != 9 {
                return Err("field opcode needs a field constant".into());
            }
            site += 1;
            add(
                rows,
                entry,
                json!({"kind":"field-access","caller":caller,"site":site,"sourceLine":line,"opcode":opcode,"owner":owner,"name":name,"descriptor":desc}),
            );
        } else if (0xb6..=0xb9).contains(&opcode) {
            let (tag, owner, name, desc) = cp.member(index())?;
            if tag != 10 && tag != 11 {
                return Err("invoke needs a method constant".into());
            }
            if opcode == 0xb9 && (instruction[3] == 0 || instruction[4] != 0 || tag != 11) {
                return Err("invalid invokeinterface operands".into());
            }
            site += 1;
            add(
                rows,
                entry,
                json!({"kind":"invoke","caller":caller,"site":site,"sourceLine":line,"opcode":opcode,"owner":owner,"name":name,"descriptor":desc,"target":format!("{owner}#{name}{desc}"),"interface":tag==11}),
            );
        } else if opcode == 0xba {
            if instruction[3..] != [0, 0] {
                return Err("invalid invokedynamic operands".into());
            }
            site += 1;
            dynamic(
                rows,
                entry,
                caller,
                site,
                line,
                index(),
                cp,
                bootstrap,
                false,
            )?;
        } else if (0x12..=0x14).contains(&opcode) {
            let id = if opcode == 0x12 {
                instruction[1] as u16
            } else {
                index()
            };
            match cp.get(id)? {
                Cp::String(text) => {
                    site += 1;
                    match cp.utf8(*text) {
                        Ok(value) if value.encode_utf16().count() <= 4096 => add(
                            rows,
                            entry,
                            json!({"kind":"string","caller":caller,"site":site,"value":value}),
                        ),
                        Ok(value) => add(
                            rows,
                            entry,
                            json!({"kind":"long-string","caller":caller,"site":site,"length":value.encode_utf16().count(),"sha256":hash(value.as_bytes())}),
                        ),
                        Err(_) => {
                            let Cp::Utf8(raw) = cp.get(*text)? else {
                                return Err("string constant needs UTF8".into());
                            };
                            add(
                                rows,
                                entry,
                                json!({"kind":"raw-string","caller":caller,"site":site,"encoding":"modified-utf8","bytes":raw.len(),"sha256":hash(raw)}),
                            );
                        }
                    }
                }
                Cp::Handle(..) => {
                    site += 1;
                    add_handle(rows, entry, caller, site, id, "constant-handle", cp)?;
                }
                Cp::Class(_) => {
                    site += 1;
                    let name = cp.class(id)?;
                    let descriptor = if name.starts_with('[') {
                        name
                    } else {
                        format!("L{name};")
                    };
                    add(
                        rows,
                        entry,
                        json!({"kind":"type-constant","caller":caller,"site":site,"descriptor":descriptor}),
                    );
                }
                Cp::MethodType(desc) => {
                    site += 1;
                    add(
                        rows,
                        entry,
                        json!({"kind":"type-constant","caller":caller,"site":site,"descriptor":cp.utf8(*desc)?}),
                    );
                }
                Cp::Dynamic(true, ..) => {
                    site += 1;
                    dynamic(rows, entry, caller, site, line, id, cp, bootstrap, true)?;
                }
                Cp::Other => {}
                _ => return Err("invalid ldc constant kind".into()),
            }
        }
        pc += len;
    }
    Ok(())
}

fn read_class(data: &[u8], entry: &str, rows: &mut Vec<Value>) -> Result<(), String> {
    if data.len() > CLASS_LIMIT {
        return Err("class exceeds 16 MiB".into());
    }
    let mut r = Reader::new(data);
    if r.u4()? != 0xcafebabe {
        return Err("not an ordinary CLASS file".into());
    }
    r.u2()?;
    let major = r.u2()?;
    let cp = pool(&mut r)?;
    let access = r.u2()?;
    let owner = cp.class(r.u2()?)?;
    let super_id = r.u2()?;
    let parent = if super_id == 0 {
        None
    } else {
        Some(cp.class(super_id)?)
    };
    add(
        rows,
        entry,
        json!({"kind":"class","name":owner,"major":major,"bytes":data.len(),"sha256":hash(data),"super":parent,"access":access}),
    );
    for _ in 0..r.u2()? {
        add(
            rows,
            entry,
            json!({"kind":"implements","owner":owner,"target":cp.class(r.u2()?)?}),
        );
    }
    for _ in 0..r.u2()? {
        let access = r.u2()?;
        let name = cp.utf8(r.u2()?)?;
        let desc = cp.utf8(r.u2()?)?;
        add(
            rows,
            entry,
            json!({"kind":"field","owner":owner,"name":name,"descriptor":desc,"access":access}),
        );
        attributes(&mut r, &cp)?;
    }
    let mut methods = Vec::new();
    for _ in 0..r.u2()? {
        let access = r.u2()?;
        let name = cp.utf8(r.u2()?)?;
        let desc = cp.utf8(r.u2()?)?;
        let id = format!("{owner}#{name}{desc}");
        let attrs = attributes(&mut r, &cp)?;
        let mut body = None;
        for (kind, bytes) in attrs {
            if kind == "Code" {
                if body.is_some() {
                    return Err("duplicate Code attribute".into());
                }
                body = Some(code(bytes, &cp)?);
            }
        }
        methods.push((json!({"kind":"method","id":id,"owner":owner,"name":name,"descriptor":desc,"access":access}),id,body));
    }
    let mut bootstrap = Vec::new();
    let mut seen_bootstrap = false;
    for (name, data) in attributes(&mut r, &cp)? {
        if name == "BootstrapMethods" {
            if seen_bootstrap {
                return Err("duplicate BootstrapMethods".into());
            }
            seen_bootstrap = true;
            let mut a = Reader::new(data);
            for _ in 0..a.u2()? {
                let method = a.u2()?;
                let mut args = Vec::new();
                for _ in 0..a.u2()? {
                    args.push(a.u2()?);
                }
                bootstrap.push((method, args));
            }
            a.done()?;
        }
    }
    r.done()?;
    for (row, id, body) in methods {
        add(rows, entry, row);
        if let Some(body) = body {
            scan_code(rows, entry, &id, &body, &cp, &bootstrap)?;
        }
    }
    Ok(())
}

pub fn parse(rel: &str, root: &str, data: &[u8], release: u32) -> FileParse {
    match checked(rel, root, data, release) {
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

fn checked(rel: &str, root: &str, data: &[u8], release: u32) -> Result<FileParse, String> {
    if data.len() > 512 * 1024 * 1024 {
        return Err("archive exceeds 512 MiB".into());
    }
    let artifact = hash(data);
    let mut rows = vec![
        json!({"kind":"manifest","format":"codegraph233.jvm-facts","version":1,"artifactSha256":artifact,"release":release,"exporter":"codegraph233-native/1"}),
    ];
    let mut assets = Vec::new();
    if data.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        read_class(data, rel, &mut rows)?;
    } else {
        let mut archive = zip::ZipArchive::new(Cursor::new(data))
            .map_err(|e| format!("not an ordinary readable JAR: {e}"))?;
        if archive.len() > 30000 {
            return Err("archive member bound exceeded".into());
        }
        let multi = if let Ok(mut member) = archive.by_name("META-INF/MANIFEST.MF") {
            let mut text = String::new();
            member
                .by_ref()
                .take(1024 * 1024)
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            text.replace("\r\n ", "")
                .replace("\n ", "")
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("Multi-Release: true"))
        } else {
            false
        };
        let mut selected: BTreeMap<String, (u32, usize)> = BTreeMap::new();
        let mut total = 0usize;
        for i in 0..archive.len() {
            let mut member = archive.by_index(i).map_err(|e| e.to_string())?;
            if member.is_dir() {
                continue;
            }
            let entry = member.name().to_string();
            if entry.starts_with('/') || entry.contains('\\') || entry.split('/').any(|p| p == "..")
            {
                return Err("unsafe archive member path".into());
            }
            let mut bytes = Vec::new();
            member
                .by_ref()
                .take(TOTAL_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            total = total
                .checked_add(bytes.len())
                .ok_or("archive total overflow")?;
            if total > TOTAL_LIMIT {
                return Err("archive expanded bytes exceed 256 MiB".into());
            }
            let mut meta = HashMap::new();
            meta.insert("sha256".into(), hash(&bytes));
            meta.insert("archive_sha256".into(), artifact.clone());
            meta.insert("bytes".into(), bytes.len().to_string());
            assets.push(Asset {
                key: format!("{root}:{rel}!/{entry}"),
                root: root.into(),
                kind: "jvm-resource".into(),
                path: format!("{rel}!/{entry}"),
                name: entry.clone(),
                meta,
                ..Default::default()
            });
            if !entry.ends_with(".class") {
                continue;
            }
            let (version, key) = if let Some(tail) = entry.strip_prefix("META-INF/versions/") {
                if !multi {
                    continue;
                }
                let Some((version, path)) = tail.split_once('/') else {
                    return Err("invalid multi-release class path".into());
                };
                let version = version
                    .parse::<u32>()
                    .map_err(|_| "invalid multi-release version")?;
                if version < 9 || version > release {
                    continue;
                }
                (version, path.to_string())
            } else {
                (0, entry)
            };
            if selected.get(&key).is_none_or(|(old, _)| version > *old) {
                selected.insert(key, (version, i));
            }
        }
        for (_, (_, index)) in selected {
            let mut member = archive.by_index(index).map_err(|e| e.to_string())?;
            let name = member.name().to_string();
            let mut bytes = Vec::new();
            member
                .by_ref()
                .take(CLASS_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            read_class(&bytes, &name, &mut rows).map_err(|e| format!("{name}: {e}"))?;
        }
    }
    let count = |kind: &str| rows.iter().filter(|r| r["kind"] == kind).count();
    let summary = json!({"kind":"summary","classes":count("class"),"methods":count("method"),"fields":count("field"),"directCalls":count("invoke"),"invokedynamicSites":count("invokedynamic"),"constantDynamicSites":count("constant-dynamic"),"failedClasses":0});
    rows.push(summary);
    let facts = rows
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut parsed = crate::jvm_facts::parse(rel, root, facts.as_bytes());
    if !parsed.errors.is_empty() {
        return Err(parsed.errors.join("; "));
    }
    // Binary inputs have no physical source lines. Never point a Java source
    // request at offsets in the generated JSONL representation.
    for symbol in &mut parsed.symbols {
        symbol.start_line = 0;
        symbol.end_line = 0;
        symbol.modifiers =
            "native bytecode metadata; no recovered Java source or physical source lines".into();
    }
    for reference in &mut parsed.refs {
        reference.line = 0;
    }
    parsed.assets = assets;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    const OWNED_CLASS: &[u8] = include_bytes!("../testdata/jvm/GraphFixture.class");

    #[test]
    fn compiled_owned_class_preserves_exact_overloads_lambda_and_unicode() {
        let parsed = parse("GraphFixture.class", "fixture", OWNED_CLASS, 8);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(
            parsed.symbols.iter().filter(|s| s.kind == "class").count(),
            1
        );
        assert_eq!(
            parsed.symbols.iter().filter(|s| s.kind == "field").count(),
            2
        );
        assert_eq!(
            parsed
                .symbols
                .iter()
                .filter(|s| matches!(s.kind.as_str(), "method" | "constructor"))
                .count(),
            11
        );
        assert!(parsed.refs.iter().any(|r| r.kind == "calls"
            && r.name == "graph/fixture/GraphFixture#pick(Ljava/lang/String;)I"));
        assert!(parsed.refs.iter().any(|r| r.kind == "dynamic-call"));
        assert!(parsed
            .refs
            .iter()
            .any(|r| r.kind == "bootstrap" && r.name.contains("lambda$task$0")));
        let mut rows = Vec::new();
        read_class(OWNED_CLASS, "GraphFixture.class", &mut rows).unwrap();
        assert!(rows
            .iter()
            .any(|r| r["kind"] == "string" && r["value"] == "analysis/猫😀.txt"));
        assert!(parsed.symbols.iter().all(|s| s.start_line == 0));
    }

    fn jar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn ordinary_jar_inventories_resources_and_selects_multi_release_explicitly() {
        let bytes = jar(&[
            (
                "META-INF/MANIFEST.MF",
                b"Manifest-Version: 1.0\r\nMulti-Release: true\r\n",
            ),
            ("graph/fixture/GraphFixture.class", OWNED_CLASS),
            (
                "META-INF/versions/11/graph/fixture/GraphFixture.class",
                OWNED_CLASS,
            ),
            ("analysis/猫😀.txt", b"owned resource"),
        ]);
        for (release, expected) in [
            (8, "entry=graph/fixture/GraphFixture.class;"),
            (
                11,
                "entry=META-INF/versions/11/graph/fixture/GraphFixture.class;",
            ),
        ] {
            let parsed = parse("fixture.jar", "fixture", &bytes, release);
            assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
            assert_eq!(parsed.assets.len(), 4);
            assert!(parsed
                .assets
                .iter()
                .any(|a| a.name == "analysis/猫😀.txt"
                    && a.meta["sha256"] == hash(b"owned resource")));
            let classes: Vec<_> = parsed
                .symbols
                .iter()
                .filter(|s| s.kind == "class")
                .collect();
            assert_eq!(classes.len(), 1);
            assert!(classes[0].signature.contains(expected));
        }
        let broken = jar(&[
            ("graph/fixture/GraphFixture.class", OWNED_CLASS),
            ("broken.class", b"bad"),
        ]);
        let parsed = parse("broken.jar", "fixture", &broken, 8);
        assert!(!parsed.errors.is_empty());
        assert!(
            parsed.symbols.is_empty(),
            "partial Java artifacts must fail atomically"
        );
        let unsafe_path = jar(&[("../escape.class", OWNED_CLASS)]);
        assert!(!parse("unsafe.jar", "fixture", &unsafe_path, 8)
            .errors
            .is_empty());
    }
    #[test]
    fn private_or_truncated_inputs_fail_without_guessing() {
        for data in [
            &b"opaque update data"[..],
            &b"\xca\xfe\xba\xbe\0\0"[..],
            &b"PK\x03\x04"[..],
        ] {
            let parsed = parse("input.jar", "jvm", data, 17);
            assert!(!parsed.errors.is_empty());
            assert!(parsed.symbols.is_empty());
        }
    }
    fn target_pool() -> Pool {
        Pool(vec![
            Cp::Empty,
            Cp::Ref(10, 2, 3),
            Cp::Class(4),
            Cp::Nat(5, 6),
            Cp::Utf8(b"fixture/Target".to_vec()),
            Cp::Utf8(b"work".to_vec()),
            Cp::Utf8(b"(I)V".to_vec()),
        ])
    }
    #[test]
    fn variable_length_opcodes_preserve_the_following_call() {
        let pool = target_pool();
        let wide = vec![
            0x04, 0xc4, 0x36, 0x01, 0x2c, 0xc4, 0x15, 0x01, 0x2c, 0xb8, 0, 1, 0xb1,
        ];
        let mut table = vec![0x03, 0xaa, 0, 0];
        for value in [19i32, 0, 0, 19] {
            table.extend_from_slice(&value.to_be_bytes());
        }
        table.extend_from_slice(&[0x04, 0xb8, 0, 1, 0xb1]);
        let mut lookup = vec![0x03, 0xab, 0, 0];
        for value in [19i32, 1, 7, 19] {
            lookup.extend_from_slice(&value.to_be_bytes());
        }
        lookup.extend_from_slice(&[0x04, 0xb8, 0, 1, 0xb1]);
        for bytes in [wide, table, lookup] {
            let mut rows = Vec::new();
            scan_code(
                &mut rows,
                "Fixture.class",
                "fixture/Caller#run()V",
                &Code {
                    bytes: &bytes,
                    lines: vec![(0, 4)],
                },
                &pool,
                &[],
            )
            .unwrap();
            assert_eq!(rows.iter().filter(|r| r["kind"] == "invoke").count(), 1);
            assert_eq!(rows.last().unwrap()["target"], "fixture/Target#work(I)V");
        }
        let mut rows = Vec::new();
        assert!(scan_code(
            &mut rows,
            "Fixture.class",
            "caller",
            &Code {
                bytes: &[0xaa, 0, 0],
                lines: vec![]
            },
            &pool,
            &[]
        )
        .is_err());
    }
}
