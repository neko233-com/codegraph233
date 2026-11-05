//! Shared tokenizer for the non-C# front-ends (and C# itself): comments,
//! strings, raw/template literals, preprocessor lines are skipped; identifiers,
//! punctuation and numbers are emitted with byte ranges + line numbers.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Punct,
    Number,
}

#[derive(Clone, Copy)]
pub struct Tok {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub line: i32,
}

#[derive(Clone, Copy, Default)]
pub struct Options {
    pub nested_comments: bool,
    pub template_strings: bool,
    pub rust_literals: bool,
    pub hash_directives: bool,
    pub backtick_strings: bool,
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'@' || c == b'$' || c.is_ascii_alphabetic() || c >= 0x80
}
fn is_ident_part(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

const MULTI: &[&[u8]] = &[
    b"=>", b"::", b"==", b"!=", b"<=", b">=", b"&&", b"||", b"++", b"--", b"+=", b"-=",
    b"*=", b"/=", b"%=", b"&=", b"|=", b"^=", b"??=", b"??", b"?.", b"->", b"...", b"..=",
    b"..", b"**",
];

pub fn tokenize(src: &[u8], opt: Options) -> Vec<Tok> {
    let n = src.len();
    let mut out = Vec::with_capacity(n / 4 + 16);
    let mut i = 0usize;
    let mut line = 1i32;
    let mut line_start = true;

    while i < n {
        let c = src[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
                line_start = true;
                continue;
            }
            b' ' | b'\r' | b'\t' => {
                i += 1;
                continue;
            }
            _ => {}
        }
        if opt.hash_directives && c == b'#' && line_start {
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        line_start = false;
        if c == b'/' && i + 1 < n && src[i + 1] == b'/' {
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < n && src[i + 1] == b'*' {
            i += 2;
            let mut depth = 1i32;
            while i < n && depth > 0 {
                if i + 1 < n && src[i] == b'*' && src[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                    continue;
                }
                if opt.nested_comments && i + 1 < n && src[i] == b'/' && src[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if src[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            continue;
        }
        // Rust raw strings r"..." / r#"..."# / br#"..."#
        if opt.rust_literals && (c == b'r' || c == b'b') {
            let mut j = i;
            if src[j] == b'b' {
                j += 1;
            }
            if j < n && src[j] == b'r' {
                let mut k = j + 1;
                let mut hashes = 0usize;
                while k < n && src[k] == b'#' {
                    hashes += 1;
                    k += 1;
                }
                if k < n && src[k] == b'"' {
                    k += 1;
                    while k < n {
                        if src[k] == b'\n' {
                            line += 1;
                        }
                        if src[k] == b'"'
                            && (hashes == 0
                                || (k + hashes < n
                                    && src[k + 1..k + 1 + hashes].iter().all(|b| *b == b'#')))
                        {
                            k += 1 + hashes;
                            break;
                        }
                        k += 1;
                    }
                    i = k;
                    continue;
                }
            }
        }
        if (opt.template_strings || opt.backtick_strings) && c == b'`' {
            i += 1;
            while i < n {
                if src[i] == b'\\' && i + 1 < n {
                    if src[i + 1] == b'\n' {
                        line += 1;
                    }
                    i += 2;
                    continue;
                }
                if src[i] == b'\n' {
                    line += 1;
                }
                if src[i] == b'`' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == b'"'
            || ((c == b'@' || c == b'$') && i + 1 < n && matches!(src[i + 1], b'"' | b'@' | b'$'))
        {
            let mut j = i;
            let mut verbatim = false;
            while j < n && (src[j] == b'@' || src[j] == b'$') {
                verbatim |= src[j] == b'@';
                j += 1;
            }
            if j < n && src[j] == b'"' {
                j += 1;
                while j < n {
                    if src[j] == b'\n' {
                        line += 1;
                    }
                    if verbatim && src[j] == b'"' && j + 1 < n && src[j + 1] == b'"' {
                        j += 2;
                        continue;
                    }
                    if !verbatim && src[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if src[j] == b'"' {
                        j += 1;
                        break;
                    }
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        // Rust lifetimes ('a) vs char literals
        if c == b'\'' {
            if opt.rust_literals && i + 1 < n && is_ident_start(src[i + 1]) {
                let mut j = i + 1;
                while j < n && is_ident_part(src[j]) {
                    j += 1;
                }
                if j >= n || src[j] != b'\'' {
                    out.push(Tok { kind: Kind::Ident, start: i, end: j, line });
                    i = j;
                    continue;
                }
            }
            i += 1;
            while i < n && src[i] != b'\'' && src[i] != b'\n' {
                if src[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(n);
            continue;
        }
        if is_ident_start(c) {
            let start = i;
            while i < n && is_ident_part(src[i]) {
                i += 1;
            }
            out.push(Tok { kind: Kind::Ident, start, end: i, line });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < n && (is_ident_part(src[i]) || src[i] == b'.' || src[i] == b'\'') {
                i += 1;
            }
            out.push(Tok { kind: Kind::Number, start, end: i, line });
            continue;
        }
        let mut matched = 0usize;
        for mp in MULTI {
            if i + mp.len() <= n && &src[i..i + mp.len()] == *mp {
                matched = mp.len();
                break;
            }
        }
        if matched > 0 {
            out.push(Tok { kind: Kind::Punct, start: i, end: i + matched, line });
            i += matched;
            continue;
        }
        out.push(Tok { kind: Kind::Punct, start: i, end: i + 1, line });
        i += 1;
    }
    out
}

pub struct Cursor<'a> {
    pub src: &'a [u8],
    pub toks: Vec<Tok>,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a [u8], opt: Options) -> Cursor<'a> {
        Cursor {
            src,
            toks: tokenize(src, opt),
        }
    }
    pub fn len(&self) -> usize {
        self.toks.len()
    }
    pub fn text(&self, i: usize) -> &str {
        if i >= self.toks.len() {
            return "";
        }
        let t = self.toks[i];
        std::str::from_utf8(&self.src[t.start..t.end]).unwrap_or("")
    }
    pub fn is_punct(&self, i: usize, s: &str) -> bool {
        i < self.toks.len() && self.toks[i].kind == Kind::Punct && self.text(i) == s
    }
    pub fn is_ident(&self, i: usize) -> bool {
        i < self.toks.len() && self.toks[i].kind == Kind::Ident
    }
    pub fn line(&self, i: usize) -> i32 {
        if i < self.toks.len() {
            self.toks[i].line
        } else {
            0
        }
    }
    pub fn match_close(&self, open: usize) -> usize {
        let mut depth = 0i32;
        for j in open..self.toks.len() {
            if self.toks[j].kind != Kind::Punct {
                continue;
            }
            match self.text(j) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return j;
                    }
                }
                _ => {}
            }
        }
        self.toks.len().saturating_sub(1)
    }
    pub fn match_angle(&self, open: usize, end: usize) -> Option<usize> {
        let mut depth = 0i32;
        for j in open..end {
            if self.toks[j].kind != Kind::Punct {
                continue;
            }
            match self.text(j) {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(j);
                    }
                }
                _ => {}
            }
        }
        None
    }
    pub fn raw_text(&self, from: usize, to: usize) -> String {
        let mut out = String::new();
        let mut prev_ident = false;
        for j in from..to.min(self.toks.len()) {
            let t = self.toks[j];
            if prev_ident && t.kind == Kind::Ident {
                out.push(' ');
            }
            out.push_str(self.text(j));
            prev_ident = t.kind == Kind::Ident;
        }
        out
    }
}

pub fn last_segment(s: &str) -> String {
    let s = s.trim();
    let s = s.rsplit("::").next().unwrap_or(s);
    let s = s.rsplit('.').next().unwrap_or(s);
    let s = match s.find('<') {
        Some(i) => &s[..i],
        None => s,
    };
    s.trim().to_string()
}
