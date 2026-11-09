//! CI/build pipeline frontends: Jenkinsfile, GitHub Actions, GitLab CI,
//! Azure Pipelines, CircleCI and Makefiles. These are deliberately read as
//! line-oriented declarative graphs (pipeline -> stage/job -> step -> action),
//! not as a general YAML/Groovy parser. No command is executed.

use crate::model::{Asset, AssetEdge, FileParse};
use std::collections::HashMap;

const MAX_STAGES: usize = 200;
const MAX_JOBS: usize = 200;
const MAX_STEPS: usize = 500;
const MAX_TARGETS: usize = 200;
const MAX_RUN_REFS: usize = 64;

pub fn parse(rel: &str, root: &str, lang: &str, data: &[u8]) -> FileParse {
    let text = String::from_utf8_lossy(data);
    match lang {
        "jenkinsfile" => jenkins(rel, root, &text),
        "github_actions" => github_actions(rel, root, &text),
        "gitlab_ci" => gitlab_ci(rel, root, &text),
        "azure_pipeline" => azure_pipeline(rel, root, &text),
        "circleci" => circleci(rel, root, &text),
        "makefile" => makefile(rel, root, &text),
        other => FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: other.to_string(),
            errors: vec![format!("no CI frontend for {other:?}")],
            ..Default::default()
        },
    }
}

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

fn push_file(
    res: &mut FileParse,
    rel: &str,
    root: &str,
    kind: &str,
    name: &str,
    mut meta: HashMap<String, String>,
) {
    meta.entry("format".into()).or_insert_with(|| kind.to_string());
    res.assets.push(Asset {
        key: file_key(root, rel),
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: 1,
        meta,
    });
}

fn push_child(
    res: &mut FileParse,
    rel: &str,
    root: &str,
    kind: &str,
    name: &str,
    line: usize,
    meta: HashMap<String, String>,
) -> String {
    let key = format!(
        "asset:{root}:{rel}#{kind}:{}:{}",
        slugify(name),
        line
    );
    res.assets.push(Asset {
        key: key.clone(),
        root: root.to_string(),
        kind: kind.to_string(),
        path: rel.to_string(),
        name: name.to_string(),
        unity_id: String::new(),
        line: line as i32,
        meta,
    });
    key
}

fn edge(
    res: &mut FileParse,
    root: &str,
    src: &str,
    dst: &str,
    kind: &str,
    detail: &str,
    line: usize,
) {
    if src.is_empty() || dst.is_empty() {
        return;
    }
    res.asset_edges.push(AssetEdge {
        src: src.to_string(),
        dst: dst.to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
        root: root.to_string(),
        line: line as i32,
    });
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_' | '.' | '/' | ':') && !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "unnamed".into()
    } else {
        out
    }
}

