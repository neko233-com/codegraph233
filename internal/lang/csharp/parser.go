// Package csharp implements a tolerant, dependency-free C# structure and
// reference extractor.  It is a declaration scanner rather than a full parser:
// game codebases are regular, and scanning keeps indexing fast enough for ~20k
// files while still producing the type/member/call graph an agent needs.
package csharp

import (
	"strings"

	"github.com/neko233-com/codegraph233/internal/model"
)

type parser struct {
	toks    []token
	res     *model.FileResult
	ns      string
	usings  []string
	types   map[string]string // qualname -> kind
	members map[string]string // "qual|member" -> declared type text
}

var typeKeywords = map[string]string{
	"class":     model.KindClass,
	"struct":    model.KindStruct,
	"interface": model.KindInterface,
	"enum":      model.KindEnum,
	"record":    model.KindRecord,
	"delegate":  model.KindDelegate,
}

var modifiers = map[string]bool{
	"public": true, "private": true, "protected": true, "internal": true,
	"static": true, "readonly": true, "const": true, "virtual": true,
	"override": true, "abstract": true, "async": true, "partial": true,
	"extern": true, "unsafe": true, "new": true, "sealed": true,
	"volatile": true, "required": true, "event": true, "file": true,
}

var notCalls = map[string]bool{
	"if": true, "for": true, "foreach": true, "while": true, "switch": true,
	"catch": true, "using": true, "return": true, "new": true, "sizeof": true,
	"typeof": true, "nameof": true, "default": true, "checked": true,
	"unchecked": true, "lock": true, "fixed": true, "await": true, "throw": true,
	"when": true, "case": true, "yield": true, "delegate": true, "stackalloc": true,
}

// Parse extracts declarations and unresolved references from one C# file.
func Parse(relPath, root string, src []byte) *model.FileResult {
	p := &parser{
		toks:    tokenize(src),
		res:     &model.FileResult{RelPath: relPath, Root: root, Lang: "csharp"},
		types:   map[string]string{},
		members: map[string]string{},
	}
	p.parseBlock(0, len(p.toks)-1, "", "")
	return p.res
}

func (p *parser) tok(idx int) token {
	if idx < 0 || idx >= len(p.toks) {
		return token{kind: tokEOF}
	}
	return p.toks[idx]
}

func (p *parser) text(idx int) string { return p.tok(idx).text }

func (p *parser) isPunct(idx int, s string) bool {
	t := p.tok(idx)
	return t.kind == tokPunct && t.text == s
}

func (p *parser) isIdent(idx int) bool { return p.tok(idx).kind == tokIdent }

func (p *parser) isMod(idx int) bool {
	return p.tok(idx).kind == tokIdent && modifiers[p.text(idx)]
}

func (p *parser) newKey(qual, kind string, line int) string {
	return qual + "#" + kind + "@" + itoa(line)
}

func itoa(v int) string {
	if v == 0 {
		return "0"
	}
	neg := v < 0
	if neg {
		v = -v
	}
	var b [20]byte
	i := len(b)
	for v > 0 {
		i--
		b[i] = byte('0' + v%10)
		v /= 10
	}
	if neg {
		i--
		b[i] = '-'
	}
	return string(b[i:])
}

// matchClose returns the index of the token closing the bracket opened at open.
func (p *parser) matchClose(open int) int {
	depth := 0
	for j := open; j < len(p.toks); j++ {
		t := p.toks[j]
		if t.kind != tokPunct {
			continue
		}
		switch t.text {
		case "(", "{", "[":
			depth++
		case ")", "}", "]":
			depth--
			if depth == 0 {
				return j
			}
		}
	}
	return len(p.toks) - 1
}

func (p *parser) matchAngle(open, end int) int {
	depth := 0
	for j := open; j < end; j++ {
		t := p.toks[j]
		if t.kind != tokPunct {
			continue
		}
		switch t.text {
		case "<":
			depth++
		case ">":
			depth--
			if depth == 0 {
				return j
			}
		}
	}
	return -1
}

