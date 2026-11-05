// Package rust indexes Rust sources with a declaration scanner: mod/struct/
// enum/trait/impl/fn/type, use-alias resolution, calls, construction,
// trait implementation and field type inference.  It is intentionally not a
// compiler; unresolved paths fall back to name matching with an explicit
// confidence at bind time.
package rust

import (
	"path"
	"strings"

	"github.com/neko233-com/codegraph233/internal/lang/scanner"
	"github.com/neko233-com/codegraph233/internal/model"
)

var modKeywords = map[string]bool{
	"pub": true, "crate": true, "super": true, "self": true, "async": true,
	"unsafe": true, "extern": true, "const": true, "static": true, "default": true,
	"move": true, "ref": true, "mut": true, "dyn": true, "impl": true, "where": true,
}

type parser struct {
	toks    []scanner.Token
	res     *model.FileResult
	aliases map[string]string // local alias -> full path (crate::a::B)
	vars    map[string]string // current fn scope: name -> type path
	keySeq  int
}

// Parse extracts Rust declarations and references from one file.
func Parse(relPath, root string, src []byte) *model.FileResult {
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "rust"}
	p := &parser{
		toks:    scanner.Tokenize(src, scanner.Options{NestedComments: true, RustLiterals: true}),
		res:     res,
		aliases: map[string]string{},
		vars:    map[string]string{},
	}
	p.parseItems(0, len(p.toks)-1, moduleOf(relPath))
	return res
}

// moduleOf maps a source path to a Rust module path.
func moduleOf(relPath string) string {
	rel := strings.TrimSuffix(path.Clean(relPath), ".rs")
	parts := strings.Split(rel, "/")
	if len(parts) > 0 && (parts[0] == "src" || parts[0] == "lib" || parts[0] == "tests" || parts[0] == "benches") {
		parts = parts[1:]
	}
	if len(parts) > 0 && parts[len(parts)-1] == "mod" {
		parts = parts[:len(parts)-1]
	}
	if len(parts) == 0 || (len(parts) == 1 && (parts[0] == "main" || parts[0] == "lib")) {
		return "crate"
	}
	if len(parts) > 0 && (parts[len(parts)-1] == "main" || parts[len(parts)-1] == "lib") {
		parts = parts[:len(parts)-1]
	}
	if len(parts) == 0 {
		return "crate"
	}
	return "crate::" + strings.Join(parts, "::")
}

func (p *parser) tok(i int) scanner.Token {
	if i < 0 || i >= len(p.toks) {
		return scanner.Token{Kind: scanner.EOF}
	}
	return p.toks[i]
}

func (p *parser) text(i int) string { return p.tok(i).Text }

func (p *parser) isPunct(i int, s string) bool {
	t := p.tok(i)
	return t.Kind == scanner.Punct && t.Text == s
}

func (p *parser) isIdent(i int) bool { return p.tok(i).Kind == scanner.Ident }

func (p *parser) key(qual, kind string, line int) string {
	return qual + "#" + kind + "@" + itoa(line)
}

func itoa(v int) string {
	if v == 0 {
		return "0"
	}
	var b [20]byte
	i := len(b)
	for v > 0 {
		i--
		b[i] = byte('0' + v%10)
		v /= 10
	}
	return string(b[i:])
}

