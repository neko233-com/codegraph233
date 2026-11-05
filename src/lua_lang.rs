//! Lua front-end: module-scoped functions, `a.b.c` / `a:b` methods, locals,
//! table constructors, calls and dotted member references.
//!
//! Lua has no type system, so member resolution relies on the binder's
//! name/namespace heuristics; what this parser contributes is an accurate
//! declaration tree plus call-site names and arity.

use crate::lexer::{last_segment, Cursor, Kind, Options};
use crate::model::{symbol_key, FileParse, Ref, Symbol};
use std::collections::HashSet;

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
    "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    "self", "continue",
];

pub fn parse(rel: &str, root: &str, src: &[u8]) -> FileParse {
    let mut p = Parser {
        c: Cursor::new(
            src,
            Options {
                dash_comments: true,
                long_brackets: true,
                ..Default::default()
            },
        ),
        res: FileParse {
            rel_path: rel.to_string(),
            root: root.to_string(),
            lang: "lua".into(),
            module: module_of(rel),
            ..Default::default()
        },
        seen: HashSet::new(),
    };
    p.run();
    p.res
}

fn module_of(rel: &str) -> String {
    let mut r = rel.to_string();
    if let Some(s) = r.strip_suffix(".lua") {
        r = s.to_string();
    }
    r.replace('\\', "/").replace('/', ".")
}

struct Parser<'a> {
    c: Cursor<'a>,
    res: FileParse,
    seen: HashSet<String>,
}

impl<'a> Parser<'a> {
    fn add_symbol(
        &mut self,
        kind: &str,
        name: &str,
        qual: &str,
        line: i32,
        end: i32,
        sig: String,
        params: i32,
        container: &str,
    ) {
        self.res.symbols.push(Symbol {
            key: symbol_key(qual, kind, line),
            lang: "lua".into(),
            kind: kind.to_string(),
            name: name.to_string(),
            qualname: qual.to_string(),
            namespace: self.res.module.clone(),
            start_line: line,
            end_line: end,
            signature: sig,
            modifiers: String::new(),
            param_count: params,
            container: container.to_string(),
            module: self.res.module.clone(),
        });
    }

    fn add_ref(&mut self, src_key: &str, name: &str, kind: &str, line: i32, arity: i32) {
        if name.is_empty() {
            return;
        }
        let dedup = format!("{src_key}|{name}|{kind}|{arity}");
        if !self.seen.insert(dedup) {
            return;
        }
        self.res.refs.push(Ref {
            src_key: src_key.to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            receiver_type: String::new(),
            namespace_hint: self.res.module.clone(),
            line,
            arity,
        });
    }

    fn run(&mut self) {
        let n = self.c.len();
        let mut i = 0usize;
        while i < n {
            if !self.c.is_ident(i) {
                i += 1;
                continue;
            }
            match self.c.text(i) {
                "function" => i = self.parse_function(i),
                "local" => i = self.parse_local(i),
                _ => i += 1,
            }
        }
    }

    /// `function Name` / `function a.b.c` / `function a:b`; `i` is the
    /// `function` token.
    fn parse_function(&mut self, i: usize) -> usize {
        let start = i;
        let mut j = start + 1;
        if !self.c.is_ident(j) {
            return j;
        }
        let mut parts: Vec<String> = vec![self.c.text(j).to_string()];
        let mut method = false;
        while self.c.is_punct(j + 1, ".") || self.c.is_punct(j + 1, ":") {
            method = self.c.is_punct(j + 1, ":");
            if !self.c.is_ident(j + 2) {
                break;
            }
            parts.push(self.c.text(j + 2).to_string());
            j += 2;
        }
        let name = parts.last().cloned().unwrap_or_default();
        let owner = if parts.len() > 1 {
            format!("{}.{}", self.res.module, parts[..parts.len() - 1].join("."))
        } else {
            String::new()
        };
        let qual = format!("{}.{}", self.res.module, parts.join("."));
        let line = self.c.line(start + 1);
        let k = j + 1;
        if !self.c.is_punct(k, "(") {
            return k;
        }
        let pc = self.c.match_close(k);
        let (end, close) = self.block_end(pc + 1);
        let kind = if method { "method" } else { "function" };
        let sig = self.c.raw_text(start, k.saturating_sub(1).max(start));
        self.add_symbol(
            kind,
            &name,
            &qual,
            line,
            end,
            sig,
            count_params(&self.c, k, pc),
            &owner,
        );
        let key = symbol_key(&qual, kind, line);
        self.scan_refs(pc + 1, close, &key, &owner);
        close + 1
    }

