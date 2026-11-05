package csharp

import "strings"

type tokKind uint8

const (
	tokEOF tokKind = iota
	tokIdent
	tokPunct
	tokNumber
	tokString
	tokChar
)

type token struct {
	kind tokKind
	text string
	line int
	pos  int
}

var multiPunct = []string{
	"=>", "::", "==", "!=", "<=", ">=", "&&", "||", "++", "--",
	"+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "??=", "??", "?.",
	"->", "...",
}

var multiPunctBuckets = func() map[byte][]string {
	m := map[byte][]string{}
	for _, p := range multiPunct {
		m[p[0]] = append(m[p[0]], p)
	}
	return m
}()

func isIdentStart(c byte) bool {
	return c == '_' || c == '@' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c >= 0x80
}

func isIdentPart(c byte) bool {
	return isIdentStart(c) || (c >= '0' && c <= '9')
}

// tokenize converts C# source into a token stream, dropping comments, preprocessor
// directives and string/char contents (their line span is preserved).
func tokenize(src []byte) []token {
	toks := make([]token, 0, len(src)/4+16)
	line := 1
	i := 0
	n := len(src)
	atLineStart := true

	advanceLine := func(b byte) {
		if b == '\n' {
			line++
			atLineStart = true
		} else if b != '\r' && b != ' ' && b != '\t' {
			atLineStart = false
		}
	}

	for i < n {
		c := src[i]
		if c == '\n' || c == '\r' || c == ' ' || c == '\t' {
			advanceLine(c)
			i++
			continue
		}
		// preprocessor directive: skip to end of line
		if c == '#' && atLineStart {
			for i < n && src[i] != '\n' {
				i++
			}
			continue
		}
		// comments
		if c == '/' && i+1 < n && src[i+1] == '/' {
			for i < n && src[i] != '\n' {
				i++
			}
			continue
		}
		if c == '/' && i+1 < n && src[i+1] == '*' {
			i += 2
			for i+1 < n && !(src[i] == '*' && src[i+1] == '/') {
				advanceLine(src[i])
				i++
			}
			i += 2
			if i > n {
				i = n
			}
			continue
		}
		// verbatim / interpolated strings
		if (c == '@' || c == '$') && i+1 < n && (src[i+1] == '"' || src[i+1] == '@' || src[i+1] == '$') {
			j := i
			for j < n && (src[j] == '@' || src[j] == '$') {
				j++
			}
			if j < n && src[j] == '"' {
				verbatim := strings.ContainsRune(string(src[i:j]), '@')
				line, i = skipQuoted(src, j, line, verbatim)
				continue
			}
		}
		if c == '"' {
			line, i = skipQuoted(src, i, line, false)
			continue
		}
		if c == '\'' {
			line, i = skipQuoted(src, i, line, false)
			continue
		}
		if isIdentStart(c) {
			start := i
			for i < n && isIdentPart(src[i]) {
				i++
			}
			toks = append(toks, token{kind: tokIdent, text: string(src[start:i]), line: line, pos: start})
			atLineStart = false
			continue
		}
		if c >= '0' && c <= '9' {
			start := i
			for i < n && (isIdentPart(src[i]) || src[i] == '.' || src[i] == 'x' || src[i] == 'X') {
				i++
			}
			toks = append(toks, token{kind: tokNumber, text: string(src[start:i]), line: line, pos: start})
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
			toks = append(toks, token{kind: tokPunct, text: matched, line: line, pos: i})
			i += len(matched)
			atLineStart = false
			continue
		}
		toks = append(toks, token{kind: tokPunct, text: string(c), line: line, pos: i})
		i++
		atLineStart = false
	}
	toks = append(toks, token{kind: tokEOF, text: "", line: line, pos: n})
	return toks
}

func skipQuoted(src []byte, start int, line int, verbatim bool) (int, int) {
	n := len(src)
	i := start + 1
	if i > n {
		return n, n
	}
	for i < n {
		c := src[i]
		if c == '\n' {
			line++
			if !verbatim {
				return line, i + 1
			}
			i++
			continue
		}
		if verbatim {
			if c == '"' && i+1 < n && src[i+1] == '"' {
				i += 2
				continue
			}
			if c == '"' {
				return line, i + 1
			}
		} else {
			if c == '\\' && i+1 < n {
				if src[i+1] == '\n' {
					line++
				}
				i += 2
				continue
			}
			if c == '"' || c == '\'' {
				return line, i + 1
			}
		}
		i++
	}
	return line, n
}