func (p *parser) matchClose(open int) int {
	depth := 0
	for j := open; j < len(p.toks); j++ {
		t := p.toks[j]
		if t.Kind != scanner.Punct {
			continue
		}
		switch t.Text {
		case "(", "[", "{":
			depth++
		case ")", "]", "}":
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
		if t.Kind != scanner.Punct {
			continue
		}
		switch t.Text {
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

func (p *parser) signature(from, bodyOpen, end int) string {
	stop := bodyOpen
	if stop < 0 {
		stop = from + 24
	}
	if stop > end {
		stop = end
	}
	var b strings.Builder
	for j := from; j < stop; j++ {
		t := p.toks[j]
		if t.Kind == scanner.EOF {
			break
		}
		if b.Len() > 0 && p.toks[j-1].Kind == scanner.Ident && t.Kind == scanner.Ident {
			b.WriteByte(' ')
		}
		b.WriteString(t.Text)
		if b.Len() > 400 {
			break
		}
	}
	return b.String()
}

// parseItems walks a module body (or the file root).
func (p *parser) parseItems(from, to int, modPath string) {
	i := from
	for i < to {
		t := p.toks[i]
		if t.Kind == scanner.EOF {
			return
		}
		if t.Kind == scanner.Punct {
			if t.Text == "}" {
				return
			}
			if t.Text == "{" || t.Text == "(" || t.Text == "[" {
				i = p.matchClose(i) + 1
				continue
			}
			i++
			continue
		}
		if t.Kind != scanner.Ident {
			i++
			continue
		}
		if t.Text == "#" && p.isPunct(i+1, "[") {
			i = p.matchClose(i+1) + 1
			continue
		}
		switch t.Text {
		case "mod":
			i = p.parseMod(i, to, modPath)
		case "use":
			i = p.parseUse(i, to)
		case "struct":
			i = p.parseStruct(i, to, modPath)
		case "enum":
			i = p.parseEnum(i, to, modPath)
		case "trait":
			i = p.parseTrait(i, to, modPath)
		case "impl":
			i = p.parseImpl(i, to, modPath)
		case "fn":
			i = p.parseFn(i, to, modPath, "")
		case "type":
			i = p.parseTypeAlias(i, to, modPath)
		case "const", "static":
			i = p.parseConst(i, to, modPath)
		default:
			i++
		}
	}
}

func (p *parser) parseMod(i, end int, modPath string) int {
	j := i + 1
	name := ""
	for j < end {
		t := p.toks[j]
		if t.Kind == scanner.Punct && (t.Text == "{" || t.Text == ";") {
			break
		}
		if t.Kind == scanner.Ident && name == "" {
			name = t.Text
		}
		j++
	}
	if p.isPunct(j, "{") {
		close := p.matchClose(j)
		sub := modPath + "::" + name
		p.parseItems(j+1, close, sub)
		return close + 1
	}
	return j + 1
}

// parseUse records alias -> full path for `use a::b::{C, D as E};`.
func (p *parser) parseUse(i, end int) int {
	j := i + 1
	start := j
	depth := 0
	for j < end {
		t := p.toks[j]
		if t.Kind == scanner.Punct {
			if t.Text == "{" {
				depth++
			} else if t.Text == "}" {
				depth--
			} else if t.Text == ";" && depth == 0 {
				break
			}
		}
		j++
	}
	p.recordUse(start, j, "")
	if j < end {
		return j + 1
	}
	return j
}

func (p *parser) recordUse(from, to int, prefix string) {
	// split the segment by commas at depth 0 and resolve nested groups
	var seg [][2]int
	depth := 0
	start := from
	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.Kind != scanner.Punct {
			continue
		}
		switch t.Text {
		case "{":
			depth++
		case "}":
			depth--
		case ",":
			if depth == 0 {
				seg = append(seg, [2]int{start, j})
				start = j + 1
			}
		}
	}
	if start < to {
		seg = append(seg, [2]int{start, to})
	}
	for _, s := range seg {
		p.recordUseSeg(s[0], s[1], prefix)
	}
}

func (p *parser) recordUseSeg(from, to int, prefix string) {
	// path up to '{' or 'as'
	var parts []string
	alias := ""
	groupStart := -1
	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.Kind == scanner.Ident {
			if t.Text == "as" {
				if j+1 < to && p.isIdent(j+1) {
					alias = p.text(j + 1)
				}
				continue
			}
			if alias != "" {
				continue
			}
			parts = append(parts, t.Text)
			continue
		}
		if t.Kind == scanner.Punct && t.Text == "{" {
			groupStart = j
			break
		}
	}
	if groupStart >= 0 {
		close := p.matchClose(groupStart)
		base := strings.Join(parts, "::")
		if prefix != "" {
			if base == "" {
				base = prefix
			} else {
				base = prefix + "::" + base
			}
		}
		p.recordUse(groupStart+1, close, base)
		return
	}
	if len(parts) == 0 {
		return
	}
	full := strings.Join(parts, "::")
	if prefix != "" {
		full = prefix + "::" + full
	}
	name := parts[len(parts)-1]
	if alias != "" {
		name = alias
	}
	p.aliases[name] = full
}

func (p *parser) parseStruct(i, end int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "::" + name
	line := p.tok(i).Line
	k := j + 1
	if p.isPunct(k, "<") {
		if gt := p.matchAngle(k, end); gt > 0 {
			k = gt + 1
		}
	}
	// fields
	bodyOpen, bodyClose := -1, -1
	for x := k; x < end; x++ {
		if p.isPunct(x, "{") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
		if p.isPunct(x, "(") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
		if p.isPunct(x, ";") {
			break
		}
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	key := p.key(qual, model.KindStruct, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: model.KindStruct, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, end), ParamCount: -1, Module: modPath,
	})
	if bodyOpen >= 0 && p.isPunct(bodyOpen, "{") {
		depth := 0
		for x := bodyOpen + 1; x < bodyClose; x++ {
			t := p.toks[x]
			if t.Kind == scanner.Punct {
				switch t.Text {
				case "{", "(", "[":
					depth++
				case "}", ")", "]":
					depth--
				}
				continue
			}
			if depth != 0 || t.Kind != scanner.Ident {
				continue
			}
			// name: Type
			if p.isPunct(x+1, ":") && p.isIdent(x+2) {
				fname := t.Text
				ftype := p.readTypePath(x + 2, bodyClose)
				fqual := qual + "::" + fname
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.res.Root, Lang: "rust", Kind: model.KindField, Name: fname, Qualname: fqual,
					Namespace: modPath, StartLine: t.Line, EndLine: t.Line, Signature: fname + ": " + ftype,
					ParamCount: -1, Container: qual, Module: modPath,
				})
				if ftype != "" {
					p.res.Refs = append(p.res.Refs, model.Ref{
						Root: p.res.Root, SrcKey: key, Name: lastSeg(ftype), Kind: "references",
						Receiver: ftype, ReceiverType: resolveRustPath(ftype, p.aliases, modPath),
						Line: t.Line, NamespaceHint: modPath, Arity: -1,
					})
				}
				x += 2
			}
		}
	}
	if bodyClose > 0 {
		return bodyClose + 1
	}
	return k
}