// parseBlock reads declarations in [start, end).
func (p *parser) parseBlock(start, end int, ns, parentQual string) {
	i := start
	for i < end {
		t := p.toks[i]
		if t.kind == tokEOF {
			return
		}
		if t.kind == tokPunct {
			switch t.text {
			case "}":
				return
			case "{", "(":
				i = p.matchClose(i) + 1
				continue
			case ";", ")", "]", ",":
				i++
				continue
			}
			i++
			continue
		}
		if t.kind != tokIdent {
			i++
			continue
		}

		switch t.text {
		case "using":
			i = p.parseUsing(i, end)
			continue
		case "namespace":
			i = p.parseNamespace(i, end, parentQual)
			continue
		}

		if kw, ok := typeKeywords[t.text]; ok && p.looksLikeTypeDecl(i) {
			i = p.parseTypeDecl(i, end, ns, parentQual, kw)
			continue
		}

		if parentQual != "" {
			consumed := p.parseMember(i, end, ns, parentQual)
			if consumed > i {
				i = consumed
				continue
			}
		}
		i++
	}
}

func (p *parser) looksLikeTypeDecl(i int) bool {
	if !p.isIdent(i+1) {
		return false
	}
	j := i + 2
	if p.isPunct(j, "<") {
		if gt := p.matchAngle(j, len(p.toks)-1); gt > 0 {
			j = gt + 1
		}
	}
	next := p.tok(j)
	if next.kind == tokPunct && (next.text == "{" || next.text == ":" || next.text == ";") {
		return true
	}
	return next.kind == tokEOF
}

func (p *parser) parseUsing(i, end int) int {
	j := i + 1
	if p.isPunct(j, "(") {
		return j
	}
	var parts []string
	for j < end {
		t := p.toks[j]
		if t.kind == tokPunct && t.text == ";" {
			break
		}
		if t.kind == tokIdent || (t.kind == tokPunct && (t.text == "." || t.text == "::")) || p.isPunct(j, "=") {
			parts = append(parts, t.text)
		}
		j++
	}
	if len(parts) > 0 {
		p.usings = append(p.usings, strings.Join(parts, ""))
	}
	if j < end {
		return j + 1
	}
	return j
}

func (p *parser) parseNamespace(i, end int, parentQual string) int {
	j := i + 1
	var parts []string
	for j < end {
		t := p.toks[j]
		if t.kind == tokPunct && (t.text == "{" || t.text == ";") {
			break
		}
		if t.kind == tokIdent || (t.kind == tokPunct && (t.text == "." || t.text == "::")) {
			parts = append(parts, t.text)
		}
		j++
	}
	ns := strings.Join(parts, "")
	if ns == "" {
		ns = p.ns
	}
	if p.isPunct(j, ";") {
		p.ns = ns
		return j + 1
	}
	if p.isPunct(j, "{") {
		closeIdx := p.matchClose(j)
		old := p.ns
		p.ns = ns
		p.parseBlock(j+1, closeIdx, ns, parentQual)
		p.ns = old
		return closeIdx + 1
	}
	return j
}

func (p *parser) qualOf(parentQual, ns, name string) string {
	if parentQual != "" {
		return parentQual + "." + name
	}
	if ns != "" {
		return ns + "." + name
	}
	return name
}

