// Package golang builds the graph from Go sources using the standard library
// AST (no cgo, no external parser).  Cross-package targets are resolved later by
// the binder through package import paths recorded here.
package golang

import (
	"go/ast"
	"go/parser"
	"go/token"
	"strings"

	"github.com/neko233-com/game-project-graph/internal/model"
)

type fileParser struct {
	fset       *token.FileSet
	src        []byte
	root       string
	relPath    string
	pkgPath    string
	pkgName    string
	imports    map[string]string // alias -> path
	fieldTypes map[string]string // "Type.field" -> type text
	funcRet    map[string]string // funcName -> return type text
	res        *model.FileResult
	keySeq     int
}

// Parse extracts Go declarations and references from one file.
func Parse(relPath, root, modulePath, relDir string, src []byte) *model.FileResult {
	pkgPath := modulePath
	if relDir != "" && relDir != "." {
		pkgPath = strings.TrimSuffix(modulePath, "/") + "/" + relDir
	}
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "go", Module: pkgPath}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, relPath, src, parser.SkipObjectResolution)
	if err != nil && file == nil {
		res.Errors = append(res.Errors, err.Error())
		return res
	}
	p := &fileParser{
		fset: fset, src: src, root: root, relPath: relPath, pkgPath: pkgPath,
		imports: map[string]string{}, fieldTypes: map[string]string{},
		funcRet: map[string]string{}, res: res,
	}
	p.pkgName = file.Name.Name
	p.collectImports(file)
	for _, d := range file.Decls {
		p.collectDecl(d)
	}
	for _, d := range file.Decls {
		p.collectRefs(d)
	}
	return res
}

func (p *fileParser) key(qual, kind string, line int) string {
	p.keySeq++
	return qual + "#" + kind + "@" + itoa(line) + ":" + itoa(p.keySeq)
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

func (p *fileParser) line(pos token.Pos) int {
	return p.fset.Position(pos).Line
}

func (p *fileParser) text(from, to token.Pos) string {
	f := p.fset.Position(from).Offset
	t := p.fset.Position(to).Offset
	if f < 0 || t > len(p.src) || f >= t {
		return ""
	}
	s := string(p.src[f:t])
	s = strings.Join(strings.Fields(s), " ")
	if len(s) > 400 {
		s = s[:400]
	}
	return s
}

func (p *fileParser) collectImports(f *ast.File) {
	for _, imp := range f.Imports {
		path := strings.Trim(imp.Path.Value, `"`)
		alias := path
		if i := strings.LastIndex(path, "/"); i >= 0 {
			alias = path[i+1:]
		}
		if imp.Name != nil {
			alias = imp.Name.Name
		}
		if alias == "_" || alias == "." {
			continue
		}
		p.imports[alias] = path
	}
}

func (p *fileParser) collectDecl(d ast.Decl) {
	switch decl := d.(type) {
	case *ast.GenDecl:
		for _, spec := range decl.Specs {
			switch s := spec.(type) {
			case *ast.TypeSpec:
				p.collectTypeSpec(s)
			case *ast.ValueSpec:
				p.collectValueSpec(s)
			}
		}
	case *ast.FuncDecl:
		p.collectFunc(decl)
	}
}

func (p *fileParser) collectTypeSpec(s *ast.TypeSpec) {
	kind := model.KindType
	switch s.Type.(type) {
	case *ast.StructType:
		kind = model.KindStruct
	case *ast.InterfaceType:
		kind = model.KindInterface
	}
	name := s.Name.Name
	qual := p.pkgPath + "." + name
	line := p.line(s.Pos())
	key := p.key(qual, kind, line)
	endLine := p.line(s.End())
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.root, Lang: "go", Kind: kind, Name: name, Qualname: qual, Namespace: p.pkgPath,
		StartLine: line, EndLine: endLine, Signature: p.text(s.Pos(), s.End()),
		ParamCount: -1, Module: p.pkgPath,
	})
	switch t := s.Type.(type) {
	case *ast.StructType:
		for _, f := range t.Fields.List {
			if f.Type == nil {
				continue
			}
			typeText := p.text(f.Type.Pos(), f.Type.End())
			if len(f.Names) == 0 {
				// embedded: composition edge
				p.res.Refs = append(p.res.Refs, model.Ref{
					Root: p.root, SrcKey: key, Name: lastTypeName(typeText), Kind: "embeds",
					Receiver: typeText, ReceiverType: p.resolveAlias(typeText),
					Line: p.line(f.Pos()), NamespaceHint: p.pkgPath, Arity: -1,
				})
				continue
			}
			for _, n := range f.Names {
				fq := qual + "." + n.Name
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.root, Lang: "go", Kind: model.KindField, Name: n.Name, Qualname: fq,
					Namespace: p.pkgPath, StartLine: p.line(n.Pos()), EndLine: p.line(f.End()),
					Signature: typeText, ParamCount: -1, Container: qual, Module: p.pkgPath,
				})
				p.fieldTypes[qual+"."+n.Name] = typeText
			}
		}
	case *ast.InterfaceType:
		for _, m := range t.Methods.List {
			fn, ok := m.Type.(*ast.FuncType)
			if !ok || len(m.Names) == 0 {
				continue
			}
			for _, n := range m.Names {
				fq := qual + "." + n.Name
				p.res.Symbols = append(p.res.Symbols, model.Symbol{
					Root: p.root, Lang: "go", Kind: model.KindMethod, Name: n.Name, Qualname: fq,
					Namespace: p.pkgPath, StartLine: p.line(n.Pos()), EndLine: p.line(m.End()),
					Signature: p.text(m.Pos(), m.End()), ParamCount: paramCount(fn.Params),
					Container: qual, Module: p.pkgPath,
				})
			}
		}
	}
}