func (p *parser) parseEnum(i, end int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "::" + name
	line := p.tok(i).Line
	bodyOpen, bodyClose := -1, -1
	for x := j + 1; x < end; x++ {
		if p.isPunct(x, "{") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
		if p.isPunct(x, ";") {
			break
		}
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: model.KindEnum, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, end), ParamCount: -1, Module: modPath,
	})
	if bodyOpen >= 0 {
		depth := 0
		for x := bodyOpen + 1; x < bodyClose; x++ {
			t := p.toks[x]
			if t.Kind == scanner.Punct {
				switch t.Text {
				case "(", "{", "[":
					depth++
				case ")", "}", "]":
					depth--
				}
				continue
			}
			if depth == 0 && t.Kind == scanner.Ident {
				vqual := qual + "::" + t.Text
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.res.Root, Lang: "rust", Kind: model.KindEnumMember, Name: t.Text, Qualname: vqual,
					Namespace: modPath, StartLine: t.Line, EndLine: t.Line, Signature: t.Text,
					ParamCount: -1, Container: qual, Module: modPath,
				})
			}
		}
		return bodyClose + 1
	}
	return j
}

func (p *parser) parseTrait(i, end int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "::" + name
	line := p.tok(i).Line
	// super trait: `trait X: Base + Other`
	var bases []string
	x := j + 1
	if p.isPunct(x, ":") {
		x++
		var cur []string
		for x < end && !p.isPunct(x, "{") && !p.isPunct(x, ";") {
			t := p.toks[x]
			if t.Kind == scanner.Ident && !modKeywords[t.Text] {
				cur = append(cur, t.Text)
			}
			if t.Kind == scanner.Punct && (t.Text == "," || t.Text == "+") {
				if len(cur) > 0 {
					bases = append(bases, strings.Join(cur, ""))
					cur = nil
				}
			}
			x++
		}
		if len(cur) > 0 {
			bases = append(bases, strings.Join(cur, ""))
		}
	}
	bodyOpen, bodyClose := -1, -1
	for y := x; y < end; y++ {
		if p.isPunct(y, "{") {
			bodyOpen = y
			bodyClose = p.matchClose(y)
			break
		}
		if p.isPunct(y, ";") {
			break
		}
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	key := p.key(qual, model.KindInterface, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: model.KindInterface, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, end), ParamCount: -1, Module: modPath,
	})
	for _, b := range bases {
		p.res.Refs = append(p.res.Refs, model.Ref{
			Root: p.res.Root, SrcKey: key, Name: lastSeg(b), Kind: "implements",
			Receiver: b, ReceiverType: resolveRustPath(b, p.aliases, modPath),
			Line: line, NamespaceHint: modPath, Arity: -1,
		})
	}
	if bodyOpen >= 0 {
		p.parseTraitBody(bodyOpen+1, bodyClose, modPath, qual, key)
		return bodyClose + 1
	}
	return x
}

