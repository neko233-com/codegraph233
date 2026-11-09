//! Declarative, path-based links for everything that is not code.
//!
//! A project can teach codegraph233 relationships that no frontend could know
//! (audio file stems, design folders, naming conventions) without writing a
//! parser. Rules are bounded, deterministic and only use the file path, so the
//! cost is one glob match per indexed file.

use crate::config::LinkRule;
use crate::model::AssetEdge;

const MAX_LINKS_PER_FILE: usize = 64;

/// Apply `link_rules` to one indexed path.
pub fn apply(rel: &str, root: &str, rules: &[LinkRule]) -> Vec<AssetEdge> {
    if rules.is_empty() {
        return Vec::new();
    }
    let src = format!("asset:{root}:{rel}");
    let mut out = Vec::new();
    for rule in rules.iter().take(32) {
        if !glob_match(&rule.include, rel) {
            continue;
        }
        let Some(dst) = render(&rule.dst, rel, root) else {
            continue;
        };
        if dst.is_empty() {
            continue;
        }
        let dst = if has_scheme(&dst) {
            dst
        } else {
            format!("asset:{root}:{dst}")
        };
        if dst == src {
            continue;
        }
        out.push(AssetEdge {
            src: src.clone(),
            dst,
            kind: rule.kind.clone(),
            detail: rule
                .detail
                .clone()
                .unwrap_or_else(|| "link_rule".to_string()),
            root: root.to_string(),
            line: 0,
        });
        if out.len() >= MAX_LINKS_PER_FILE {
            break;
        }
    }
    out
}

fn has_scheme(value: &str) -> bool {
    let Some(colon) = value.find(':') else {
        return false;
    };
    colon > 0
        && value[..colon]
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Glob with `**` (any depth), `*` (within one segment) and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_here(&p, &t)
}

fn glob_here(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' if p.get(1) == Some(&'*') => {
            // `**/` also matches zero directories.
            let mut rest = &p[2..];
            if rest.first() == Some(&'/') && t.first() != Some(&'/') {
                rest = &rest[1..];
                if glob_here(rest, t) {
                    return true;
                }
            }
            for i in 0..=t.len() {
                if glob_here(rest, &t[i..]) {
                    return true;
                }
            }
            false
        }
        '*' => {
            for i in 0..=t.len() {
                if glob_here(&p[1..], &t[i..]) {
                    return true;
                }
                if i < t.len() && t[i] == '/' {
                    break;
                }
            }
            false
        }
        '?' => !t.is_empty() && t[0] != '/' && glob_here(&p[1..], &t[1..]),
        c => !t.is_empty() && t[0] == c && glob_here(&p[1..], &t[1..]),
    }
}

/// Expand the bounded template placeholders used by link rules.
pub fn render(template: &str, rel: &str, root: &str) -> Option<String> {
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let stem = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    let ext = match name.rfind('.') {
        Some(i) if i + 1 < name.len() => &name[i + 1..],
        _ => "",
    };
    let dir = rel
        .rfind('/')
        .map(|i| &rel[..i])
        .unwrap_or("");
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let key = &after[..close];
        match key {
            "stem" => out.push_str(stem),
            "name" => out.push_str(name),
            "ext" => out.push_str(ext),
            "dir" => out.push_str(dir),
            "path" => out.push_str(rel),
            "root" => out.push_str(root),
            _ => return None,
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(include: &str, dst: &str) -> LinkRule {
        LinkRule {
            include: include.into(),
            kind: "references".into(),
            dst: dst.into(),
            detail: None,
        }
    }

    #[test]
    fn globs_match_segments_and_double_star() {
        assert!(glob_match("Audio/*.wav", "Audio/Jump.wav"));
        assert!(!glob_match("Audio/*.wav", "Audio/Sub/Jump.wav"));
        assert!(glob_match("**/Jump.wav", "Assets/Audio/Jump.wav"));
        assert!(glob_match("Art/*.???", "Art/Hero.png"));
        assert!(!glob_match("Art/*.png", "Art/Hero.jpg"));
    }

    #[test]
    fn templates_render_stem_and_are_bounded() {
        assert_eq!(
            render("audio_event:{stem}", "Audio/Jump_01.wav", "app").as_deref(),
            Some("audio_event:Jump_01")
        );
        assert_eq!(
            render("design:{root}:{stem}", "Docs/Spec.md", "app").as_deref(),
            Some("design:app:Spec")
        );
        assert!(render("x:{unknown}", "a.wav", "app").is_none());
    }

    #[test]
    fn rules_emit_relative_asset_targets() {
        let rules = vec![rule("Audio/*.wav", "audio_event:{stem}")];
        let edges = apply("Audio/Jump.wav", "app", &rules);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].src, "asset:app:Audio/Jump.wav");
        assert_eq!(edges[0].dst, "audio_event:Jump");
        assert_eq!(edges[0].kind, "references");
    }
}