    fn parse_local(&mut self, i: usize) -> usize {
        let mut j = i + 1;
        if self.c.is_ident(j) && self.c.text(j) == "function" {
            return self.parse_function(j);
        }
        while j < self.c.len() && !self.c.is_ident(j) {
            j += 1;
        }
        if j >= self.c.len() {
            return j;
        }
        let name = self.c.text(j).to_string();
        let line = self.c.line(j);
        let k = j + 1;
        if self.c.is_punct(k, "=") {
            let x = k + 1;
            if self.c.is_ident(x) && self.c.text(x) == "function" {
                if self.c.is_punct(x + 1, "(") {
                    let pc = self.c.match_close(x + 1);
                    let (end, close) = self.block_end(pc + 1);
                    let qual = format!("{}.{}", self.res.module, name);
                    self.add_symbol(
                        "function",
                        &name,
                        &qual,
                        line,
                        end,
                        format!("local {name} = function"),
                        count_params(&self.c, x + 1, pc),
                        "",
                    );
                    let key = symbol_key(&qual, "function", line);
                    self.scan_refs(pc + 1, close, &key, "");
                    return close + 1;
                }
            }
            let qual = format!("{}.{}", self.res.module, name);
            let kind = if self.c.is_punct(x, "{") {
                "table"
            } else {
                "field"
            };
            // A table constructor spans braces; anything else ends with the line.
            let mut stop = if self.c.is_punct(x, "{") {
                self.c.match_close(x) + 1
            } else {
                let mut s = x;
                while s < self.c.len() && self.c.line(s) == line {
                    s += 1;
                }
                s
            };
            if stop > self.c.len() {
                stop = self.c.len();
            }
            self.add_symbol(kind, &name, &qual, line, line, self.c.raw_text(i, stop.min(x + 24)), -1, "");
            let key = symbol_key(&qual, kind, line);
            self.scan_refs(x, stop, &key, "");
            return stop;
        }
        k
    }