// parseTraitBody records declaration-only methods (no body).
func (p *parser) parseTraitBody(from, to int, modPath, traitQual, traitKey string) {
	i := from
	for i < to {
		if p.isPunct(i, "{") {
			i = p.matchClose(i) + 1
			continue
		}
		if p.isIdent(i) && p.text(i) == "fn" {
			j := i + 1
			if !p.isIdent(j) {
				i++
				continue
			}
			name := p.text(j)
			qual := traitQual + "::" + name
			p.res.Symbols = append(p.res.Symbols, model.Symbol{
				Root: p.res.Root, Lang: "rust", Kind: model.KindMethod, Name: name, Qualname: qual,
				Namespace: modPath, StartLine: p.tok(i).Line, EndLine: p.tok(i).Line,
				Signature: p.signature(i, -1, to), ParamCount: -1, Container: traitQual, Module: modPath,
			})
			for j < to && !p.isPunct(j, ";") && !p.isPunct(j, "{") {
				j++
			}
			if p.isPunct(j, "{") {
				j = p.matchClose(j) + 1
			}
			i = j
			continue
		}
		i++
	}
}

func (p *parser) parseImpl(i, end int, modPath string) int {
	j := i + 1
	if p.isPunct(j, "<") {
		if gt := p.matchAngle(j, end); gt > 0 {
			j = gt + 1
		}
	}
	// head: [Trait for] Type
	var head []string
	x := j
	for x < end && !p.isPunct(x, "{") && !p.isPunct(x, ";") {
		t := p.toks[x]
		if t.Kind == scanner.Ident {
			head = append(head, t.Text)
		}
		if t.Kind == scanner.Punct && (t.Text == "::" || t.Text == "<" || t.Text == ">" || t.Text == ",") {
			// keep generics out of the simple path
			if t.Text == "<" {
				if gt := p.matchAngle(x, end); gt > 0 {
					x = gt + 1
					continue
				}
			}
		}
		x++
	}
	traitName, typeName := "", ""
	for k, w := range head {
		if w == "for" {
			if k > 0 {
				traitName = head[k-1]
			}
			if k+1 < len(head) {
				typeName = head[k+1]
			}
			break
		}
	}
	if typeName == "" && len(head) > 0 {
		typeName = head[len(head)-1]
	}
	if typeName == "" {
		return x
	}
	typeQual := modPath + "::" + typeName
	if p.isPunct(x, "{") {
		close := p.matchClose(x)
		if traitName != "" {
			srcKey := p.typeKey(typeQual)
			p.res.Refs = append(p.res.Refs, model.Ref{
				Root: p.res.Root, SrcKey: srcKey, Name: traitName,
				Kind: "implements", Receiver: traitName,
				ReceiverType: resolveRustPath(traitName, p.aliases, modPath),
				Line: p.tok(i).Line, NamespaceHint: modPath, Arity: -1,
			})
		}
		p.parseItems(x+1, close, modPath)
		// impl bodies contain fn at top level; parseItems handles `fn` with the
		// current modPath, so rewrite container afterwards using a marker pass.
		p.retargetMethods(x+1, close, typeQual, modPath)
		return close + 1
	}
	return x
}