func (p *parser) parseTypeDecl(i, end int, ns, parentQual, kind string) int {
	startLine := p.tok(i).line
	nameIdx := i + 1
	name := p.text(nameIdx)
	qual := p.qualOf(parentQual, ns, name)
	next := nameIdx + 1
	if p.isPunct(next, "<") {
		if gt := p.matchAngle(next, end); gt > 0 {
			next = gt + 1
		}
	}

	// base list: skip nested generic args; stop at '{', ';' or 'where'
	var bases []string
	if p.isPunct(next, ":") {
		var cur []string
		paren := 0
		angle := 0
		j := next + 1
	loop:
		for j < end {
			t := p.toks[j]
			if t.kind == tokIdent && t.text == "where" && paren == 0 && angle == 0 {
				break
			}
			if t.kind == tokPunct {
				switch t.text {
				case "{", ";":
					break loop
				case "(":
					paren++
				case ")":
					if paren > 0 {
						paren--
					}
				case "<":
					angle++
				case ">":
					if angle > 0 {
						angle--
					}
				case ",":
					if paren == 0 && angle == 0 {
						if len(cur) > 0 {
							bases = append(bases, strings.Join(cur, ""))
							cur = nil
						}
						j++
						continue
					}
				}
			}
			if t.kind == tokIdent || (t.kind == tokPunct && (t.text == "." || t.text == "::")) {
				cur = append(cur, t.text)
			}
			j++
		}
		if len(cur) > 0 {
			bases = append(bases, strings.Join(cur, ""))
		}
		next = j
		// generic constraints
		for next < end && p.isIdent(next) && p.text(next) == "where" {
			for next < end && !p.isPunct(next, "{") && !p.isPunct(next, ";") {
				next++
			}
		}
	}

	bodyOpen := -1
	bodyClose := -1
	for j := next; j < end; j++ {
		t := p.toks[j]
		if t.kind != tokPunct {
			continue
		}
		if t.text == "{" {
			bodyOpen = j
			break
		}
		if t.text == ";" {
			break
		}
	}

	endLine := startLine
	if bodyOpen >= 0 {
		bodyClose = p.matchClose(bodyOpen)
		endLine = p.tok(bodyClose).line
	}

	key := p.newKey(qual, kind, startLine)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "csharp", Kind: kind, Name: name, Qualname: qual,
		Namespace: ns, StartLine: startLine, EndLine: endLine,
		Signature: p.signatureText(i, bodyOpen, end),
		Modifiers: p.collectModifiers(i), ParamCount: -1, Container: parentQual,
	})
	p.types[qual] = kind

	for _, base := range bases {
		if base == "" {
			continue
		}
		p.res.Refs = append(p.res.Refs, model.Ref{
			Root: p.res.Root, SrcKey: key, Name: lastTypeName(base), Receiver: base,
			Kind: "inherits", Line: startLine, NamespaceHint: ns, Arity: -1,
		})
	}

	if bodyOpen >= 0 {
		if kind == model.KindEnum {
			p.collectEnumMembers(bodyOpen+1, bodyClose, qual, ns, key)
			return bodyClose + 1
		}
		p.parseBlock(bodyOpen+1, bodyClose, ns, qual)
		return bodyClose + 1
	}
	return next
}

// collectEnumMembers records enum constants; enum bodies are not member lists.
func (p *parser) collectEnumMembers(from, to int, typeQual, ns, parentKey string) {
	if from >= to {
		return
	}
	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.kind != tokIdent || modifiers[t.text] {
			continue
		}
		next := j + 1
		if next >= to {
			continue
		}
		nx := p.toks[next]
		if nx.kind == tokPunct && (nx.text == "=" || nx.text == "," || nx.text == "}") {
			name := t.text
			qual := typeQual + "." + name
			p.res.Symbols = append(p.res.Symbols, model.Symbol{
				Root: p.res.Root, Lang: "csharp", Kind: model.KindEnumMember, Name: name, Qualname: qual,
				Namespace: ns, StartLine: t.line, EndLine: t.line, Signature: name,
				ParamCount: -1, Container: typeQual,
			})
			p.res.Refs = append(p.res.Refs, model.Ref{
				Root: p.res.Root, SrcKey: parentKey, Name: name, Kind: "declares",
				Line: t.line, NamespaceHint: ns, Arity: -1,
			})
		}
	}
}

func (p *parser) collectModifiers(from int) string {
	var mods []string
	for j := from - 1; j >= 0; j-- {
		t := p.toks[j]
		if t.kind == tokPunct && (t.text == ";" || t.text == "{" || t.text == "}") {
			break
		}
		if t.kind == tokIdent && modifiers[t.text] {
			mods = append(mods, t.text)
		}
		if len(mods) >= 8 {
			break
		}
	}
	for l, r := 0, len(mods)-1; l < r; l, r = l+1, r-1 {
		mods[l], mods[r] = mods[r], mods[l]
	}
	return strings.Join(mods, " ")
}

