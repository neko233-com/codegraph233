//! Dependency manifests are relationship dense and cheap: one file describes
//! how a game project is stitched together across package ecosystems. This
//! frontend does not resolve versions or download anything; it records the
//! declared `depends_on` edges so agents can find shared libraries, duplicate
//! versions and the manifests that introduced them.

use crate::model::{Asset, AssetEdge, FileParse};
use regex::Regex;
use std::collections::{BTreeSet, HashMap};

const MAX_DEPS: usize = 2000;

struct Dep {
    ecosystem: &'static str,
    name: String,
    version: String,
    section: String,
}

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    let text = String::from_utf8_lossy(data);
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: lang.to_string(),
        ..Default::default()
    };
    let mut deps = Vec::new();
    let mut meta = HashMap::new();
    let outcome = match lang {
        "deps_npm" => npm(&text, &mut deps, &mut meta),
        "deps_python" => requirements(&text, &mut deps),
        "deps_rust" => cargo_manifest(&text, &mut deps, &mut meta),
        "deps_go" => go_mod(&text, &mut deps, &mut meta),
        "deps_vcpkg" => vcpkg(&text, &mut deps, &mut meta),
        "deps_conan" => conanfile(&text, &mut deps),
        "deps_dotnet" => csproj(&text, &mut deps),
        "deps_java" => pom(&text, &mut deps, &mut meta),
        other => Err(format!("unsupported dependency manifest language {other:?}")),
    };
    if let Err(e) = outcome {
        res.errors.push(e);
    }
    let ecosystem = deps
        .first()
        .map(|d| d.ecosystem)
        .unwrap_or_else(|| meta.get("ecosystem").map(|s| s.as_str()).unwrap_or("unknown"));
    meta.insert("ecosystem".into(), ecosystem.to_string());
    meta.insert("packages".into(), deps.len().to_string());
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: "dependency_manifest".into(),
        path: rel.to_string(),
        name: base_name(rel).to_string(),
        line: 1,
        meta,
        ..Default::default()
    });
    let src = file_key(root, rel);
    let mut seen = BTreeSet::new();
    for dep in deps.into_iter().take(MAX_DEPS) {
        let dst = if dep.version.is_empty() {
            format!("package:{}:{}", dep.ecosystem, dep.name)
        } else {
            format!("package:{}:{}@{}", dep.ecosystem, dep.name, dep.version)
        };
        if !seen.insert(dst.clone()) {
            continue;
        }
        res.asset_edges.push(AssetEdge {
            src: src.clone(),
            dst,
            kind: "depends_on".into(),
            detail: dep.section,
            root: root.to_string(),
            line: 0,
        });
    }
    res
}

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn base_name(rel: &str) -> &str {
    rel.rsplit(['/', '\\']).next().unwrap_or(rel)
}

fn clean_version(v: &str) -> String {
    let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
    let v = v.trim_start_matches(['^', '~', '=', '<', '>', '!']);
    v.trim().to_string()
}

fn npm(text: &str, out: &mut Vec<Dep>, meta: &mut HashMap<String, String>) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("package.json is invalid: {e}"))?;
    if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
        meta.insert("package_name".into(), name.to_string());
    }
    if let Some(version) = value.get("version").and_then(|v| v.as_str()) {
        meta.insert("package_version".into(), version.to_string());
    }
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        let Some(obj) = value.get(section).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, version) in obj {
            let version = version.as_str().map(clean_version).unwrap_or_default();
            out.push(Dep {
                ecosystem: "npm",
                name: name.clone(),
                version,
                section: section.to_string(),
            });
        }
    }
    Ok(())
}

fn requirements(text: &str, out: &mut Vec<Dep>) -> Result<(), String> {
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }
        let cut = line
            .find(|c: char| {
                matches!(c, '<' | '>' | '=' | '!' | '~' | ';' | ' ' | '[') || c == '@'
            })
            .unwrap_or(line.len());
        let name = line[..cut].trim();
        if name.is_empty() {
            continue;
        }
        let rest = line[cut..].trim();
        let rest = rest.split(';').next().unwrap_or("").trim();
        let version = if rest.starts_with('@') {
            String::new()
        } else {
            clean_version(rest.split_whitespace().next().unwrap_or(""))
        };
        out.push(Dep {
            ecosystem: "pypi",
            name: name.to_string(),
            version,
            section: "requirements".into(),
        });
    }
    Ok(())
}