// typeKey finds the declaration key of a type in this file.
func (p *parser) typeKey(qual string) string {
	for _, s := range p.res.Symbols {
		if s.Qualname == qual && model.IsTypeKind(s.Kind) {
			return s.Qualname + "#" + s.Kind + "@" + itoa(s.StartLine)
		}
	}
	return qual + "#struct@0"
}

// retargetMethods re-points methods parsed inside an impl block to the impl type.
func (p *parser) retargetMethods(from, to int, typeQual, modPath string) {
	for idx := range p.res.Symbols {
		s := &p.res.Symbols[idx]
		if s.Lang != "rust" || s.Container != "" {
			continue
		}
		if s.Kind != model.KindMethod && s.Kind != model.KindFunction {
			continue
		}
		if s.StartLine < p.tok(from).Line || s.StartLine > p.tok(to).Line {
			continue
		}
		s.Kind = model.KindMethod
		s.Container = typeQual
		s.Qualname = typeQual + "::" + s.Name
	}
}

func (p *parser) parseFn(i, end int, modPath, container string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	line := p.tok(i).Line
	qual := modPath + "::" + name
	kind := model.KindFunction
	if container != "" {
		qual = container + "::" + name
		kind = model.KindMethod
	}
	k := j + 1
	if p.isPunct(k, "<") {
		if gt := p.matchAngle(k, end); gt > 0 {
			k = gt + 1
		}
	}
	paramsOpen, paramsClose := -1, -1
	if p.isPunct(k, "(") {
		paramsOpen = k
		paramsClose = p.matchClose(k)
	}
	bodyOpen, bodyClose := -1, -1
	if paramsClose > 0 {
		for x := paramsClose + 1; x < end; x++ {
			if p.isPunct(x, "{") {
				bodyOpen = x
				bodyClose = p.matchClose(x)
				break
			}
			if p.isPunct(x, ";") {
				break
			}
		}
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	key := p.key(qual, kind, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: kind, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, end), ParamCount: p.paramCount(paramsOpen),
		Container: container, Module: modPath,
	})
	if bodyOpen >= 0 {
		vars := p.collectVars(paramsOpen, paramsClose, bodyOpen, bodyClose)
		p.scanRefs(bodyOpen+1, bodyClose, key, modPath, container, vars)
	}
	if bodyClose > 0 {
		return bodyClose + 1
	}
	return k
}

func (p *parser) paramCount(open int) int {
	if open < 0 {
		return -1
	}
	close := p.matchClose(open)
	depth := 0
	count := 0
	has := false
	for j := open + 1; j < close; j++ {
		t := p.toks[j]
		if t.Kind == scanner.Ident {
			has = true
		}
		if t.Kind != scanner.Punct {
			continue
		}
		switch t.Text {
		case "(", "[", "{", "<":
			depth++
		case ")", "]", "}", ">":
			if depth > 0 {
				depth--
			}
		case ",":
			if depth == 0 {
				count++
			}
		}
	}
	if !has {
		return 0
	}
	return count + 1
}