func (p *parser) signatureText(from, bodyOpen, end int) string {
	stop := bodyOpen
	if stop < 0 {
		stop = from + 40
	}
	if stop > end {
		stop = end
	}
	var b strings.Builder
	for j := from; j < stop; j++ {
		t := p.toks[j]
		if t.kind == tokEOF {
			break
		}
		if b.Len() > 0 && p.toks[j-1].kind == tokIdent && t.kind == tokIdent {
			b.WriteByte(' ')
		}
		b.WriteString(t.text)
		if b.Len() > 400 {
			break
		}
	}
	return b.String()
}

func lastTypeName(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.Index(s, "<"); i >= 0 {
		s = s[:i]
	}
	if i := strings.LastIndex(s, "."); i >= 0 {
		s = s[i+1:]
	}
	if i := strings.IndexAny(s, "?["); i >= 0 {
		s = s[:i]
	}
	return s
}

func isAccessorKeyword(name string) bool {
	switch name {
	case "get", "set", "init", "add", "remove":
		return true
	}
	return false
}

func isValidIdent(s string) bool {	if s == "" {
		return false
	}
	if !isIdentStart(s[0]) {
		return false
	}
	for k := 0; k < len(s); k++ {
		if !isIdentPart(s[k]) {
			return false
		}
	}
	return true
}

func findSemicolon(toks []token, from, end int) int {
	depth := 0
	for j := from; j < end; j++ {
		t := toks[j]
		if t.kind != tokPunct {
			continue
		}
		switch t.text {
		case "(", "{", "[":
			depth++
		case ")", "]", "}":
			if depth > 0 {
				depth--
			} else if t.text == "}" {
				return j
			}
		case ";":
			if depth == 0 {
				return j
			}
		}
	}
	return end - 1
}

func countParams(toks []token, open, close int) int {
	if close <= open+1 {
		return 0
	}
	depth := 0
	count := 1
	hasIdent := false
	for j := open + 1; j < close; j++ {
		t := toks[j]
		if t.kind == tokIdent {
			hasIdent = true
		}
		if t.kind != tokPunct {
			continue
		}
		switch t.text {
		case "(", "{", "[", "<":
			depth++
		case ")", "}", "]", ">":
			if depth > 0 {
				depth--
			}
		case ",":
			if depth == 0 {
				count++
			}
		}
	}
	if !hasIdent {
		return 0
	}
	return count
}

