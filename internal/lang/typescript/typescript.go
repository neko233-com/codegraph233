// Package typescript indexes TypeScript/JavaScript sources with a declaration
// scanner: import aliases, class/interface/type/enum/function, methods and
// properties, calls, construction and inheritance.
package typescript

import (
	"path"
	"strings"

	"github.com/neko233-com/codegraph233/internal/lang/scanner"
	"github.com/neko233-com/codegraph233/internal/model"
)

type parser struct {
	toks    []scanner.Token
	res     *model.FileResult
	aliases map[string]string // local name -> module.Export
	elementByModule map[string]string
	keySeq  int
}

// Parse extracts TypeScript declarations and references from one file.
func Parse(relPath, root string, src []byte) *model.FileResult {
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "typescript"}
	p := &parser{
		toks:    scanner.Tokenize(src, scanner.Options{TemplateStrings: true}),
		res:     res,
		aliases: map[string]string{},
	}
	p.parseItems(0, len(p.toks)-1, moduleOf(relPath))
	return res
}

// moduleOf maps a file path to a dotted module name.
func moduleOf(relPath string) string {
	rel := strings.TrimSuffix(path.Clean(relPath), ".tsx")
	rel = strings.TrimSuffix(rel, ".ts")
	rel = strings.TrimSuffix(rel, ".js")
	rel = strings.TrimSuffix(rel, ".jsx")
	if strings.HasSuffix(rel, "/index") {
		rel = strings.TrimSuffix(rel, "/index")
	}
	return strings.ReplaceAll(rel, "/", ".")
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

var keywords = map[string]bool{
	"if": true, "for": true, "while": true, "switch": true, "catch": true,
	"return": true, "new": true, "typeof": true, "instanceof": true, "delete": true,
	"in": true, "of": true, "do": true, "else": true, "case": true, "throw": true,
	"await": true, "yield": true, "super": true, "this": true, "function": true,
}

var modifiers = map[string]bool{
	"public": true, "private": true, "protected": true, "static": true, "readonly": true,
	"abstract": true, "async": true, "override": true, "declare": true, "export": true,
	"default": true, "const": true, "let": true, "var": true, "get": true, "set": true,
}

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
			if t.Text == "@" {
				// decorator: @Name or @Name(...)
				j := i + 1
				if p.isIdent(j) {
					j++
				}
				if p.isPunct(j, "(") {
					j = p.matchClose(j) + 1
				}
				i = j
				continue
			}
			i++
			continue
		}
		if t.Kind != scanner.Ident {
			i++
			continue
		}
		switch t.Text {
		case "import":
			i = p.parseImport(i, to)
		case "class":
			i = p.parseClass(i, to, modPath)
		case "interface":
			i = p.parseInterface(i, to, modPath)
		case "type":
			i = p.parseTypeAlias(i, to, modPath)
		case "enum":
			i = p.parseEnum(i, to, modPath)
		case "function":
			i = p.parseFunction(i, to, modPath)
		case "const", "let", "var":
			i = p.parseVarDecl(i, to, modPath)
		default:
			i++
		}
	}
}

// parseImport resolves named/default/namespace imports into aliases.
func (p *parser) parseImport(i, to int) int {
	j := i + 1
	fromModule := ""
	for j < to && !p.isPunct(j, ";") {
		if p.toks[j].Kind == scanner.Ident && p.toks[j].Text == "from" {
			if j+1 < to && p.toks[j+1].Kind == scanner.String {
				fromModule = p.stringValue(j + 1)
			}
			break
		}
		j++
	}
	for j < to && !p.isPunct(j, ";") {
		j++
	}
	mod := resolveModule(fromModule, p.res.RelPath)
	if mod == "" {
		return j + 1
	}
	// second pass: names
	k := i + 1
	braces := 0
	for k < to && k < j {
		t := p.toks[k]
		if t.Kind == scanner.Punct {
			switch t.Text {
			case "{":
				braces++
			case "}":
				braces--
			}
			k++
			continue
		}
		if t.Kind != scanner.Ident {
			k++
			continue
		}
		if t.Text == "from" {
			break
		}
		switch t.Text {
		case "import", "type", "as", "default":
			k++
			continue
		}
		// default import: `import X from`
		if braces == 0 && p.isIdent(k) && (p.isPunct(k+1, ",") || (p.isIdent(k+1) && p.text(k+1) == "from")) {
			p.aliases[t.Text] = mod + "." + t.Text
			k++
			continue
		}
		if braces > 0 {
			// A, B as C
			local := t.Text
			orig := t.Text
			if p.isIdent(k+1) && p.text(k+1) == "as" && p.isIdent(k+2) {
				local = p.text(k + 2)
				k += 2
			}
			p.aliases[local] = mod + "." + orig
		}
		k++
	}
	return j + 1
}