    /// Find the `end` that closes a block opened before `from`.
    fn block_end(&self, from: usize) -> (i32, usize) {
        let n = self.c.len();
        let mut depth = 1i32;
        let mut await_do = false;
        let mut j = from;
        while j < n {
            if self.c.is_ident(j) {
                let t = self.c.text(j);
                match t {
                    "function" | "if" => depth += 1,
                    "for" | "while" => {
                        depth += 1;
                        await_do = true;
                    }
                    "do" => {
                        if await_do {
                            await_do = false;
                        } else {
                            depth += 1;
                        }
                    }
                    "repeat" => depth += 1,
                    "until" => depth -= 1,
                    "end" => {
                        depth -= 1;
                        if depth == 0 {
                            return (self.c.line(j), j);
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        (self.c.line(n.saturating_sub(1)), n.saturating_sub(1))
    }

    fn scan_refs(&mut self, from: usize, to: usize, src_key: &str, owner: &str) {
        let mut j = from;
        while j < to {
            if !self.c.is_ident(j) {
                j += 1;
                continue;
            }
            let word = self.c.text(j).to_string();
            if word == "function" || word == "local" {
                j += 1;
                continue;
            }
            if word == "require" {
                // require("module") / require "module"
                let mut m = j + 1;
                if self.c.is_punct(m, "(") {
                    m += 1;
                }
                if self.c.is_ident(m) && self.c.text(m) == "self" {
                    m += 1;
                }
                let mut stop = m;
                while stop < to && !self.c.is_punct(stop, ")") && !self.c.is_punct(stop, ";") {
                    stop += 1;
                }
                let module = self.c.raw_text(m, stop).trim().to_string();
                if !module.is_empty() {
                    let seg = last_segment(&module.trim_matches(|c| c == '"' || c == '\''));
                    self.add_ref(src_key, &seg, "references", self.c.line(j), -1);
                }
                j += 1;
                continue;
            }
            if KEYWORDS.contains(&word.as_str()) {
                j += 1;
                continue;
            }
            if self.c.is_punct(j + 1, "(") {
                let arity = count_params(&self.c, j + 1, self.c.match_close(j + 1));
                self.add_ref(src_key, &word, "calls", self.c.line(j), arity);
                j += 1;
                continue;
            }
            if (self.c.is_punct(j + 1, ".") || self.c.is_punct(j + 1, ":"))
                && self.c.is_ident(j + 2)
            {
                let member = self.c.text(j + 2).to_string();
                let kind = if self.c.is_punct(j + 3, "(") {
                    "calls"
                } else {
                    "references"
                };
                let arity = if kind == "calls" {
                    count_params(&self.c, j + 3, self.c.match_close(j + 3))
                } else {
                    -1
                };
                let _ = owner;
                self.add_ref(src_key, &member, kind, self.c.line(j), arity);
                j += 2;
                continue;
            }
            j += 1;
        }
    }
}

fn count_params(c: &Cursor, open: usize, close: usize) -> i32 {
    if close <= open + 1 {
        return 0;
    }
    let mut depth = 0i32;
    let mut count = 1i32;
    let mut has = false;
    for j in open + 1..close {
        if c.toks[j].kind == Kind::Ident {
            has = true;
        }
        if c.toks[j].kind != Kind::Punct {
            continue;
        }
        match c.text(j) {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "," if depth == 0 => count += 1,
            _ => {}
        }
    }
    if has {
        count
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_src(src: &str) -> FileParse {
        parse("Assets/Lua/Game/Bag.lua", "client", src.as_bytes())
    }

    #[test]
    fn declares_module_functions_and_methods() {
        let src = r#"
--[[ block comment
     with a function nope() end ]]
local M = {}

function M.Open(id)
    return id
end

function M:Refresh()
    self:Load()
end

local function helper(a, b)
    return a + b
end

return M
"#;
        let res = parse_src(src);
        let names: Vec<&str> = res.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Open"));
        assert!(names.contains(&"Refresh"));
        assert!(names.contains(&"helper"));
        let refresh = res.symbols.iter().find(|s| s.name == "Refresh").unwrap();
        assert_eq!(refresh.container, "Assets.Lua.Game.Bag.M");
        assert_eq!(refresh.qualname, "Assets.Lua.Game.Bag.M.Refresh");
        let open = res.symbols.iter().find(|s| s.name == "Open").unwrap();
        assert_eq!(open.param_count, 1);
    }

    #[test]
    fn records_calls_and_skips_comments() {
        let src = r#"
-- comment: Fake(1,2)
local function run()
    print("hi")
    local cfg = require "Game.Config"
    cfg:Reload()
end
"#;
        let res = parse_src(src);
        let calls: Vec<&str> = res
            .refs
            .iter()
            .filter(|r| r.kind == "calls")
            .map(|r| r.name.as_str())
            .collect();
        assert!(calls.contains(&"print"), "calls={calls:?}");
        assert!(calls.contains(&"Reload"), "calls={calls:?}");
        assert!(!calls.iter().any(|c| *c == "Fake"), "commented call leaked");
    }

    #[test]
    fn nested_blocks_close_the_right_function() {
        let src = r#"
function M.Calc(v)
    for i = 1, 10 do
        if v > i then
            do
                v = v - 1
            end
        else
            v = v + 1
        end
    end
    return v
end
"#;
        let res = parse_src(src);
        let calc = res.symbols.iter().find(|s| s.name == "Calc").unwrap();
        assert_eq!(calc.end_line, 13, "end_line should be the final `end`");
    }
}
