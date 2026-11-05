package indexer

import (
	"strings"

	"github.com/neko233-com/codegraph233/internal/model"
	"github.com/neko233-com/codegraph233/internal/store"
)

// binder resolves harvested references against the global symbol table.
type binder struct {
	byQual          map[string][]store.SymbolRow
	byName          map[string][]store.SymbolRow
	byContainerName map[string][]store.SymbolRow
	byNamespaceName map[string][]store.SymbolRow
	typeBySimple    map[string][]store.SymbolRow
	packagePaths    map[string]bool
}

func newBinder(syms []store.SymbolRow) *binder {
	b := &binder{
		byQual:          map[string][]store.SymbolRow{},
		byName:          map[string][]store.SymbolRow{},
		byContainerName: map[string][]store.SymbolRow{},
		byNamespaceName: map[string][]store.SymbolRow{},
		typeBySimple:    map[string][]store.SymbolRow{},
		packagePaths:    map[string]bool{},
	}
	for _, s := range syms {
		b.byQual[s.Qualname] = append(b.byQual[s.Qualname], s)
		b.byName[s.Name] = append(b.byName[s.Name], s)
		if s.Container != "" {
			b.byContainerName[s.Container+"|"+s.Name] = append(b.byContainerName[s.Container+"|"+s.Name], s)
		}
		if s.Namespace != "" {
			b.byNamespaceName[s.Namespace+"|"+s.Name] = append(b.byNamespaceName[s.Namespace+"|"+s.Name], s)
			b.packagePaths[s.Namespace] = true
		}
		if model.IsTypeKind(s.Kind) {
			b.typeBySimple[s.Name] = append(b.typeBySimple[s.Name], s)
		}
	}
	return b
}

type resolution struct {
	target     store.SymbolRow
	confidence string
}

// resolve maps one reference to a symbol.  Ordering matters: typed receivers win
// over lexical scope, which wins over a global unique-name fallback.
func (b *binder) resolve(r store.LoadedRef, src store.SymbolRow) (resolution, bool) {
	if r.Name == "" {
		return resolution{}, false
	}
	wantCallable := r.Kind == "calls"
	wantType := r.Kind == "constructs" || r.Kind == "inherits" || r.Kind == "embeds"

	if r.ReceiverType != "" {
		if cand, ok := b.resolveReceiver(r, src, wantCallable, wantType); ok {
			return cand, true
		}
	}
	// same container (member of the declaring type)
	if src.Container != "" {
		if c := pick(b.byContainerName[src.Container+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "exact"}, true
		}
	}
	// same namespace / package
	if r.NamespaceHint != "" && r.NamespaceHint != src.Namespace {
		if c := pick(b.byNamespaceName[r.NamespaceHint+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "exact"}, true
		}
	}
	if src.Namespace != "" {
		if c := pick(b.byNamespaceName[src.Namespace+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "exact"}, true
		}
	}
	if wantType {
		if c := pick(b.typeBySimple[r.Name], r, src, false, true); c != nil {
			return resolution{*c, "name"}, true
		}
	}
	cands := b.byName[r.Name]
	if wantCallable {
		cands = filterCallable(cands)
	}
	if wantType {
		var types []store.SymbolRow
		for _, c := range cands {
			if model.IsTypeKind(c.Kind) {
				types = append(types, c)
			}
		}
		cands = types
	}
	if c := pick(cands, r, src, wantCallable, wantType); c != nil && len(cands) > 0 {
		conf := "name"
		if len(cands) > 1 {
			conf = "name-multi"
		}
		return resolution{*c, conf}, true
	}
	return resolution{}, false
}