// stringValue extracts the raw text inside a string token. The tokenizer drops
// string contents for most languages; TS module specifiers are recovered by
// scanning the source slice is not available here, so the token keeps its text
// only when the scanner preserved it.  Fallback: empty (name-based binding).
func (p *parser) stringValue(i int) string {
	return p.text(i)
}

func resolveModule(spec, relPath string) string {
	if spec == "" {
		return ""
	}
	if strings.HasPrefix(spec, ".") {
		base := path.Dir(path.Clean(relPath))
		joined := path.Clean(path.Join(base, spec))
		return moduleOf(joined)
	}
	// package import: keep the specifier
	return strings.ReplaceAll(strings.TrimPrefix(spec, "@"), "/", ".")
}

func (p *parser) parseClass(i, to int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "." + name
	line := p.tok(i).Line
	var bases []string
	x := j + 1
	if p.isPunct(x, "<") {
		// generic params: skip to matching '>' by depth
		depth := 0
		for x < to {
			if p.isPunct(x, "<") {
				depth++
			} else if p.isPunct(x, ">") {
				depth--
				if depth == 0 {
					x++
					break
				}
			}
			x++
		}
	}
	bodyOpen, bodyClose := -1, -1
	for x < to {
		if p.isIdent(x) && (p.text(x) == "extends" || p.text(x) == "implements") {
			kind := "inherits"
			if p.text(x) == "implements" {
				kind = "implements"
			}
			x++
			for x < to && !p.isPunct(x, "{") {
				if p.isIdent(x) {
					base := p.text(x)
					bases = append(bases, kind+"|"+base)
				}
				x++
			}
			continue
		}
		if p.isPunct(x, "{") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
		x++
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	key := p.key(qual, model.KindClass, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindClass, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, to), ParamCount: -1, Module: modPath,
	})
	for _, b := range bases {
		parts := strings.SplitN(b, "|", 2)
		p.addRef(key, parts[0], parts[1], p.aliases[parts[1]], line, modPath)
	}
	if bodyOpen >= 0 {
		p.parseClassBody(bodyOpen+1, bodyClose, modPath, qual, key)
		return bodyClose + 1
	}
	return x
}