func (p *parser) collectVars(paramsOpen, paramsClose, bodyOpen, bodyClose int) map[string]string {
	vars := map[string]string{}
	if paramsOpen >= 0 && paramsClose > paramsOpen {
		depth := 0
		segStart := paramsOpen + 1
		for j := paramsOpen + 1; j <= paramsClose; j++ {
			if j == paramsClose || (p.isPunct(j, ",") && depth == 0) {
				seg := []string{}
				for x := segStart; x < j; x++ {
					if p.toks[x].Kind == scanner.Ident && p.toks[x].Text != "mut" && p.toks[x].Text != "ref" {
						seg = append(seg, p.toks[x].Text)
					}
				}
				if len(seg) >= 2 {
					vars[seg[0]] = seg[1]
				}
				segStart = j + 1
				continue
			}
			if p.toks[j].Kind == scanner.Punct {
				switch p.toks[j].Text {
				case "(", "[", "{", "<":
					depth++
				case ")", "]", "}", ">":
					if depth > 0 {
						depth--
					}
				}
			}
		}
	}
	// `let x: T = ...` / `let x = T::new(...)`
	for j := bodyOpen + 1; j+2 < bodyClose; j++ {
		if !(p.isIdent(j) && p.text(j) == "let") {
			continue
		}
		k := j + 1
		if p.isIdent(k) && (p.text(k) == "mut" || p.text(k) == "ref") {
			k++
		}
		if !p.isIdent(k) {
			continue
		}
		name := p.text(k)
		if p.isPunct(k+1, ":") && p.isIdent(k+2) {
			vars[name] = p.readTypePath(k+2, bodyClose)
			continue
		}
		if p.isPunct(k+1, "=") && p.isIdent(k+2) {
			// Type::new(...) / Type { ... } / Type(...)
			if p.isPunct(k+3, "::") && p.isIdent(k+4) {
				vars[name] = p.text(k + 2)
			} else if p.isPunct(k+3, "{") || p.isPunct(k+3, "(") {
				vars[name] = p.text(k + 2)
			}
		}
	}
	return vars
}

func (p *parser) readTypePath(from, end int) string {
	var parts []string
	j := from
	for j < end {
		t := p.toks[j]
		if t.Kind == scanner.Ident && !modKeywords[t.Text] && t.Text != "for" && t.Text != "where" {
			parts = append(parts, t.Text)
			j++
			continue
		}
		if t.Kind == scanner.Punct && (t.Text == "::" || t.Text == "&") {
			j++
			continue
		}
		break
	}
	return strings.Join(parts, "::")
}

func (p *parser) parseTypeAlias(i, end int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "::" + name
	line := p.tok(i).Line
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: model.KindType, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: line,
		Signature: p.signature(i, -1, end), ParamCount: -1, Module: modPath,
	})
	for j < end && !p.isPunct(j, ";") {
		j++
	}
	return j + 1
}

func (p *parser) parseConst(i, end int, modPath string) int {
	j := i + 1
	for j < end && !p.isIdent(j) {
		j++
	}
	if j >= end {
		return j
	}
	name := p.text(j)
	qual := modPath + "::" + name
	line := p.tok(j).Line
	kind := model.KindField
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "rust", Kind: kind, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: line,
		Signature: p.signature(i, -1, end), ParamCount: -1, Module: modPath,
	})
	for j < end && !p.isPunct(j, ";") {
		j++
	}
	return j + 1
}

