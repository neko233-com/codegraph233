// Package scanner provides a language-agnostic tokenizer shared by the
// non-C#/Go front-ends.  It keeps identifiers, punctuation and literal contents
// (needed for TS module specifiers and Rust attribute-ish strings).
package scanner

// Kind is the token class.
type Kind uint8

const (
	EOF Kind = iota
	Ident
	Punct
	Number
	String
	Char
)

// Token is one lexical unit.
type Token struct {
	Kind Kind
	Text string
	Line int
	Pos  int
}

// Options toggles language-specific literal forms.
type Options struct {
	NestedComments  bool // Rust: /* /* */ */
	TemplateStrings bool // TS/JS: `...`
	RustLiterals    bool // r"..." / r#"..."# and 'a lifetimes
	HashDirectives  bool // C#: skip #if/#region lines
}

var multiPunct = []string{
	"::", "=>", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "--",
	"+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "??=", "??", "?.",
	"...", "..=", "..", "**",
}

var multiPunctBuckets = func() map[byte][]string {
	m := map[byte][]string{}
	for _, p := range multiPunct {
		m[p[0]] = append(m[p[0]], p)
	}
	return m
}()

func isIdentStart(c byte) bool {
	return c == '_' || c == '$' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c >= 0x80
}

func isIdentPart(c byte) bool {
	return isIdentStart(c) || (c >= '0' && c <= '9')
}

// Tokenize converts source bytes into tokens, dropping comments and preserving
// line numbers.
func Tokenize(src []byte, opt Options) []Token {
	toks := make([]Token, 0, len(src)/4+16)
	line := 1
	i := 0
	n := len(src)
	atLineStart := true

	advance := func(c byte) {
		if c == '\n' {
			line++
			atLineStart = true
		} else if c != '\r' && c != ' ' && c != '\t' {
			atLineStart = false
		}
	}

	for i < n {
		c := src[i]
		if c == '\n' || c == '\r' || c == ' ' || c == '\t' {
			advance(c)
			i++
			continue
		}
		if opt.HashDirectives && c == '#' && atLineStart {
			for i < n && src[i] != '\n' {
				i++
			}
			continue
		}
		if c == '/' && i+1 < n && src[i+1] == '/' {
			for i < n && src[i] != '\n' {
				i++
			}
			continue
		}
		if c == '/' && i+1 < n && src[i+1] == '*' {
			i += 2
			depth := 1
			for i < n && depth > 0 {
				if i+1 < n && src[i] == '*' && src[i+1] == '/' {
					depth--
					i += 2
					continue
				}
				if opt.NestedComments && i+1 < n && src[i] == '/' && src[i+1] == '*' {
					depth++
					i += 2
					continue
				}
				advance(src[i])
				i++
			}
			continue
		}
		// Rust raw strings: r"..." / r#"..."# / br#"..."#
		if opt.RustLiterals && (c == 'r' || c == 'b') {
			j := i
			if src[j] == 'b' {
				j++
			}
			if j < n && src[j] == 'r' {
				k := j + 1
				hashes := 0
				for k < n && src[k] == '#' {
					hashes++
					k++
				}
				if k < n && src[k] == '"' {
					quoteStart := k
					k++
					for k < n {
						if src[k] == '\n' {
							line++
						}
						if src[k] == '"' && (hashes == 0 || matchAt(src, k+1, make([]byte, hashes))) {
							if hashes > 0 && !allHashes(src, k+1, hashes) {
								k++
								continue
							}
							end := k
							k += 1 + hashes
							toks = append(toks, Token{Kind: String, Text: string(src[quoteStart+1 : end]), Line: line, Pos: i})
							break
						}
						k++
					}
					i = k
					continue
				}
			}
		}
		if opt.TemplateStrings && c == '`' {
			quoteStart := i + 1
			i++
			for i < n {
				if src[i] == '\\' && i+1 < n {
					if src[i+1] == '\n' {
						line++
					}
					i += 2
					continue
				}
				if src[i] == '`' {
					content := ""
					if i > quoteStart {
						content = string(src[quoteStart:i])
					}
					toks = append(toks, Token{Kind: String, Text: content, Line: line, Pos: i})
					i++
					break
				}
				if src[i] == '\n' {
					line++
				}
				i++
			}
			continue
		}
		if c == '"' {
			var content string
			line, i, content = skipQuoted(src, i, line)
			toks = append(toks, Token{Kind: String, Text: content, Line: line, Pos: i})
			continue
		}
		// Rust lifetimes ('a) vs char literals ('a')
		if c == '\'' {
			if opt.RustLiterals && i+1 < n && isIdentStart(src[i+1]) {
				j := i + 1
				for j < n && isIdentPart(src[j]) {
					j++
				}
				if j >= n || src[j] != '\'' {
					toks = append(toks, Token{Kind: Ident, Text: string(src[i:j]), Line: line, Pos: i})
					i = j
					continue
				}
			}
			var content string
			line, i, content = skipQuoted(src, i, line)
			toks = append(toks, Token{Kind: Char, Text: content, Line: line, Pos: i})
			continue
		}
		if isIdentStart(c) {
			start := i
			for i < n && isIdentPart(src[i]) {
				i++
			}
			toks = append(toks, Token{Kind: Ident, Text: string(src[start:i]), Line: line, Pos: start})
			atLineStart = false
			continue
		}
		if c >= '0' && c <= '9' {
			start := i
			for i < n && (isIdentPart(src[i]) || src[i] == '.' || src[i] == '\'' || src[i] == 'x' || src[i] == 'X') {
				i++
			}
			toks = append(toks, Token{Kind: Number, Text: string(src[start:i]), Line: line, Pos: start})
			atLineStart = false
			continue
		}
		matched := ""
		for _, mp := range multiPunctBuckets[c] {
			if i+len(mp) <= n && string(src[i:i+len(mp)]) == mp {
				matched = mp
				break
			}
		}
		if matched != "" {
			toks = append(toks, Token{Kind: Punct, Text: matched, Line: line, Pos: i})
			i += len(matched)
			atLineStart = false
			continue
		}
		toks = append(toks, Token{Kind: Punct, Text: string(c), Line: line, Pos: i})
		i++
		atLineStart = false
	}
	toks = append(toks, Token{Kind: EOF, Line: line, Pos: n})
	return toks
}

func matchAt(src []byte, at int, pat []byte) bool {
	if at+len(pat) > len(src) {
		return false
	}
	for k := range pat {
		if src[at+k] != pat[k] {
			return false
		}
	}
	return true
}

func allHashes(src []byte, at, count int) bool {
	for k := 0; k < count; k++ {
		if at+k >= len(src) || src[at+k] != '#' {
			return false
		}
	}
	return true
}

// skipQuoted returns the new line, next index and the literal contents.
func skipQuoted(src []byte, start int, line int) (int, int, string) {
	n := len(src)
	i := start + 1
	quote := src[start]
	for i < n {
		c := src[i]
		if c == '\n' {
			line++
			return line, i + 1, string(src[start+1 : i])
		}
		if c == '\\' && i+1 < n {
			if src[i+1] == '\n' {
				line++
			}
			i += 2
			continue
		}
		if c == quote {
			return line, i + 1, string(src[start+1 : i])
		}
		i++
	}
	return line, n, ""
}