// parseMember scans one member declaration starting at `from`.
func (p *parser) parseMember(from, end int, ns, typeQual string) int {
	// Initializer expressions and parameters look like declarations; only treat a
	// sequence as a member when it starts a statement (not after '=', '=>', ',').
	if from > 0 {
		prev := p.toks[from-1]
		if prev.kind == tokPunct && (prev.text == "=" || prev.text == "=>" || prev.text == "," ||
			prev.text == "(" || prev.text == "[" || prev.text == "return" || prev.text == ":") {
			return from
		}
	}
	i := from
	for i < end && p.isPunct(i, "[") {
		i = p.matchClose(i) + 1
	}
	start := i
	for start < end && p.isMod(start) {
		start++
	}
	if start >= end || !p.isIdent(start) {
		return from
	}
	startText := p.text(start)
	if notCalls[startText] || startText == "var" || startText == "const" {
		return from
	}
	// A member head must contain at least a type and a name; single-identifier
	// lines are labels/statements, not declarations.
	declHead := start
	depth := 0
	j := declHead
	term := ""
	termIdx := -1
	eqIdx := -1
	for j < end {
		t := p.toks[j]
		if t.kind == tokEOF {
			break
		}
		if t.kind == tokPunct {
			switch t.text {
			case "<":
				depth++
			case ">":
				if depth > 0 {
					depth--
				}
			}
			if depth == 0 {
				switch t.text {
				case "=":
					if eqIdx < 0 {
						eqIdx = j
					}
				case "(", "{", ";", "=>", "}":
					// after '=' a '(' belongs to the initializer (new T()), not to
					// a parameter list.
					if t.text == "(" && eqIdx >= 0 {
						break
					}
					term = t.text
					termIdx = j
				}
			}
			if term != "" {
				break
			}
		}
		j++
	}
	if term == "" || term == "}" || termIdx < 0 {
		return from
	}

	nameEnd := termIdx
	if eqIdx > 0 && eqIdx < termIdx {
		nameEnd = eqIdx
	}
	nameIdx := -1
	for k := nameEnd - 1; k >= declHead; k-- {
		if p.toks[k].kind == tokIdent {
			nameIdx = k
			break
		}
	}
	if nameIdx < declHead {
		return from
	}
	name := p.text(nameIdx)
	if !isValidIdent(name) || isAccessorKeyword(name) {
		return from
	}
	isCtor := nameIdx == declHead && term == "(" && name == lastTypeName(typeQual)
	if nameIdx == declHead && !isCtor {
		return from
	}
	typeText := p.typeTextOf(declHead, nameIdx)
	if typeText == "" && !isCtor {
		return from
	}

	line := p.tok(declHead).line
	kind := model.KindField
	paramCount := -1
	qual := typeQual + "." + name
	mods := p.collectModifiers(declHead)

	bodyOpen, bodyClose := -1, -1
	next := termIdx + 1
	switch term {
	case "(":
		closeParen := p.matchClose(termIdx)
		paramCount = countParams(p.toks, termIdx, closeParen)
		kind = model.KindMethod
		if name == lastTypeName(typeQual) {
			kind = model.KindConstructor
		}
		k := closeParen + 1
		next = closeParen + 1
		for k < end {
			if p.isPunct(k, "{") {
				bodyOpen = k
				bodyClose = p.matchClose(k)
				next = bodyClose + 1
				break
			}
			if p.isPunct(k, "=>") {
				bodyOpen = k
				bodyClose = findSemicolon(p.toks, k, end)
				next = bodyClose + 1
				break
			}
			if p.isPunct(k, ";") {
				next = k + 1
				break
			}
			if p.isPunct(k, "}") {
				break
			}
			k++
		}
	case "{":
		kind = model.KindProperty
		bodyOpen = termIdx
		bodyClose = p.matchClose(termIdx)
		next = bodyClose + 1
	case "=>":
		kind = model.KindProperty
		bodyOpen = termIdx
		bodyClose = findSemicolon(p.toks, termIdx, end)
		next = bodyClose + 1
	case ";":
		kind = model.KindField
		if strings.Contains(mods, "event") {
			kind = model.KindEvent
		}
		next = termIdx + 1
		if eqIdx > 0 && eqIdx < termIdx {
			bodyOpen = eqIdx
			bodyClose = termIdx
		}
	}

	endLine := p.tok(nameIdx).line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).line
	}
	sigEnd := bodyOpen
	if sigEnd < 0 {
		sigEnd = nameEnd
	}
	key := p.newKey(qual, kind, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "csharp", Kind: kind, Name: name, Qualname: qual,
		Namespace: ns, StartLine: line, EndLine: endLine,
		Signature: p.signatureText(declHead, sigEnd, end),
		Modifiers: mods, ParamCount: paramCount, Container: typeQual,
	})
	p.members[typeQual+"|"+name] = typeText

	if bodyOpen >= 0 && bodyClose > bodyOpen {
		var paramTypes map[string]string
		if term == "(" {
			paramTypes = p.paramTypes(termIdx)
		} else {
			paramTypes = map[string]string{}
		}
		p.scanRefs(bodyOpen+1, bodyClose, key, ns, typeQual, paramTypes)
	}
	if next <= from {
		return from + 1
	}
	return next
}