func (p *parser) parseClassBody(from, to int, modPath, typeQual, typeKey string) {
	i := from
	for i < to {
		t := p.toks[i]
		if t.Kind == scanner.Punct {
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
		start := i
		for p.isIdent(i) && modifiers[p.text(i)] && !p.isPunct(i+1, "(") {
			i++
		}
		if i >= to {
			break
		}
		if !p.isIdent(i) {
			continue
		}
		name := p.text(i)
		if name == "constructor" || p.isPunct(i+1, "(") || p.isPunct(i+1, "<") {
			kind := model.KindMethod
			if name == "constructor" {
				kind = model.KindConstructor
			}
			qual := typeQual + "." + name
			line := p.tok(start).Line
			// find params and body/end
			k := i + 1
			if p.isPunct(k, "<") {
				depth := 0
				for k < to {
					if p.isPunct(k, "<") {
						depth++
					} else if p.isPunct(k, ">") {
						depth--
						if depth == 0 {
							k++
							break
						}
					}
					k++
				}
			}
			paramsOpen, paramsClose := -1, -1
			if p.isPunct(k, "(") {
				paramsOpen = k
				paramsClose = p.matchClose(k)
			}
			bodyOpen, bodyClose := -1, -1
			if paramsClose > 0 {
				for x := paramsClose + 1; x < to; x++ {
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
				Root: p.res.Root, Lang: "typescript", Kind: kind, Name: name, Qualname: qual,
				Namespace: modPath, StartLine: line, EndLine: endLine,
				Signature: p.signature(start, bodyOpen, to), ParamCount: p.paramCount(paramsOpen),
				Container: typeQual, Module: modPath,
			})
			if bodyOpen >= 0 {
				vars := p.collectVars(paramsOpen, paramsClose, bodyOpen, bodyClose)
				p.scanRefs(bodyOpen+1, bodyClose, key, modPath, typeQual, vars)
			}
			i = k
			if bodyClose > 0 {
				i = bodyClose + 1
			}
			continue
		}
		// property: name: T / name = expr / name?: T
		k := i + 1
		if p.isPunct(k, "?") || p.isPunct(k, "!") {
			k++
		}
		if p.isPunct(k, ":") && p.isIdent(k+1) {
			ptype := p.text(k + 1)
			qual := typeQual + "." + name
			line := p.tok(start).Line
			p.res.Symbols = append(p.res.Symbols, model.Symbol{
				Root: p.res.Root, Lang: "typescript", Kind: model.KindProperty, Name: name, Qualname: qual,
				Namespace: modPath, StartLine: line, EndLine: line, Signature: name + ": " + ptype,
				ParamCount: -1, Container: typeQual, Module: modPath,
			})
			p.addRef(typeKey, "references", ptype, p.aliases[ptype], line, modPath)
			for k < to && !p.isPunct(k, ";") {
				k++
			}
			i = k + 1
			continue
		}
		i = k
	}
}

func (p *parser) parseInterface(i, to int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "." + name
	line := p.tok(i).Line
	var bases []string
	x := j + 1
	bodyOpen, bodyClose := -1, -1
	for x < to {
		if p.isIdent(x) && p.text(x) == "extends" {
			x++
			for x < to && !p.isPunct(x, "{") {
				if p.isIdent(x) {
					bases = append(bases, p.text(x))
				}
				x++
			}
			continue
		}
		if p.isPunct(x, "{") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
		x++
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	key := p.key(qual, model.KindInterface, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindInterface, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, to), ParamCount: -1, Module: modPath,
	})
	for _, b := range bases {
		p.addRef(key, "inherits", b, p.aliases[b], line, modPath)
	}
	if bodyOpen >= 0 {
		// members: name(params): T / name: T
		for y := bodyOpen + 1; y < bodyClose; y++ {
			if !p.isIdent(y) {
				continue
			}
			mname := p.text(y)
			if p.isPunct(y+1, "(") {
				close := p.matchClose(y + 1)
				mqual := qual + "." + mname
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.res.Root, Lang: "typescript", Kind: model.KindMethod, Name: mname, Qualname: mqual,
					Namespace: modPath, StartLine: p.tok(y).Line, EndLine: p.tok(y).Line,
					Signature: mname, ParamCount: p.paramCount(y + 1), Container: qual,
					Module: modPath,
				})
				y = close
				continue
			}
			if p.isPunct(y+1, ":") && p.isIdent(y+2) {
				qual2 := qual + "." + mname
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.res.Root, Lang: "typescript", Kind: model.KindProperty, Name: mname, Qualname: qual2,
					Namespace: modPath, StartLine: p.tok(y).Line, EndLine: p.tok(y).Line,
					Signature: mname + ": " + p.text(y+2), ParamCount: -1, Container: qual, Module: modPath,
				})
				y += 2
			}
		}
		return bodyClose + 1
	}
	return x
}

func (p *parser) parseTypeAlias(i, to int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "." + name
	line := p.tok(i).Line
	end := j
	for end < to && !p.isPunct(end, ";") {
		end++
	}
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindType, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: p.tok(end).Line,
		Signature: p.signature(i, -1, to), ParamCount: -1, Module: modPath,
	})
	return end + 1
}

func (p *parser) parseEnum(i, to int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "." + name
	line := p.tok(i).Line
	bodyOpen, bodyClose := -1, -1
	for x := j + 1; x < to; x++ {
		if p.isPunct(x, "{") {
			bodyOpen = x
			bodyClose = p.matchClose(x)
			break
		}
	}
	endLine := line
	if bodyClose > 0 {
		endLine = p.tok(bodyClose).Line
	}
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindEnum, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, to), ParamCount: -1, Module: modPath,
	})
	if bodyOpen >= 0 {
		depth := 0
		for y := bodyOpen + 1; y < bodyClose; y++ {
			t := p.toks[y]
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
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.res.Root, Lang: "typescript", Kind: model.KindEnumMember, Name: t.Text,
					Qualname: qual + "." + t.Text, Namespace: modPath, StartLine: t.Line, EndLine: t.Line,
					Signature: t.Text, ParamCount: -1, Container: qual, Module: modPath,
				})
			}
		}
		return bodyClose + 1
	}
	return j
}