func (b *binder) resolveReceiver(r store.LoadedRef, src store.SymbolRow, wantCallable, wantType bool) (resolution, bool) {
	rt := strings.TrimSpace(r.ReceiverType)
	rt = strings.TrimPrefix(rt, "*")
	if rt == "" {
		return resolution{}, false
	}
	// Go package path receiver: pkg.Func
	if strings.Contains(rt, "/") || b.packagePaths[rt] {
		if c := pick(b.byNamespaceName[rt+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "exact"}, true
		}
		return resolution{}, false
	}
	// qualified "pkg.Type" or "Namespace.Type"
	simple := rt
	if i := strings.LastIndex(rt, "."); i >= 0 {
		simple = rt[i+1:]
	}
	if simple == "" {
		return resolution{}, false
	}
	if b.packagePaths[rt] {
		if c := pick(b.byNamespaceName[rt+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "exact"}, true
		}
	}
	// type qualname may be namespace-prefixed; try exact then simple-name index
	typeCands := b.typeBySimple[simple]
	if len(typeCands) == 0 {
		if rows, ok := b.byQual[rt]; ok {
			typeCands = rows
		}
	}
	best := preferScoreList(typeCands, r, src)
	for _, t := range best {
		if c := pick(b.byContainerName[t.Qualname+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
			return resolution{*c, "type"}, true
		}
	}
	// static call on a type: Namespace.Type style
	if c := pick(b.byNamespaceName[rt+"|"+r.Name], r, src, wantCallable, wantType); c != nil {
		return resolution{*c, "type"}, true
	}
	return resolution{}, false
}

func filterCallable(cands []store.SymbolRow) []store.SymbolRow {
	var out []store.SymbolRow
	for _, c := range cands {
		switch c.Kind {
		case model.KindMethod, model.KindFunction, model.KindConstructor, model.KindOperator:
			out = append(out, c)
		}
	}
	return out
}

// pick selects the best candidate honoring root, language, arity and kind.
func pick(cands []store.SymbolRow, r store.LoadedRef, src store.SymbolRow, wantCallable, wantType bool) *store.SymbolRow {
	if len(cands) == 0 {
		return nil
	}
	var filtered []store.SymbolRow
	for _, c := range cands {
		if src.Root != "" && c.Root != "" && c.Root != src.Root {
			continue
		}
		if src.Lang != "" && c.Lang != "" && c.Lang != src.Lang {
			continue
		}
		if wantCallable {
			switch c.Kind {
			case model.KindMethod, model.KindFunction, model.KindConstructor, model.KindOperator:
			default:
				continue
			}
			if r.Arity >= 0 && c.ParamCount >= 0 && r.Arity != c.ParamCount {
				continue
			}
		}
		if wantType && !model.IsTypeKind(c.Kind) {
			continue
		}
		filtered = append(filtered, c)
	}
	if len(filtered) == 0 {
		filtered = cands
	}
	best := filtered[0]
	bestScore := score(best, r, src)
	for _, c := range filtered[1:] {
		s := score(c, r, src)
		if s > bestScore {
			best, bestScore = c, s
		}
	}
	// arity mismatch on every candidate is still better than nothing, but mark
	// it by preferring an exact arity when one exists.
	return &best
}

func score(c store.SymbolRow, r store.LoadedRef, src store.SymbolRow) int {
	s := 0
	if src.Root != "" && c.Root == src.Root {
		s += 8
	}
	if src.Lang != "" && c.Lang == src.Lang {
		s += 4
	}
	if src.Namespace != "" && c.Namespace == src.Namespace {
		s += 4
	}
	if src.Container != "" && c.Container == src.Container {
		s += 6
	}
	if r.Arity >= 0 && c.ParamCount >= 0 {
		if r.Arity == c.ParamCount {
			s += 6
		} else {
			s -= 3
		}
	}
	if strings.HasSuffix(c.Qualname, "."+r.Name) {
		s++
	}
	return s
}

func preferScoreList(cands []store.SymbolRow, r store.LoadedRef, src store.SymbolRow) []store.SymbolRow {
	if len(cands) <= 1 {
		return cands
	}
	out := append([]store.SymbolRow{}, cands...)
	for i := 1; i < len(out); i++ {
		for j := i; j > 0 && score(out[j], r, src) > score(out[j-1], r, src); j-- {
			out[j], out[j-1] = out[j-1], out[j]
		}
	}
	return out
}