func (p *parser) scanRefs(from, to int, srcKey, modPath, ownerQual string, vars map[string]string) {
	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.Kind != scanner.Ident {
			continue
		}
		if t.Text == "let" || t.Text == "match" || t.Text == "if" || t.Text == "for" ||
			t.Text == "while" || t.Text == "return" || t.Text == "fn" || t.Text == "impl" {
			continue
		}
		next := j + 1
		// path call: a::b::c(...)
		if p.isPunct(next, "::") {
			k := j
			var segs []string
			for k < to && (p.isIdent(k) || p.isPunct(k, "::")) {
				if p.isIdent(k) {
					segs = append(segs, p.text(k))
				}
				k++
			}
			if len(segs) >= 2 && (p.isPunct(k, "(") || p.isPunct(k, "{")) {
				name := segs[len(segs)-1]
				recv := strings.Join(segs[:len(segs)-1], "::")
				kind := "calls"
				if p.isPunct(k, "{") {
					kind = "constructs"
				}
				p.addRef(srcKey, recv, kind, name, resolveRustPath(recv, p.aliases, modPath), p.tok(j).Line, modPath, p.argCount(k))
			}
			continue
		}
		// method call: x.method(...)
		if p.isPunct(next, ".") && p.isIdent(next+1) {
			if p.isPunct(next+2, "(") {
				recv := t.Text
				recvType := vars[recv]
				if recvType == "" {
					recvType = p.knownType(recv, ownerQual)
				}
				p.addRef(srcKey, recv, "calls", p.text(next+1), recvType, p.tok(j).Line, modPath, p.argCount(next+2))
				continue
			}
			p.addRef(srcKey, t.Text, "references", p.text(next+1), vars[t.Text], p.tok(j).Line, modPath, -1)
			continue
		}
		// free call or struct literal: Name( / Name {
		if p.isPunct(next, "(") {
			p.addRef(srcKey, "", "calls", t.Text, "", p.tok(j).Line, modPath, p.argCount(next))
			continue
		}
		if p.isPunct(next, "{") && len(t.Text) > 0 && t.Text[0] >= 'A' && t.Text[0] <= 'Z' {
			p.addRef(srcKey, "", "constructs", t.Text, t.Text, p.tok(j).Line, modPath, -1)
		}
	}
}

func (p *parser) knownType(name, ownerQual string) string {
	if ownerQual == "" {
		return ""
	}
	// self -> impl type
	if name == "self" {
		return ownerQual
	}
	return ""
}

func (p *parser) argCount(open int) int {
	if !p.isPunct(open, "(") {
		return -1
	}
	close := p.matchClose(open)
	if close <= open+1 {
		return 0
	}
	depth := 0
	count := 1
	for j := open + 1; j < close; j++ {
		t := p.toks[j]
		if t.Kind != scanner.Punct {
			continue
		}
		switch t.Text {
		case "(", "[", "{", "<":
			depth++
		case ")", "]", "}", ">":
			if depth > 0 {
				depth--
			}
		case ",":
			if depth == 0 {
				count++
			}
		}
	}
	return count
}

func (p *parser) addRef(srcKey, receiver, kind, name, recvType string, line int, modPath string, arity int) {
	if name == "" {
		return
	}
	p.res.Refs = append(p.res.Refs, model.Ref{
		Root: p.res.Root, SrcKey: srcKey, Name: name, Receiver: receiver, ReceiverType: recvType,
		Kind: kind, Line: line, NamespaceHint: modPath, Arity: arity,
	})
}

func lastSeg(s string) string {
	if i := strings.LastIndex(s, "::"); i >= 0 {
		return s[i+2:]
	}
	return s
}

// resolveRustPath expands crate::/self::/super:: and local use-aliases.
func resolveRustPath(p string, aliases map[string]string, modPath string) string {
	if p == "" {
		return ""
	}
	if full, ok := aliases[p]; ok {
		return full
	}
	parts := strings.Split(p, "::")
	if len(parts) > 0 {
		if full, ok := aliases[parts[0]]; ok {
			parts[0] = full
			return strings.Join(parts, "::")
		}
	}
	switch parts[0] {
	case "crate":
		return p
	case "self":
		return modPath + "::" + strings.Join(parts[1:], "::")
	case "super":
		base := modPath
		for strings.HasPrefix(strings.Join(parts, "::"), "super") {
			if i := strings.LastIndex(base, "::"); i > 0 {
				base = base[:i]
			}
			parts = parts[1:]
		}
		if len(parts) > 0 {
			return base + "::" + strings.Join(parts, "::")
		}
		return base
	}
	// bare name: prefer a local alias, else keep it for name-based binding
	if full, ok := aliases[parts[0]]; ok {
		return full
	}
	return p
}