func (p *fileParser) collectValueSpec(s *ast.ValueSpec) {
	typeText := ""
	if s.Type != nil {
		typeText = p.text(s.Type.Pos(), s.Type.End())
	}
	kind := model.KindField
	for _, n := range s.Names {
		qual := p.pkgPath + "." + n.Name
		p.res.Symbols = append(p.res.Symbols, model.Symbol{
			Root: p.root, Lang: "go", Kind: kind, Name: n.Name, Qualname: qual,
			Namespace: p.pkgPath, StartLine: p.line(n.Pos()), EndLine: p.line(s.End()),
			Signature: typeText, ParamCount: -1, Module: p.pkgPath,
		})
	}
}

func (p *fileParser) collectFunc(d *ast.FuncDecl) {
	if d.Name == nil {
		return
	}
	recv := receiverName(d)
	name := d.Name.Name
	qual := p.pkgPath + "." + name
	kind := model.KindFunction
	container := ""
	if recv != "" {
		qual = p.pkgPath + "." + recv + "." + name
		kind = model.KindMethod
		container = p.pkgPath + "." + recv
	}
	line := p.line(d.Pos())
	sig := ""
	if d.Body != nil {
		sig = p.text(d.Pos(), d.Body.Pos())
	} else {
		sig = p.text(d.Pos(), d.End())
	}
	pc := -1
	if d.Type != nil && d.Type.Params != nil {
		pc = paramCount(d.Type.Params)
	}
	p.res.Symbols = append(p.res.Symbols, model.Symbol{
		Root: p.root, Lang: "go", Kind: kind, Name: name, Qualname: qual, Namespace: p.pkgPath,
		StartLine: line, EndLine: p.line(d.End()), Signature: sig, ParamCount: pc,
		Container: container, Module: p.pkgPath,
	})
	if d.Recv != nil && len(d.Recv.List) > 0 {
		if t := p.text(d.Recv.List[0].Type.Pos(), d.Recv.List[0].Type.End()); t != "" {
			// receiver variable name -> receiver type for scans
			if len(d.Recv.List[0].Names) > 0 {
				p.fieldTypes["$recv$"+d.Recv.List[0].Names[0].Name] = t
			}
		}
	}
	if kind == model.KindFunction && d.Type != nil && d.Type.Results != nil && len(d.Type.Results.List) > 0 {
		ret := p.text(d.Type.Results.List[0].Type.Pos(), d.Type.Results.List[0].Type.End())
		if ret != "" {
			p.funcRet[name] = ret
		}
	}
}

func receiverName(d *ast.FuncDecl) string {
	if d.Recv == nil {
		return ""
	}
	for _, f := range d.Recv.List {
		name := p_typeName(f.Type)
		if name != "" {
			return name
		}
	}
	return ""
}