// typeTextOf joins declaration tokens, dropping generic bodies and initializers.
func (p *parser) typeTextOf(from, to int) string {
	var parts []string
	depth := 0
	stopped := false
	for j := from; j < to && !stopped; j++ {
		t := p.toks[j]
		if t.kind == tokEOF {
			break
		}
		if t.kind == tokPunct {
			switch t.text {
			case "<":
				depth++
				continue
			case ">":
				if depth > 0 {
					depth--
				}
				continue
			case ",", "=":
				stopped = true
				continue
			}
		}
		if depth > 0 {
			continue
		}
		parts = append(parts, t.text)
	}
	s := strings.Join(parts, "")
	for _, m := range []string{"public", "private", "protected", "internal", "static", "readonly", "const", "virtual", "override", "abstract", "async", "partial", "extern", "unsafe", "new", "sealed", "volatile", "required", "event", "file"} {
		s = strings.ReplaceAll(s, m, "")
	}
	return s
}

// paramTypes extracts "name -> type" for a parameter list.
func (p *parser) paramTypes(open int) map[string]string {
	out := map[string]string{}
	if !p.isPunct(open, "(") {
		return out
	}
	close := p.matchClose(open)
	depth := 0
	segStart := open + 1
	for j := open + 1; j < close; j++ {
		t := p.toks[j]
		if t.kind != tokPunct {
			continue
		}
		switch t.text {
		case "(", "{", "[", "<":
			depth++
		case ")", "}", "]", ">":
			if depth > 0 {
				depth--
			}
		case ",":
			if depth == 0 {
				addParam(p.toks, out, segStart, j)
				segStart = j + 1
			}
		}
	}
	addParam(p.toks, out, segStart, close)
	return out
}

func addParam(toks []token, out map[string]string, from, to int) {
	var idents []string
	for j := from; j < to; j++ {
		t := toks[j]
		if t.kind != tokIdent {
			continue
		}
		if t.text == "ref" || t.text == "out" || t.text == "in" || t.text == "params" || t.text == "this" || t.text == "scoped" {
			continue
		}
		idents = append(idents, t.text)
	}
	if len(idents) >= 2 {
		out[idents[len(idents)-1]] = idents[len(idents)-2]
	}
}

func (p *parser) scanRefs(from, to int, srcKey, ns, ownerQual string, paramTypes map[string]string) {
	varTypes := map[string]string{}
	for k, v := range paramTypes {
		varTypes[k] = v
	}
	p.collectLocalVars(from, to, varTypes)

	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.kind != tokIdent {
			continue
		}
		if t.text == "new" {
			k := j + 1
			var parts []string
			for k < to {
				x := p.toks[k]
				if x.kind == tokIdent {
					parts = append(parts, x.text)
					k++
					continue
				}
				if x.kind == tokPunct && (x.text == "." || x.text == "::") {
					parts = append(parts, ".")
					k++
					continue
				}
				break
			}
			if len(parts) > 0 {
				name := lastTypeName(strings.Join(parts, ""))
				if name != "" {
					p.addRef(srcKey, "", "constructs", name, "", strings.Join(parts, ""), t.line, ns, -1)
				}
			}
			continue
		}
		if notCalls[t.text] || t.text == "base" && p.isPunct(j+1, ".") {
			continue
		}
		next := j + 1
		if p.isPunct(next, "<") {
			gt := p.matchAngle(next, to)
			if gt > 0 && p.isPunct(gt+1, "(") {
				recv, recvType := p.receiverBefore(j, from, varTypes, ownerQual)
				p.addRef(srcKey, recv, "calls", t.text, recvType, "", t.line, ns, countParams(p.toks, gt+1, p.matchClose(gt+1)))
				continue
			}
		}
		if p.isPunct(next, "(") {
			recv, recvType := p.receiverBefore(j, from, varTypes, ownerQual)
			p.addRef(srcKey, recv, "calls", t.text, recvType, "", t.line, ns, countParams(p.toks, next, p.matchClose(next)))
			continue
		}
		if (p.isPunct(next, ".") || p.isPunct(next, "?.")) && p.isIdent(next+1) {
			// A member call (x.M(...)) is recorded as `calls` on M, not as a
			// reference edge from x to M; avoid the duplicate lower-value edge.
			if p.isPunct(next+2, "(") || p.isPunct(next+2, "<") {
				continue
			}
			p.addRef(srcKey, t.text, "references", p.text(next+1), "", "", t.line, ns, -1)
		}
	}
}