fn cargo_manifest(
    text: &str,
    out: &mut Vec<Dep>,
    meta: &mut HashMap<String, String>,
) -> Result<(), String> {
    let mut section = String::new();
    let mut in_deps = false;
    let version_re = Regex::new(r#"version\s*=\s*"([^"]+)""#).unwrap();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            in_deps = section == "dependencies"
                || section == "dev-dependencies"
                || section == "build-dependencies"
                || section == "workspace.dependencies"
                || (section.starts_with("target.") && section.ends_with(".dependencies"));
            continue;
        }
        if !in_deps {
            if let Some(name) = line.strip_prefix("name =") {
                meta.insert("package_name".into(), name.trim().trim_matches('"').to_string());
            }
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(' ') {
            continue;
        }
        let value = value.trim();
        let version = if value.starts_with('"') {
            clean_version(value)
        } else {
            version_re
                .captures(value)
                .map(|c| clean_version(&c[1]))
                .unwrap_or_default()
        };
        out.push(Dep {
            ecosystem: "cargo",
            name: name.to_string(),
            version,
            section: section.clone(),
        });
    }
    Ok(())
}

fn go_mod(text: &str, out: &mut Vec<Dep>, meta: &mut HashMap<String, String>) -> Result<(), String> {
    let mut in_require = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(module) = line.strip_prefix("module ") {
            meta.insert("package_name".into(), module.trim().to_string());
            continue;
        }
        if line == "require (" {
            in_require = true;
            continue;
        }
        if in_require && line == ")" {
            in_require = false;
            continue;
        }
        let dep = line.strip_prefix("require ").unwrap_or(line);
        if !in_require && !line.starts_with("require ") {
            continue;
        }
        let mut parts = dep.split_whitespace();
        let (Some(name), Some(version)) = (parts.next(), parts.next()) else {
            continue;
        };
        out.push(Dep {
            ecosystem: "go",
            name: name.to_string(),
            version: clean_version(version),
            section: "require".into(),
        });
    }
    Ok(())
}

fn vcpkg(text: &str, out: &mut Vec<Dep>, meta: &mut HashMap<String, String>) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("vcpkg.json is invalid: {e}"))?;
    if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
        meta.insert("package_name".into(), name.to_string());
    }
    let Some(deps) = value.get("dependencies").and_then(|v| v.as_array()) else {
        return Ok(());
    };
    for dep in deps {
        let (name, version) = match dep {
            serde_json::Value::String(s) => (s.clone(), String::new()),
            serde_json::Value::Object(o) => {
                let name = o
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let version = o
                    .get("version>=")
                    .or_else(|| o.get("version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                (name, clean_version(version))
            }
            _ => continue,
        };
        if name.is_empty() {
            continue;
        }
        out.push(Dep {
            ecosystem: "vcpkg",
            name,
            version,
            section: "dependencies".into(),
        });
    }
    Ok(())
}

fn conanfile(text: &str, out: &mut Vec<Dep>) -> Result<(), String> {
    let mut in_requires = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_requires = line.eq_ignore_ascii_case("[requires]");
            continue;
        }
        if !in_requires {
            continue;
        }
        let (name, version) = match line.split_once('/') {
            Some((name, rest)) => (
                name.trim().to_string(),
                clean_version(rest.split('@').next().unwrap_or(rest)),
            ),
            None => (line.to_string(), String::new()),
        };
        if name.is_empty() {
            continue;
        }
        out.push(Dep {
            ecosystem: "conan",
            name,
            version,
            section: "requires".into(),
        });
    }
    Ok(())
}