func p_typeName(e ast.Expr) string {
	switch t := e.(type) {
	case *ast.StarExpr:
		return p_typeName(t.X)
	case *ast.Ident:
		return t.Name
	case *ast.IndexExpr:
		return p_typeName(t.X)
	case *ast.IndexListExpr:
		return p_typeName(t.X)
	case *ast.SelectorExpr:
		return t.Sel.Name
	}
	return ""
}

func paramCount(fl *ast.FieldList) int {
	if fl == nil {
		return 0
	}
	n := 0
	for _, f := range fl.List {
		if len(f.Names) == 0 {
			n++
		} else {
			n += len(f.Names)
		}
	}
	return n
}

func lastTypeName(s string) string {
	s = strings.TrimSpace(s)
	s = strings.TrimPrefix(s, "*")
	if i := strings.Index(s, "["); i >= 0 {
		s = s[:i]
	}
	if i := strings.LastIndex(s, "."); i >= 0 {
		s = s[i+1:]
	}
	return s
}

func (p *fileParser) resolveAlias(typeText string) string {
	head, _, found := strings.Cut(typeText, ".")
	if found {
		if path, ok := p.imports[head]; ok {
			return path
		}
	}
	return typeText
}

// collectRefs walks declarations with a scope of parameter/var types.
func (p *fileParser) collectRefs(d ast.Decl) {
	switch decl := d.(type) {
	case *ast.FuncDecl:
		if decl.Body == nil {
			return
		}
		recv := receiverName(decl)
		qual := p.pkgPath + "." + decl.Name.Name
		kind := model.KindFunction
		if recv != "" {
			qual = p.pkgPath + "." + recv + "." + decl.Name.Name
			kind = model.KindMethod
		}
		srcKey := qual + "#" + kind + "@" + itoa(p.line(decl.Pos()))
		scope := map[string]string{}
		if decl.Recv != nil {
			for _, f := range decl.Recv.List {
				t := p.text(f.Type.Pos(), f.Type.End())
				for _, n := range f.Names {
					scope[n.Name] = t
				}
			}
		}
		addFields(scope, decl.Type.Params, p)
		collectLocalTypes(decl.Body, scope, p)
		p.walkBody(decl.Body, srcKey, descope(qual), scope)
	case *ast.GenDecl:
		for _, spec := range decl.Specs {
			if vs, ok := spec.(*ast.ValueSpec); ok {
				for _, v := range vs.Values {
					p.walkExpr(v, p.pkgPath, p.pkgPath, map[string]string{})
				}
			}
		}
	}
}

func descope(qual string) string {
	// qual = pkgPath.Type.Method -> pkgPath.Type
	if i := strings.LastIndex(qual, "."); i > 0 {
		return qual[:i]
	}
	return qual
}

func addFields(scope map[string]string, fl *ast.FieldList, p *fileParser) {
	if fl == nil {
		return
	}
	for _, f := range fl.List {
		if f.Type == nil {
			continue
		}
		t := p.text(f.Type.Pos(), f.Type.End())
		for _, n := range f.Names {
			scope[n.Name] = t
		}
	}
}

func collectLocalTypes(body *ast.BlockStmt, scope map[string]string, p *fileParser) {
	ast.Inspect(body, func(n ast.Node) bool {
		switch s := n.(type) {
		case *ast.AssignStmt:
			if s.Tok.String() != ":=" && s.Tok.String() != "=" {
				return true
			}
			for i, lhs := range s.Lhs {
				id, ok := lhs.(*ast.Ident)
				if !ok || i >= len(s.Rhs) {
					continue
				}
				if t := inferExprType(s.Rhs[i], p.funcRet); t != "" {
					scope[id.Name] = t
				}
			}
		case *ast.DeclStmt:
			gd, ok := s.Decl.(*ast.GenDecl)
			if !ok {
				return true
			}
			for _, spec := range gd.Specs {
				vs, ok := spec.(*ast.ValueSpec)
				if !ok {
					continue
				}
				t := ""
				if vs.Type != nil {
					t = p.text(vs.Type.Pos(), vs.Type.End())
				}
				for i, n := range vs.Names {
					if t != "" {
						scope[n.Name] = t
					} else if i < len(vs.Values) {
						if it := inferExprType(vs.Values[i], p.funcRet); it != "" {
							scope[n.Name] = it
						}
					}
				}
			}
		}
		return true
	})
}