func (p *parser) parseFunction(i, to int, modPath string) int {
	j := i + 1
	if p.isPunct(j, "*") {
		j++
	}
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	qual := modPath + "." + name
	line := p.tok(i).Line
	k := j + 1
	if p.isPunct(k, "<") {
		depth := 0
		for k < to {
			if p.isPunct(k, "<") {
				depth++
			} else if p.isPunct(k, ">") {
				depth--
				if depth == 0 {
					k++
					break
				}
			}
			k++
		}
	}
	paramsOpen, paramsClose := -1, -1
	if p.isPunct(k, "(") {
		paramsOpen = k
		paramsClose = p.matchClose(k)
	}
	bodyOpen, bodyClose := -1, -1
	if paramsClose > 0 {
		for x := paramsClose + 1; x < to; x++ {
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
	key := p.key(qual, model.KindFunction, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindFunction, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: endLine,
		Signature: p.signature(i, bodyOpen, to), ParamCount: p.paramCount(paramsOpen), Module: modPath,
	})
	if bodyOpen >= 0 {
		vars := p.collectVars(paramsOpen, paramsClose, bodyOpen, bodyClose)
		p.scanRefs(bodyOpen+1, bodyClose, key, modPath, "", vars)
		return bodyClose + 1
	}
	return k
}

// parseVarDecl handles `const f = () => {}` / `const f = function() {}` and
// plain constants.
func (p *parser) parseVarDecl(i, to int, modPath string) int {
	j := i + 1
	if !p.isIdent(j) {
		return i + 1
	}
	name := p.text(j)
	line := p.tok(i).Line
	k := j + 1
	if p.isPunct(k, ":") {
		for k < to && !p.isPunct(k, "=") && !p.isPunct(k, ";") {
			k++
		}
	}
	if !p.isPunct(k, "=") {
		return k + 1
	}
	eq := k
	arrowBody := -1
	arrowClose := -1
	x := eq + 1
	if p.isIdent(x) && p.text(x) == "async" {
		x++
	}
	if p.isPunct(x, "(") {
		close := p.matchClose(x)
		y := close + 1
		if p.isPunct(y, "=>") {
			arrowBody = y
			arrowClose = p.matchClose(y+1)
			if p.isPunct(y+1, "{") {
				arrowClose = p.matchClose(y + 1)
			} else {
				arrowClose = y + 2
			}
		}
	} else if p.isIdent(x) && p.text(x) == "function" {
		arrowBody = x
		y := x + 1
		if p.isIdent(y) {
			y++
		}
		if p.isPunct(y, "(") {
			y = p.matchClose(y) + 1
		}
		if p.isPunct(y, "{") {
			arrowClose = p.matchClose(y)
		}
	}
	if arrowBody >= 0 && arrowClose > 0 {
		qual := modPath + "." + name
		key := p.key(qual, model.KindFunction, line)
		p.res.Symbols = append(p.res.Symbols, model.Symbol{
			Root: p.res.Root, Lang: "typescript", Kind: model.KindFunction, Name: name, Qualname: qual,
			Namespace: modPath, StartLine: line, EndLine: p.tok(arrowClose).Line,
			Signature: p.signature(i, arrowBody, to), ParamCount: -1, Module: modPath,
		})
		bodyFrom := arrowBody
		if p.isPunct(arrowBody, "=>") {
			bodyFrom = arrowBody + 1
		}
		p.scanRefs(bodyFrom, arrowClose, key, modPath, "", map[string]string{})
		return arrowClose + 1
	}
	// plain constant: scan initializer for calls/construction
	qual := modPath + "." + name
	key := p.key(qual, model.KindField, line)
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.res.Root, Lang: "typescript", Kind: model.KindField, Name: name, Qualname: qual,
		Namespace: modPath, StartLine: line, EndLine: p.tok(eq).Line,
		Signature: p.signature(i, eq, to), ParamCount: -1, Module: modPath,
	})
	stop := eq
	for stop < to && !p.isPunct(stop, ";") && !p.isPunct(stop, "}") {
		stop++
	}
	p.scanRefs(eq+1, stop, key, modPath, "", map[string]string{})
	return stop + 1
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
				var seg []string
				for x := segStart; x < j; x++ {
					if p.toks[x].Kind == scanner.Ident && !modifiers[p.toks[x].Text] {
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
	// const/let x: T / x = new T(...)
	for j := bodyOpen + 1; j+2 < bodyClose; j++ {
		if !p.isIdent(j) || (p.text(j) != "const" && p.text(j) != "let" && p.text(j) != "var") {
			continue
		}
		k := j + 1
		if !p.isIdent(k) {
			continue
		}
		name := p.text(k)
		if p.isPunct(k+1, ":") && p.isIdent(k+2) {
			vars[name] = p.text(k + 2)
			continue
		}
		if p.isPunct(k+1, "=") && p.isIdent(k+2) {
			if p.isIdent(k+2) && p.text(k+2) == "new" && p.isIdent(k+3) {
				vars[name] = p.text(k + 3)
			} else if p.isPunct(k+3, "(") {
				// factory call: keep the callee name as a weak type hint
				vars[name] = p.text(k + 2)
			}
		}
	}
	return vars
}

func (p *parser) scanRefs(from, to int, srcKey, modPath, ownerQual string, vars map[string]string) {
	for j := from; j < to; j++ {
		t := p.toks[j]
		if t.Kind != scanner.Ident {
			continue
		}
		next := j + 1
		if t.Text == "new" {
			if p.isIdent(next) {
				name := p.text(next)
				p.addRef(srcKey, "constructs", name, p.aliases[name], t.Line, modPath)
			}
			continue
		}
		if keywords[t.Text] || modifiers[t.Text] {
			continue
		}
		if p.isPunct(next, "(") {
			recv, recvType := p.receiverBefore(j, from, vars, ownerQual)
			p.addRef(srcKey, "calls", t.Text, recvType, t.Line, modPath)
			_ = recv
			continue
		}
		if (p.isPunct(next, ".") || p.isPunct(next, "?.")) && p.isIdent(next+1) {
			member := p.text(next + 1)
			if p.isPunct(next+2, "(") {
				// member call is captured by the member identifier branch below
				continue
			}
			recvType := vars[t.Text]
			if recvType == "" && ownerQual != "" {
				recvType = ownerQual
			}
			p.addRef(srcKey, "references", member, recvType, t.Line, modPath)
			continue
		}
		// member call: a.b(...)
		if j > from && (p.isPunct(j-1, ".") || p.isPunct(j-1, "?.")) && p.isPunct(next, "(") {
			recv := ""
			if j-2 >= from && p.isIdent(j-2) {
				recv = p.text(j - 2)
			}
			recvType := vars[recv]
			if recvType == "" {
				recvType = p.aliases[recv]
			}
			p.addRef(srcKey, "calls", t.Text, recvType, t.Line, modPath)
		}
	}
}

func (p *parser) receiverBefore(nameIdx, from int, vars map[string]string, ownerQual string) (string, string) {
	if nameIdx <= from || !(p.isPunct(nameIdx-1, ".") || p.isPunct(nameIdx-1, "?.")) {
		return "", ""
	}
	if nameIdx-2 >= from && p.isIdent(nameIdx-2) {
		recv := p.text(nameIdx - 2)
		if t, ok := vars[recv]; ok {
			return recv, t
		}
		if t, ok := p.aliases[recv]; ok {
			return recv, t
		}
		if c := recv[0]; c >= 'A' && c <= 'Z' {
			return recv, recv
		}
		return recv, ownerQual
	}
	return "", ""
}

func (p *parser) addRef(srcKey, kind, name, recvType string, line int, modPath string) {
	if name == "" {
		return
	}
	p.res.Refs = append(p.res.Refs, model.Ref{
		Root: p.res.Root, SrcKey: srcKey, Name: name, Kind: kind,
		ReceiverType: recvType, Line: line, NamespaceHint: modPath, Arity: -1,
	})
}