fn csproj(text: &str, out: &mut Vec<Dep>) -> Result<(), String> {
    if !text.contains("<Project") {
        return Err("not an MSBuild project (missing <Project>)".into());
    }
    let open = Regex::new(r#"(?s)<PackageReference\s+Include="([^"]+)"([^>]*)>"#).unwrap();
    let version_attr = Regex::new(r#"Version="([^"]+)""#).unwrap();
    let version_tag = Regex::new(r"(?s)<Version>\s*([^<]+?)\s*</Version>").unwrap();
    let mut seen = BTreeSet::new();
    for caps in open.captures_iter(text) {
        let name = caps[1].trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        let tail = &caps[2];
        let version = if let Some(v) = version_attr.captures(tail) {
            clean_version(&v[1])
        } else if tail.trim_end().ends_with('/') {
            String::new()
        } else {
            let after = &text[caps.get(0).unwrap().end()..];
            let body = after.split("</PackageReference>").next().unwrap_or("");
            version_tag
                .captures(body)
                .map(|c| clean_version(&c[1]))
                .unwrap_or_default()
        };
        out.push(Dep {
            ecosystem: "nuget",
            name: name.to_string(),
            version,
            section: "PackageReference".into(),
        });
    }
    Ok(())
}

fn pom(text: &str, out: &mut Vec<Dep>, meta: &mut HashMap<String, String>) -> Result<(), String> {
    if !text.contains("<project") {
        return Err("not a Maven POM (missing <project>)".into());
    }
    let dep_re = Regex::new(r"(?s)<dependency>(.*?)</dependency>").unwrap();
    let field = |body: &str, tag: &str| {
        let re = Regex::new(&format!(r"(?s)<{tag}>\s*([^<]+?)\s*</{tag}>")).unwrap();
        re.captures(body)
            .map(|c| c[1].trim().to_string())
            .unwrap_or_default()
    };
    let artifact_re = Regex::new(r"(?s)<artifactId>\s*([^<]+?)\s*</artifactId>").unwrap();
    for caps in dep_re.captures_iter(text) {
        let body = &caps[1];
        let name = field(body, "artifactId");
        if name.is_empty() {
            continue;
        }
        if meta.get("package_name").is_none() {
            if let Some(first) = artifact_re.captures(text) {
                meta.insert("package_name".into(), first[1].trim().to_string());
            }
        }
        let mut full = name.clone();
        let group = field(body, "groupId");
        if !group.is_empty() {
            full = format!("{group}:{name}");
        }
        out.push(Dep {
            ecosystem: "maven",
            name: full,
            version: field(body, "version"),
            section: "dependency".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(fp: &FileParse) -> Vec<(String, String)> {
        fp.asset_edges
            .iter()
            .map(|e| (e.dst.clone(), e.kind.clone()))
            .collect()
    }

    #[test]
    fn npm_reads_dependencies_and_metadata() {
        let text = r#"{"name":"game","version":"1.0.0","dependencies":{"three":"^0.160.0"},"devDependencies":{"vite":"5.0.0"}}"#;
        let fp = parse("package.json", "app", "deps_npm", text.as_bytes());
        let got = edges(&fp);
        assert!(got.contains(&("package:npm:three@0.160.0".into(), "depends_on".into())));
        assert!(got.contains(&("package:npm:vite@5.0.0".into(), "depends_on".into())));
        assert_eq!(fp.assets[0].meta.get("packages").map(|s| s.as_str()), Some("2"));
    }

    #[test]
    fn python_and_conan_requirements_are_parsed() {
        let py = parse(
            "requirements.txt",
            "app",
            "deps_python",
            b"requests==2.31.0\nnumpy>=1.26 # math\n-e .\n",
        );
        assert!(py
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:pypi:requests@2.31.0"));
        let conan = parse(
            "conanfile.txt",
            "app",
            "deps_conan",
            b"[requires]\nfmt/10.2.1\nzlib/1.3@user/stable\n",
        );
        assert!(conan
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:conan:fmt@10.2.1"));
    }

    #[test]
    fn cargo_go_and_csproj_manifests_are_parsed() {
        let cargo = parse(
            "Cargo.toml",
            "app",
            "deps_rust",
            b"[package]\nname = \"game\"\n\n[dependencies]\nserde = { version = \"1.0\", features = [\"derive\"] }\nanyhow = \"1\"\n",
        );
        assert!(cargo
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:cargo:serde@1.0"));
        let go = parse(
            "go.mod",
            "app",
            "deps_go",
            b"module game\n\ngo 1.22\n\nrequire (\n\tgithub.com/x/y v1.2.3\n)\n",
        );
        assert!(go
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:go:github.com/x/y@v1.2.3"));
        let csproj = parse(
            "Game.csproj",
            "app",
            "deps_dotnet",
            br#"<Project><ItemGroup><PackageReference Include="Newtonsoft.Json" Version="13.0.3" /><PackageReference Include="Units"><Version>2.0.0</Version></PackageReference></ItemGroup></Project>"#,
        );
        assert!(csproj
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:nuget:Newtonsoft.Json@13.0.3"));
        assert!(csproj
            .asset_edges
            .iter()
            .any(|e| e.dst == "package:nuget:Units@2.0.0"));
    }

    #[test]
    fn malformed_manifests_are_visible() {
        let fp = parse("package.json", "app", "deps_npm", b"{oops");
        assert!(!fp.errors.is_empty());
        assert!(fp.errors[0].contains("package.json is invalid"));
    }
}
