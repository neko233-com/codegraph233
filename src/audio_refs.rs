//! Audio event bindings in gameplay code.
//!
//! Agents asking "which code plays this Wwise/FMOD event?" should not need to
//! grep a whole project. This scanner is intentionally cheap: no parsing, no
//! allocation per line, only a single pass over bytes for a handful of known
//! middleware call shapes. It emits edges to `audio_event:{name}` pseudo keys;
//! `Engine::audio_events` resolves those names against Wwise/FMOD event assets.

use crate::model::AssetEdge;

const MAX_REFS_PER_FILE: usize = 200;

const NEEDLES: &[&[u8]] = &[
    b"PostEvent",
    b"PostEventAtLocation",
    b"PlayOneShot",
    b"playOneShot",
    b"createEvent",
    b"CreateEvent",
];

fn file_key(root: &str, rel: &str) -> String {
    format!("asset:{root}:{rel}")
}

/// True when the byte offset sits on a comment line (`//`, `#`, `--`, `*`).
/// Cheap approximation: only looks at the first non-space bytes of the line.
fn on_comment_line(text: &str, pos: usize) -> bool {
    let start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &text[start..pos];
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("--")
        || trimmed.starts_with('*')
        || trimmed.starts_with("/*")
}

/// Extract the first quoted string after the opening parenthesis following
/// `pos`. Returns the event name and the byte offset of the quote.
fn event_string(text: &str, pos: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut i = pos;
    while i < bytes.len() && i < pos + 128 {
        match bytes[i] {
            b'(' | b' ' | b'\t' => i += 1,
            b'"' | b'\'' => {
                let quote = bytes[i];
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && j < start + 256 {
                    if bytes[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == quote {
                        let raw = &text[start..j];
                        if !raw.is_empty() && raw.len() <= 160 {
                            return Some((raw.to_string(), i));
                        }
                        return None;
                    }
                    j += 1;
                }
                return None;
            }
            _ => return None,
        }
    }
    None
}

/// Scan one source file for middleware play calls.
pub fn scan(rel: &str, root: &str, text: &str) -> Vec<AssetEdge> {
    let mut out = Vec::new();
    let src = file_key(root, rel);
    let bytes = text.as_bytes();
    let mut seen: Vec<(String, i32)> = Vec::new();
    for needle in NEEDLES {
        let mut from = 0;
        while let Some(found) = find(&bytes[from..], needle) {
            let at = from + found;
            if out.len() >= MAX_REFS_PER_FILE {
                return out;
            }
            let after = at + needle.len();
            if after < bytes.len() && (bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_') {
                from = after;
                continue;
            }
            if !on_comment_line(text, at) {
                if let Some((name, quote_at)) = event_string(text, after) {
                    let line = text[..quote_at].bytes().filter(|b| *b == b'\n').count() as i32 + 1;
                    if !seen.contains(&(name.clone(), line)) {
                        seen.push((name.clone(), line));
                        out.push(AssetEdge {
                            src: src.clone(),
                            dst: format!("audio_event:{name}"),
                            kind: "plays_audio".into(),
                            detail: String::from_utf8_lossy(needle).to_string(),
                            root: root.to_string(),
                            line,
                        });
                    }
                }
            }
            from = after;
        }
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_wwise_and_fmod_calls_with_lines() {
        let src = r#"
class Audio {
    void Play() {
        AkSoundEngine.PostEvent("Play_Footstep", gameObject);
        RuntimeManager.PlayOneShot("event:/UI/Click");
        // AkSoundEngine.PostEvent("Commented", this);
    }
}
"#;
        let refs = scan("src/Audio.cs", "app", src);
        assert!(refs.iter().any(|r| r.dst == "audio_event:Play_Footstep" && r.line == 4));
        assert!(refs.iter().any(|r| r.dst == "audio_event:event:/UI/Click" && r.line == 5));
        assert!(!refs.iter().any(|r| r.dst.contains("Commented")));
    }

    #[test]
    fn scans_are_capped_and_deduplicated_per_line() {
        let src = "PostEvent(\"A\"); PostEvent(\"A\");\n";
        let refs = scan("x.cs", "app", src);
        assert_eq!(refs.len(), 1);
        let long = "PostEvent(\"X\");\n".repeat(400);
        assert!(scan("x.cs", "app", &long).len() <= MAX_REFS_PER_FILE);
    }
}