// receiverBefore resolves the receiver of a call at index nameIdx.
func (p *parser) receiverBefore(nameIdx, from int, varTypes map[string]string, ownerQual string) (string, string) {
	if nameIdx <= from || !(p.isPunct(nameIdx-1, ".") || p.isPunct(nameIdx-1, "?.")) {
		return "", ""
	}
	end := nameIdx - 2
	if end < from {
		return "", ""
	}
	recv := ""
	switch {
	case p.isIdent(end):
		recv = p.text(end)
		for end-2 >= from && (p.isPunct(end-1, ".") || p.isPunct(end-1, "?.")) && p.isIdent(end-2) {
			recv = p.text(end-2) + "." + recv
			end -= 2
		}
	case p.isPunct(end, ")"):
		depth := 1
		for k := end - 1; k >= from; k-- {
			if p.isPunct(k, ")") {
				depth++
			} else if p.isPunct(k, "(") {
				depth--
				if depth == 0 {
					if k-1 >= from && p.isIdent(k-1) {
						recv = p.text(k - 1)
					}
					break
				}
			}
		}
	case p.isPunct(end, "]"):
		depth := 1
		for k := end - 1; k >= from; k-- {
			if p.isPunct(k, "]") {
				depth++
			} else if p.isPunct(k, "[") {
				depth--
				if depth == 0 {
					if k-1 >= from && p.isIdent(k-1) {
						recv = p.text(k - 1)
					}
					break
				}
			}
		}
	}
	if recv == "" {
		return "", ""
	}
	recvType := ""
	head := recv
	if i := strings.Index(head, "."); i >= 0 {
		head = head[:i]
	}
	if vt, ok := varTypes[head]; ok {
		recvType = vt
	} else if mt, ok := p.members[ownerQual+"|"+head]; ok {
		recvType = mt
	} else if head == "this" || head == "base" {
		recvType = ownerQual
	} else if c := head[0]; c >= 'A' && c <= 'Z' {
		recvType = head
	}
	return recv, recvType
}

func (p *parser) collectLocalVars(from, to int, varTypes map[string]string) {
	for j := from; j+2 < to; j++ {
		if p.isIdent(j) && p.text(j) == "var" && p.isIdent(j+1) {
			name := p.text(j + 1)
			if p.isPunct(j+2, "=") && j+3 < to && p.isIdent(j+3) && p.text(j+3) == "new" {
				k := j + 4
				var parts []string
				for k < to {
					x := p.toks[k]
					if x.kind == tokIdent {
						parts = append(parts, x.text)
						k++
						continue
					}
					if x.kind == tokPunct && (x.text == "." || x.text == "::") {
						parts = append(parts, ".")
						k++
						continue
					}
					break
				}
				if len(parts) > 0 {
					varTypes[name] = lastTypeName(strings.Join(parts, ""))
				}
			}
			continue
		}
		if p.isIdent(j) && p.isIdent(j+1) {
			if !(p.isPunct(j+2, "=") || p.isPunct(j+2, ";") || p.isPunct(j+2, ",")) {
				continue
			}
			t := p.text(j)
			if notCalls[t] || t == "var" || t == "return" || t == "else" || t == "case" || t == "is" || t == "as" {
				continue
			}
			varTypes[p.text(j+1)] = t
		}
	}
}

func (p *parser) addRef(srcKey, receiver, kind, name, recvType, receiverText string, line int, ns string, arity int) {
	if name == "" {
		return
	}
	p.res.Refs = append(p.res.Refs, model.Ref{
		Root: p.res.Root, SrcKey: srcKey, Name: name, Receiver: receiver, ReceiverType: recvType,
		Kind: kind, Line: line, NamespaceHint: ns, Arity: arity,
	})
}