func inferExprType(e ast.Expr, funcRet map[string]string) string {
	switch t := e.(type) {
	case *ast.CompositeLit:
		return typeExprName(t.Type)
	case *ast.UnaryExpr:
		return inferExprType(t.X, funcRet)
	case *ast.CallExpr:
		if id, ok := t.Fun.(*ast.Ident); ok {
			return funcRet[id.Name]
		}
	case *ast.TypeAssertExpr:
		return typeExprName(t.Type)
	case *ast.ParenExpr:
		return inferExprType(t.X, funcRet)
	}
	return ""
}

func typeExprName(e ast.Expr) string {
	switch t := e.(type) {
	case *ast.Ident:
		return t.Name
	case *ast.StarExpr:
		return typeExprName(t.X)
	case *ast.SelectorExpr:
		if x, ok := t.X.(*ast.Ident); ok {
			return x.Name + "." + t.Sel.Name
		}
		return t.Sel.Name
	case *ast.ArrayType:
		return typeExprName(t.Elt)
	}
	return ""
}

func (p *fileParser) walkBody(body *ast.BlockStmt, srcKey, ownerQual string, scope map[string]string) {
	ast.Inspect(body, func(n ast.Node) bool {
		switch e := n.(type) {
		case *ast.CallExpr:
			p.addCall(e, srcKey, ownerQual, scope)
			return true
		case *ast.CompositeLit:
			if t := typeExprName(e.Type); t != "" {
				p.addRef(srcKey, "", "constructs", lastTypeName(t), t, p.resolveAlias(t), p.line(e.Pos()))
			}
		case *ast.SelectorExpr:
			if x, ok := e.X.(*ast.Ident); ok {
				p.addRef(srcKey, x.Name, "references", e.Sel.Name, "", p.inferReceiver(x.Name, scope, ownerQual), p.line(e.Pos()))
			}
		}
		return true
	})
}

func (p *fileParser) walkExpr(e ast.Expr, srcKey, ownerQual string, scope map[string]string) {
	if e == nil {
		return
	}
	ast.Inspect(e, func(n ast.Node) bool {
		switch x := n.(type) {
		case *ast.CallExpr:
			p.addCall(x, srcKey, ownerQual, scope)
		case *ast.CompositeLit:
			if t := typeExprName(x.Type); t != "" {
				p.addRef(srcKey, "", "constructs", lastTypeName(t), t, p.resolveAlias(t), p.line(x.Pos()))
			}
		}
		return true
	})
}

func (p *fileParser) addCall(call *ast.CallExpr, srcKey, ownerQual string, scope map[string]string) {
	arity := len(call.Args)
	switch fn := call.Fun.(type) {
	case *ast.Ident:
		p.addRef(srcKey, "", "calls", fn.Name, "", "", p.line(call.Pos()))
		p.res.Refs[len(p.res.Refs)-1].Arity = arity
	case *ast.SelectorExpr:
		recvType := ""
		recv := ""
		switch x := fn.X.(type) {
		case *ast.Ident:
			recv = x.Name
			recvType = p.inferReceiver(x.Name, scope, ownerQual)
		case *ast.CallExpr:
			if id, ok := x.Fun.(*ast.Ident); ok {
				recv = id.Name
			}
		}
		p.addRef(srcKey, recv, "calls", fn.Sel.Name, recv, recvType, p.line(call.Pos()))
		p.res.Refs[len(p.res.Refs)-1].Arity = arity
	}
}

func (p *fileParser) inferReceiver(name string, scope map[string]string, ownerQual string) string {
	if path, ok := p.imports[name]; ok {
		return path
	}
	if t, ok := scope[name]; ok {
		return strings.TrimPrefix(strings.TrimSpace(t), "*")
	}
	if ownerQual != "" {
		if t, ok := p.fieldTypes[ownerQual+"."+name]; ok {
			return strings.TrimPrefix(strings.TrimSpace(t), "*")
		}
	}
	if name == "" {
		return ""
	}
	if c := name[0]; c >= 'A' && c <= 'Z' {
		return name
	}
	return ""
}

func (p *fileParser) addRef(srcKey, receiver, kind, name, receiverText, recvType string, line int) {
	if name == "" {
		return
	}
	p.res.Refs = append(p.res.Refs, model.Ref{
		Root: p.root, SrcKey: srcKey, Name: name, Receiver: receiver,
		ReceiverType: recvType, Kind: kind, Line: line, NamespaceHint: p.pkgPath, Arity: -1,
	})
}