fn meta(pairs: &[(&str, String)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// ------------------------------------------------------------------ Jenkins

fn jenkins(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "jenkinsfile".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut stages = 0usize;
    let mut commands = 0usize;
    let mut current_stage: Option<String> = None;
    let mut stage_names: Vec<String> = Vec::new();
    let mut run_refs = 0usize;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_start();
        if line.starts_with("//") {
            continue;
        }
        if let Some(name) = jenkins_stage(line) {
            if stages < MAX_STAGES {
                let key = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_stage",
                    &name,
                    line_no,
                    meta(&[("pipeline", "jenkins".into())]),
                );
                edge(&mut res, root, &file, &key, "contains", "stage", line_no);
                stage_names.push(name.clone());
                current_stage = Some(key);
            }
            stages += 1;
            continue;
        }
        if let Some((runner, command)) = jenkins_command(line) {
            commands += 1;
            let src = current_stage.as_deref().unwrap_or(&file);
            if run_refs < MAX_RUN_REFS {
                if let Some(target) = command_script_ref(command) {
                    edge(
                        &mut res,
                        root,
                        src,
                        &format!("asset:{root}:{target}"),
                        "runs",
                        runner,
                        line_no,
                    );
                    run_refs += 1;
                }
            }
        }
    }

    let mut m = meta(&[
        ("stages".into(), stages.to_string()),
        ("stage_names".into(), stage_names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("commands".into(), commands.to_string()),
    ]);
    m.insert("format".into(), "jenkins".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

fn jenkins_stage(line: &str) -> Option<String> {
    let pos = line.find("stage(")?;
    let after = &line[pos + 6..];
    let quote = after.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let rest = &after[1..];
    let end = rest.find(quote)?;
    let name = rest[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn jenkins_command(line: &str) -> Option<(&'static str, &str)> {
    for runner in ["sh", "bat", "powershell", "pwsh"] {
        let mut after: Option<&str> = None;
        let paren = format!("{runner}(");
        if let Some(pos) = line.find(&paren) {
            after = Some(&line[pos + paren.len()..]);
        } else {
            let spaced = format!("{runner} ");
            if let Some(pos) = line.find(&spaced) {
                after = Some(line[pos + spaced.len()..].trim_start());
            }
        }
        let Some(after) = after else { continue };
        let quote = after.chars().next()?;
        if quote != '\'' && quote != '"' {
            return None;
        }
        let rest = &after[1..];
        let end = rest.find(quote)?;
        return Some((runner, &rest[..end]));
    }
    None
}

/// A command often names a script (`./ci/build.sh`, `tools/pack.ps1`). Only
/// path-shaped first tokens are linked; shell built-ins and flags are ignored.
fn command_script_ref(command: &str) -> Option<String> {
    let token = command.split_whitespace().next()?;
    if token.starts_with('-') || token.starts_with('$') || token == "echo" {
        return None;
    }
    let cleaned = token.trim_matches(|c| c == '\'' || c == '"');
    let lower = cleaned.to_ascii_lowercase();
    let shaped = lower.ends_with(".sh")
        || lower.ends_with(".ps1")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
        || lower.ends_with(".py")
        || lower.ends_with(".gradle")
        || cleaned.contains('/');
    if !shaped {
        return None;
    }
    let rel = cleaned.trim_start_matches("./").trim_start_matches(".\\");
    normalize_rel(rel)
}

fn normalize_rel(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("/"))
    }
}

// --------------------------------------------------------- GitHub Actions

fn github_actions(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "github_actions".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut jobs = 0usize;
    let mut steps = 0usize;
    let mut actions = 0usize;
    let mut job_names: Vec<String> = Vec::new();
    let mut current_job: Option<String> = None;
    let mut in_jobs = false;
    let mut in_steps = false;
    let mut pending_step: Option<(String, usize)> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if indent == 0 {
            in_jobs = line == "jobs:";
            in_steps = false;
            current_job = None;
            pending_step = None;
            continue;
        }
        if !in_jobs {
            continue;
        }
        if indent == 2 && !line.starts_with('-') && line.ends_with(':') {
            let name = line.trim_end_matches(':').trim();
            if !name.is_empty() && jobs < MAX_JOBS {
                let key = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_job",
                    name,
                    line_no,
                    meta(&[("pipeline", "github_actions".into())]),
                );
                edge(&mut res, root, &file, &key, "contains", "job", line_no);
                job_names.push(name.to_string());
                current_job = Some(key);
                in_steps = false;
                pending_step = None;
                jobs += 1;
            }
            continue;
        }
        if line == "steps:" {
            in_steps = true;
            continue;
        }
        if !in_steps {
            continue;
        }
        let step_item = line.strip_prefix("- ").map(|s| s.trim());
        if let Some(item) = step_item {
            if steps < MAX_STEPS {
                let (kind, name) = step_descriptor(item);
                if let Some(job) = &current_job {
                    let key = push_child(
                        &mut res,
                        rel,
                        root,
                        "ci_step",
                        &name,
                        line_no,
                        meta(&[("step_kind".into(), kind.to_string())]),
                    );
                    edge(&mut res, root, job, &key, "contains", kind, line_no);
                    if let Some((field, value)) = yaml_key_value(item) {
                        if field == "uses" && !value.is_empty() {
                            edge(
                                &mut res,
                                root,
                                job,
                                &format!("action:{}", value.trim_matches('"')),
                                "uses_action",
                                "uses",
                                line_no,
                            );
                            actions += 1;
                        }
                    }
                    pending_step = Some((key, line_no));
                    steps += 1;
                }
            }
            continue;
        }
        if let Some((key, _)) = &pending_step {
            if let Some((field, value)) = yaml_key_value(line) {
                match field {
                    "uses" => {
                        if !value.is_empty() {
                            let src = current_job.as_deref().unwrap_or(key);
                            edge(
                                &mut res,
                                root,
                                src,
                                &format!("action:{}", value.trim_matches('"')),
                                "uses_action",
                                "uses",
                                line_no,
                            );
                            actions += 1;
                        }
                    }
                    "run" => {
                        let command = value.trim_matches('"');
                        if let Some(target) = command_script_ref(command) {
                            edge(
                                &mut res,
                                root,
                                key,
                                &format!("asset:{root}:{target}"),
                                "runs",
                                "run",
                                line_no,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut m = meta(&[
        ("jobs".into(), jobs.to_string()),
        ("job_names".into(), job_names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("steps".into(), steps.to_string()),
        ("actions".into(), actions.to_string()),
    ]);
    m.insert("format".into(), "github_actions".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

fn step_descriptor(item: &str) -> (&'static str, String) {
    if let Some((field, value)) = yaml_key_value(item) {
        match field {
            "uses" => return ("uses", value.trim_matches('"').to_string()),
            "run" => {
                let text = value.trim_matches('"').chars().take(48).collect::<String>();
                return ("run", text);
            }
            "name" => return ("name", value.trim_matches('"').to_string()),
            other => return ("step", other.to_string()),
        }
    }
    ("step", item.trim_matches('"').to_string())
}

fn yaml_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

// ---------------------------------------------------------------- GitLab CI

fn gitlab_ci(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "gitlab_ci".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut jobs = 0usize;
    let mut stages = 0usize;
    let mut job_names: Vec<String> = Vec::new();
    let mut stage_keys: HashMap<String, String> = HashMap::new();
    let mut current_job: Option<String> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if indent == 0 && !line.starts_with('-') {
            current_job = None;
            if !line.ends_with(':') {
                continue;
            }
            let name = line.trim_end_matches(':').trim();
            if name.is_empty() || is_gitlab_reserved(name) {
                continue;
            }
            if jobs < MAX_JOBS {
                let key = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_job",
                    name,
                    line_no,
                    meta(&[("pipeline", "gitlab_ci".into())]),
                );
                edge(&mut res, root, &file, &key, "contains", "job", line_no);
                job_names.push(name.to_string());
                current_job = Some(key);
                jobs += 1;
            }
            continue;
        }
        if let Some(job) = &current_job {
            if let Some((field, value)) = yaml_key_value(line) {
                if field == "stage" {
                    let stage = value.trim_matches('"').to_string();
                    if !stage.is_empty() {
                        let key = stage_keys
                            .entry(stage.clone())
                            .or_insert_with(|| {
                                if stages < MAX_STAGES {
                                    stages += 1;
                                }
                                let key = format!(
                                    "asset:{root}:{rel}#ci_stage:{}",
                                    slugify(&stage)
                                );
                                res.assets.push(Asset {
                                    key: key.clone(),
                                    root: root.to_string(),
                                    kind: "ci_stage".into(),
                                    path: rel.to_string(),
                                    name: stage.clone(),
                                    unity_id: String::new(),
                                    line: line_no as i32,
                                    meta: meta(&[("pipeline", "gitlab_ci".into())]),
                                });
                                edge(
                                    &mut res,
                                    root,
                                    &file,
                                    &key,
                                    "contains",
                                    "stage",
                                    line_no,
                                );
                                key
                            })
                            .clone();
                        edge(&mut res, root, job, &key, "uses_stage", "stage", line_no);
                    }
                }
            }
        }
    }

    let mut m = meta(&[
        ("jobs".into(), jobs.to_string()),
        ("job_names".into(), job_names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("stages".into(), stage_keys.len().to_string()),
    ]);
    m.insert("format".into(), "gitlab_ci".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

fn is_gitlab_reserved(name: &str) -> bool {
    matches!(
        name,
        "stages"
            | "variables"
            | "default"
            | "include"
            | "image"
            | "services"
            | "cache"
            | "before_script"
            | "after_script"
            | "workflow"
            | "pages"
    )
}

// ---------------------------------------------------------- Azure Pipelines

fn azure_pipeline(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "azure_pipeline".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut stages = 0usize;
    let mut jobs = 0usize;
    let mut steps = 0usize;
    let mut names: Vec<String> = Vec::new();
    let mut current_stage: Option<String> = None;
    let mut current_job: Option<String> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let item = line.trim_start_matches("- ").trim();
        let Some((key, value)) = item.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"');
        match key {
            "stage" if !value.is_empty() && stages < MAX_STAGES => {
                let k = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_stage",
                    value,
                    line_no,
                    meta(&[("pipeline", "azure".into())]),
                );
                edge(&mut res, root, &file, &k, "contains", "stage", line_no);
                names.push(value.to_string());
                current_stage = Some(k);
                stages += 1;
            }
            "job" if !value.is_empty() && jobs < MAX_JOBS => {
                let k = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_job",
                    value,
                    line_no,
                    meta(&[("pipeline", "azure".into())]),
                );
                edge(
                    &mut res,
                    root,
                    current_stage.as_deref().unwrap_or(&file),
                    &k,
                    "contains",
                    "job",
                    line_no,
                );
                current_job = Some(k);
                jobs += 1;
            }
            "task" | "script" | "bash" | "pwsh" | "powershell" | "checkout" if steps < MAX_STEPS => {
                let label = if value.is_empty() { key } else { value };
                let k = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_step",
                    label,
                    line_no,
                    meta(&[("step_kind".into(), key.to_string())]),
                );
                edge(
                    &mut res,
                    root,
                    current_job.as_deref().unwrap_or(&file),
                    &k,
                    "contains",
                    "step",
                    line_no,
                );
                steps += 1;
            }
            "uses" => {
                if let Some(src) = &current_job {
                    edge(
                        &mut res,
                        root,
                        src,
                        &format!("action:{}", value),
                        "uses_action",
                        "uses",
                        line_no,
                    );
                }
            }
            _ => {}
        }
    }

    let mut m = meta(&[
        ("stages".into(), stages.to_string()),
        ("stage_names".into(), names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("jobs".into(), jobs.to_string()),
        ("steps".into(), steps.to_string()),
    ]);
    m.insert("format".into(), "azure".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

// ---------------------------------------------------------------- CircleCI

fn circleci(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "circleci".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut jobs = 0usize;
    let mut steps = 0usize;
    let mut job_names: Vec<String> = Vec::new();
    let mut in_jobs = false;
    let mut current_job: Option<String> = None;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if indent == 0 {
            in_jobs = line == "jobs:";
            current_job = None;
            continue;
        }
        if !in_jobs {
            continue;
        }
        if indent == 2 && line.ends_with(':') {
            let name = line.trim_end_matches(':').trim();
            if jobs < MAX_JOBS && !name.is_empty() {
                let key = push_child(
                    &mut res,
                    rel,
                    root,
                    "ci_job",
                    name,
                    line_no,
                    meta(&[("pipeline", "circleci".into())]),
                );
                edge(&mut res, root, &file, &key, "contains", "job", line_no);
                job_names.push(name.to_string());
                current_job = Some(key);
                jobs += 1;
            }
            continue;
        }
        if let Some(job) = &current_job {
            if let Some(item) = line.strip_prefix("- ") {
                let name = item.split([':', ' ']).next().unwrap_or(item).trim();
                if steps < MAX_STEPS {
                    let key = push_child(
                        &mut res,
                        rel,
                        root,
                        "ci_step",
                        name,
                        line_no,
                        meta(&[("step_kind".into(), name.to_string())]),
                    );
                    edge(&mut res, root, job, &key, "contains", "step", line_no);
                    steps += 1;
                }
            } else if let Some((field, value)) = yaml_key_value(line) {
                if (field == "run" || field == "task") && !value.is_empty() {
                    let label = value.trim_matches('"');
                    if steps < MAX_STEPS {
                        let key = push_child(
                            &mut res,
                            rel,
                            root,
                            "ci_step",
                            label,
                            line_no,
                            meta(&[("step_kind".into(), field.to_string())]),
                        );
                        edge(&mut res, root, job, &key, "contains", "step", line_no);
                        steps += 1;
                    }
                }
            }
        }
    }

    let mut m = meta(&[
        ("jobs".into(), jobs.to_string()),
        ("job_names".into(), job_names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("steps".into(), steps.to_string()),
    ]);
    m.insert("format".into(), "circleci".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

// ------------------------------------------------------------------ Make

fn makefile(rel: &str, root: &str, text: &str) -> FileParse {
    let mut res = FileParse {
        rel_path: rel.to_string(),
        root: root.to_string(),
        lang: "makefile".into(),
        module: rel.to_string(),
        ..Default::default()
    };
    let file = file_key(root, rel);
    let mut targets: Vec<(String, Vec<String>, usize, usize)> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim_end();
        if line.starts_with('\t') {
            if let Some(last) = targets.last_mut() {
                last.3 += 1;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(colon) = trimmed.find(':') else {
            continue;
        };
        if trimmed[colon..].starts_with(":=") || trimmed[colon..].starts_with("::=") {
            continue;
        }
        let head = trimmed[..colon].trim();
        if head.is_empty() || head.starts_with('.') || head.contains('=') {
            continue;
        }
        let name = head.split_whitespace().next().unwrap_or(head);
        if name.is_empty() || targets.len() >= MAX_TARGETS {
            continue;
        }
        if targets.iter().any(|t| t.0 == name) {
            continue;
        }
        let prereqs: Vec<String> = trimmed[colon + 1..]
            .split_whitespace()
            .filter(|p| !p.starts_with('|') && !p.starts_with('#'))
            .map(|p| p.trim_end_matches('\\').to_string())
            .filter(|p| !p.is_empty())
            .collect();
        targets.push((name.to_string(), prereqs, line_no, 0));
    }

    let names: Vec<String> = targets.iter().map(|t| t.0.clone()).collect();
    let mut keys: HashMap<String, String> = HashMap::new();
    for (name, _, line_no, commands) in &targets {
        let key = push_child(
            &mut res,
            rel,
            root,
            "ci_target",
            name,
            *line_no,
            meta(&[("commands".into(), commands.to_string())]),
        );
        edge(&mut res, root, &file, &key, "contains", "target", *line_no);
        keys.insert(name.clone(), key);
    }
    let mut links = 0usize;
    for (name, prereqs, line_no, _) in &targets {
        let Some(src) = keys.get(name) else { continue };
        for p in prereqs {
            if let Some(dst) = keys.get(p) {
                edge(&mut res, root, src, dst, "depends_on", "make", *line_no);
                links += 1;
            }
        }
    }

    let mut m = meta(&[
        ("targets".into(), targets.len().to_string()),
        ("target_names".into(), names.iter().take(20).cloned().collect::<Vec<_>>().join(",")),
        ("target_links".into(), links.to_string()),
    ]);
    m.insert("format".into(), "make".into());
    push_file(&mut res, rel, root, "ci_pipeline", base_name(rel), m);
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_asset(fp: &FileParse, kind: &str) -> bool {
        fp.assets.iter().any(|a| a.kind == kind)
    }

    fn has_edge(fp: &FileParse, src_contains: &str, dst_contains: &str, kind: &str) -> bool {
        fp.asset_edges.iter().any(|e| {
            e.kind == kind && e.src.contains(src_contains) && e.dst.contains(dst_contains)
        })
    }

    #[test]
    fn jenkinsfile_extracts_stages_and_script_refs() {
        let text = r#"
pipeline {
  agent any
  stages {
    stage('Build') {
      steps { sh './ci/build.sh' }
    }
    stage("Test") {
      steps { bat 'tools\\test.ps1' }
    }
  }
}
"#;
        let fp = parse("Jenkinsfile", "app", "jenkinsfile", text.as_bytes());
        assert!(fp.errors.is_empty());
        assert!(has_asset(&fp, "ci_stage"));
        assert!(has_asset(&fp, "ci_pipeline"));
        assert!(has_edge(&fp, "Jenkinsfile", "ci_stage:build", "contains"));
        assert!(has_edge(&fp, "ci_stage:build", "asset:app:ci/build.sh", "runs"));
        assert!(has_edge(&fp, "ci_stage:test", "asset:app:tools/test.ps1", "runs"));
    }

    #[test]
    fn github_actions_extracts_jobs_steps_and_actions() {
        let text = r#"
name: CI
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test
  pack:
    steps:
      - name: Package
        uses: actions/upload-artifact@v4
"#;
        let fp = parse(
            ".github/workflows/ci.yml",
            "app",
            "github_actions",
            text.as_bytes(),
        );
        assert!(fp.errors.is_empty());
        assert!(has_edge(&fp, "ci_job:build", "action:actions/checkout@v4", "uses_action"));
        assert!(has_edge(&fp, "ci_job:pack", "action:actions/upload-artifact@v4", "uses_action"));
        assert!(has_asset(&fp, "ci_step"));
    }

    #[test]
    fn gitlab_azure_and_circleci_pipelines_are_structured() {
        let gitlab = "stages:\n  - build\n\nbuild:\n  stage: build\n  script:\n    - make\n\ntest:\n  stage: test\n  script:\n    - make test\n";
        let fp = parse(".gitlab-ci.yml", "app", "gitlab_ci", gitlab.as_bytes());
        assert!(has_edge(&fp, "ci_job:build", "ci_stage:build", "uses_stage"));
        assert!(has_edge(&fp, "ci_job:test", "ci_stage:test", "uses_stage"));

        let azure = "stages:\n- stage: Build\n  jobs:\n  - job: Compile\n    steps:\n    - script: cargo build\n    - task: PublishBuildArtifacts@1\n";
        let fp = parse("azure-pipelines.yml", "app", "azure_pipeline", azure.as_bytes());
        assert!(has_edge(&fp, "ci_stage:build", "ci_job:compile", "contains"));
        assert!(has_asset(&fp, "ci_step"));

        let circle = "version: 2.1\njobs:\n  build:\n    steps:\n      - checkout\n      - run: cargo test\n";
        let fp = parse(".circleci/config.yml", "app", "circleci", circle.as_bytes());
        assert!(has_edge(&fp, "ci_job:build", "ci_step:checkout", "contains"));
        assert!(has_asset(&fp, "ci_step"));
    }

    #[test]
    fn makefile_links_declared_target_dependencies() {
        let text = "all: build test\n\t@echo done\n\nbuild:\n\tcargo build\n\ntest: build\n\tcargo test\n";
        let fp = parse("Makefile", "app", "makefile", text.as_bytes());
        assert!(fp.errors.is_empty());
        assert!(has_edge(&fp, "ci_target:all", "ci_target:build", "depends_on"));
        assert!(has_edge(&fp, "ci_target:test", "ci_target:build", "depends_on"));
        assert!(has_edge(&fp, "Makefile", "ci_target:all", "contains"));
    }
}
