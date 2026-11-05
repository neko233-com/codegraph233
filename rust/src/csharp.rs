// Minimal C# scanner used only for the Go-vs-Rust front-end benchmark.
// It tokenizes and counts declarations / references with the same granularity
// as the Go front-end's non-type-deps mode.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ident,
    Punct,
    Number,
    Str,
}

pub struct Tok {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

pub struct Counts {
    pub symbols: usize,
    pub refs: usize,
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c == b'@' || c.is_ascii_alphabetic() || c >= 0x80
}

fn is_ident_part(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

const MULTI: &[&[u8]] = &[
    b"=>", b"::", b"==", b"!=", b"<=", b">=", b"&&", b"||", b"++", b"--",
    b"+=", b"-=", b"*=", b"/=", b"%=", b"&=", b"|=", b"^=", b"??=", b"??", b"?.",
    b"->", b"...",
];

pub fn tokenize(src: &[u8]) -> Vec<Tok> {
    let n = src.len();
    let mut out = Vec::with_capacity(n / 4 + 16);
    let mut i = 0usize;
    let mut line_start = true;
    while i < n {
        let c = src[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\r' || c == b'\t' {
            i += 1;
            continue;
        }
        // preprocessor line
        if c == b'#' && line_start {
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        line_start = false;
        // comments
        if c == b'/' && i + 1 < n && src[i + 1] == b'/' {
            while i < n && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < n && src[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(src[i] == b'*' && src[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // strings (verbatim / interpolated / normal)
        if c == b'"' || ((c == b'@' || c == b'$') && i + 1 < n && matches!(src[i + 1], b'"' | b'@' | b'$')) {
            let mut j = i;
            let mut verbatim = false;
            while j < n && (src[j] == b'@' || src[j] == b'$') {
                verbatim |= src[j] == b'@';
                j += 1;
            }
            if j < n && src[j] == b'"' {
                j += 1;
                while j < n {
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
        if c == b'\'' {
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
            out.push(Tok { kind: Kind::Ident, start, end: i });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < n && (is_ident_part(src[i]) || src[i] == b'.') {
                i += 1;
            }
            out.push(Tok { kind: Kind::Number, start, end: i });
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
            out.push(Tok { kind: Kind::Punct, start: i, end: i + matched });
            i += matched;
            continue;
        }
        out.push(Tok { kind: Kind::Punct, start: i, end: i + 1 });
        i += 1;
    }
    out
}

fn is_type_kw(s: &str) -> bool {
    matches!(s, "class" | "struct" | "interface" | "enum" | "record" | "delegate")
}

fn is_control(s: &str) -> bool {
    matches!(
        s,
        "if" | "for" | "foreach" | "while" | "switch" | "catch" | "using" | "return"
            | "new" | "sizeof" | "typeof" | "nameof" | "default" | "checked" | "unchecked"
            | "lock" | "fixed" | "await" | "throw" | "when" | "case" | "yield" | "delegate"
            | "stackalloc" | "get" | "set" | "init" | "add" | "remove"
    )
}

pub fn scan(src: &[u8]) -> Counts {
    let toks = tokenize(src);
    let mut symbols = 0usize;
    let mut refs = 0usize;
    let text = |t: &Tok| -> &str { std::str::from_utf8(&src[t.start..t.end]).unwrap_or("") };

    for idx in 0..toks.len() {
        let t = &toks[idx];
        if t.kind != Kind::Ident {
            continue;
        }
        let s = text(t);

        // type declarations
        if is_type_kw(s) {
            if idx + 1 < toks.len() && toks[idx + 1].kind == Kind::Ident {
                symbols += 1;
            }
            continue;
        }
        if is_control(s) {
            continue;
        }

        // method: name '(' (not a member access, not a control keyword)
        if idx + 1 < toks.len()
            && toks[idx + 1].kind == Kind::Punct
            && text(&toks[idx + 1]) == "("
            && !(idx > 0 && toks[idx - 1].kind == Kind::Punct && text(&toks[idx - 1]) == ".")
            && !(idx > 0 && toks[idx - 1].kind == Kind::Ident && text(&toks[idx - 1]) == "new")
        {
            // only when the previous token is a type-ish token (return type)
            if idx > 0 {
                let prev = &toks[idx - 1];
                let prev_ok = prev.kind == Kind::Ident
                    || (prev.kind == Kind::Punct && matches!(text(prev), ">" | "]" | "?"));
                if prev_ok {
                    symbols += 1;
                    refs += 1;
                    continue;
                }
            }
        }

        // construct: new Type(
        if idx > 0 && text(&toks[idx - 1]) == "new" {
            refs += 1;
            continue;
        }

        // member access: a.b
        if idx + 1 < toks.len()
            && toks[idx + 1].kind == Kind::Punct
            && matches!(text(&toks[idx + 1]), "." | "?.")
            && idx + 2 < toks.len()
            && toks[idx + 2].kind == Kind::Ident
        {
            refs += 1;
            continue;
        }
    }
    Counts { symbols, refs }
}
