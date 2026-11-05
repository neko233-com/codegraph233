// Package query exposes read-only graph queries used by the MCP tools and CLI.
package query

import (
	"bufio"
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"github.com/neko233-com/game-project-graph/internal/config"
	"github.com/neko233-com/game-project-graph/internal/detect"
	"github.com/neko233-com/game-project-graph/internal/store"
)

// Engine holds an open index for querying.
type Engine struct {
	Cfg *config.Config
	St  *store.Store
}

// Open opens the index read-only.
func Open(cfg *config.Config) (*Engine, error) {
	if _, err := os.Stat(cfg.DBPath()); err != nil {
		return nil, fmt.Errorf("索引不存在，请先运行 `game-project-graph index`：%w", err)
	}
	st, err := store.Open(cfg.DBPath())
	if err != nil {
		return nil, err
	}
	return &Engine{Cfg: cfg, St: st}, nil
}

// Close closes the index.
func (e *Engine) Close() error { return e.St.Close() }

// SymbolHit is the compact symbol representation returned to agents.
type SymbolHit struct {
	ID        int64  `json:"id"`
	Kind      string `json:"kind"`
	Name      string `json:"name"`
	Qualname  string `json:"qualname"`
	Namespace string `json:"namespace,omitempty"`
	Lang      string `json:"lang"`
	Root      string `json:"root"`
	Path      string `json:"path"`
	StartLine int    `json:"start_line"`
	EndLine   int    `json:"end_line"`
	Signature string `json:"signature,omitempty"`
	Container string `json:"container,omitempty"`
	Param     int    `json:"param_count"`
}

const symbolSelect = `SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
	COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''), COALESCE(s.container,''),
	COALESCE(s.param_count,-1)
	FROM symbols s LEFT JOIN files f ON f.id = s.file_id`

func scanSymbol(rows *sql.Rows) (SymbolHit, error) {
	var h SymbolHit
	err := rows.Scan(&h.ID, &h.Kind, &h.Name, &h.Qualname, &h.Namespace, &h.Lang, &h.Root,
		&h.Path, &h.StartLine, &h.EndLine, &h.Signature, &h.Container, &h.Param)
	return h, err
}

// SearchSymbols finds declarations by name, qualname or substring.
func (e *Engine) SearchSymbols(pattern string, kinds []string, lang, root, pathFilter string, limit int) ([]SymbolHit, error) {
	if limit <= 0 {
		limit = 30
	}
	pat := "%" + pattern + "%"
	q := symbolSelect + ` WHERE (s.name LIKE ? OR s.qualname LIKE ?)`
	args := []any{pat, pat}
	if len(kinds) > 0 {
		q += ` AND s.kind IN (` + placeholders(len(kinds)) + `)`
		for _, k := range kinds {
			args = append(args, k)
		}
	}
	if lang != "" {
		q += ` AND s.lang = ?`
		args = append(args, lang)
	}
	if root != "" {
		q += ` AND s.root = ?`
		args = append(args, root)
	}
	if pathFilter != "" {
		q += ` AND f.path LIKE ?`
		args = append(args, "%"+pathFilter+"%")
	}
	q += ` ORDER BY CASE WHEN s.name = ? THEN 0 WHEN s.name LIKE ? THEN 1 ELSE 2 END,
		LENGTH(s.qualname), s.qualname LIMIT ?`
	args = append(args, pattern, pattern+"%", limit)
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []SymbolHit
	for rows.Next() {
		h, err := scanSymbol(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

// GetSymbol loads one symbol by id.
func (e *Engine) GetSymbol(id int64) (*SymbolHit, error) {
	rows, err := e.St.DB().Query(symbolSelect+` WHERE s.id = ?`, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	if !rows.Next() {
		return nil, fmt.Errorf("symbol %d not found", id)
	}
	h, err := scanSymbol(rows)
	if err != nil {
		return nil, err
	}
	return &h, nil
}

// Source returns the numbered source lines for a symbol (or file range).
func (e *Engine) Source(sym SymbolHit, maxLines int) (string, error) {
	return e.FileRange(sym.Root, sym.Path, sym.StartLine, sym.EndLine, maxLines)
}

// FileRange reads root-relative file lines [start, end] with line numbers.
func (e *Engine) FileRange(root, relPath string, start, end, maxLines int) (string, error) {
	abs := e.absOf(root, relPath)
	f, err := os.Open(abs)
	if err != nil {
		return "", err
	}
	defer f.Close()
	if maxLines <= 0 {
		maxLines = 200
	}
	if end < start {
		end = start
	}
	if end-start+1 > maxLines {
		end = start + maxLines - 1
	}
	var b strings.Builder
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 64*1024), 2*1024*1024)
	line := 0
	for sc.Scan() {
		line++
		if line < start {
			continue
		}
		if line > end {
			break
		}
		fmt.Fprintf(&b, "%5d| %s\n", line, sc.Text())
	}
	return b.String(), sc.Err()
}

func (e *Engine) absOf(root, relPath string) string {
	if filepath.IsAbs(relPath) {
		return relPath
	}
	for _, r := range e.Cfg.Roots {
		if r.Key == root {
			return filepath.Join(e.Cfg.AbsPath(r), filepath.FromSlash(relPath))
		}
	}
	return filepath.Join(e.Cfg.Dir, filepath.FromSlash(relPath))
}

// FileOutline returns the symbol list of a file ordered by position.
func (e *Engine) FileOutline(pathFilter, root string, limit int) ([]SymbolHit, error) {
	if limit <= 0 {
		limit = 400
	}
	q := symbolSelect + ` WHERE f.path LIKE ?`
	args := []any{"%" + pathFilter + "%"}
	if root != "" {
		q += ` AND s.root = ?`
		args = append(args, root)
	}
	q += ` ORDER BY f.path, s.start_line LIMIT ?`
	args = append(args, limit)
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []SymbolHit
	for rows.Next() {
		h, err := scanSymbol(rows)
		if err != nil {
			return nil, err
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

// EdgeHit is a symbol connected to the subject.
type EdgeHit struct {
	SymbolHit
	EdgeKind   string `json:"edge_kind"`
	Line       int    `json:"line"`
	Confidence string `json:"confidence,omitempty"`
	Depth      int    `json:"depth,omitempty"`
}

// Related returns callers (direction=in) or callees (out) up to depth levels.
func (e *Engine) Related(id int64, direction string, kinds []string, depth int, limit int) ([]EdgeHit, error) {
	if depth <= 0 {
		depth = 1
	}
	if depth > 4 {
		depth = 4
	}
	if limit <= 0 {
		limit = 100
	}
	kindFilter := ""
	var kindArgs []any
	if len(kinds) > 0 {
		kindFilter = ` AND e.kind IN (` + placeholders(len(kinds)) + `)`
		for _, k := range kinds {
			kindArgs = append(kindArgs, k)
		}
	}
	seen := map[int64]bool{id: true}
	current := []int64{id}
	var out []EdgeHit
	for d := 1; d <= depth; d++ {
		var next []int64
		for _, cur := range current {
			var q string
			if direction == "in" {
				q = `SELECT ` + symbolColumns("s") + `, e.kind, COALESCE(e.line,0), COALESCE(e.confidence,'')
					FROM edges e JOIN symbols s ON s.id = e.src
					LEFT JOIN files f ON f.id = s.file_id
					WHERE e.dst = ?` + kindFilter + ` LIMIT ?`
			} else {
				q = `SELECT ` + symbolColumns("s") + `, e.kind, COALESCE(e.line,0), COALESCE(e.confidence,'')
					FROM edges e JOIN symbols s ON s.id = e.dst
					LEFT JOIN files f ON f.id = s.file_id
					WHERE e.src = ?` + kindFilter + ` LIMIT ?`
			}
			args := []any{cur}
			args = append(args, kindArgs...)
			args = append(args, limit)
			rows, err := e.St.DB().Query(q, args...)
			if err != nil {
				return nil, err
			}
			for rows.Next() {
				var h EdgeHit
				var sig, container, ns, path string
				if err := rows.Scan(&h.ID, &h.Kind, &h.Name, &h.Qualname, &ns, &h.Lang, &h.Root,
					&path, &h.StartLine, &h.EndLine, &sig, &container, &h.Param,
					&h.EdgeKind, &h.Line, &h.Confidence); err != nil {
					rows.Close()
					return nil, err
				}
				h.Namespace, h.Path, h.Signature, h.Container = ns, path, sig, container
				h.Depth = d
				if !seen[h.ID] {
					seen[h.ID] = true
					next = append(next, h.ID)
					out = append(out, h)
				}
				if len(out) >= limit {
					rows.Close()
					return out, nil
				}
			}
			rows.Close()
		}
		if len(next) == 0 {
			break
		}
		current = next
	}
	return out, nil
}

func symbolColumns(alias string) string {
	p := alias + "."
	return p + `id, ` + p + `kind, ` + p + `name, ` + p + `qualname, COALESCE(` + p + `namespace,''),
		` + p + `lang, ` + p + `root, COALESCE(f.path,''), ` + p + `start_line, ` + p + `end_line,
		COALESCE(` + p + `signature,''), COALESCE(` + p + `container,''), COALESCE(` + p + `param_count,-1)`
}

// RefsTo lists references that resolved to a symbol.
func (e *Engine) RefsTo(id int64, limit int) ([]EdgeHit, error) {
	if limit <= 0 {
		limit = 100
	}
	q := `SELECT ` + symbolColumns("s") + `, e.kind, COALESCE(e.line,0), COALESCE(e.confidence,'')
		FROM edges e JOIN symbols s ON s.id = e.src LEFT JOIN files f ON f.id = s.file_id
		WHERE e.dst = ? ORDER BY e.line LIMIT ?`
	rows, err := e.St.DB().Query(q, id, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []EdgeHit
	for rows.Next() {
		var h EdgeHit
		if err := rows.Scan(&h.ID, &h.Kind, &h.Name, &h.Qualname, &h.Namespace, &h.Lang, &h.Root,
			&h.Path, &h.StartLine, &h.EndLine, &h.Signature, &h.Container, &h.Param,
			&h.EdgeKind, &h.Line, &h.Confidence); err != nil {
			return nil, err
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

// PathStep is one hop in a call path.
type PathStep struct {
	From     int64  `json:"from"`
	To       int64  `json:"to"`
	Kind     string `json:"kind"`
	FromName string `json:"from_name"`
	ToName   string `json:"to_name"`
}

// CallPath finds a shortest directed path between two symbols using calls edges.
func (e *Engine) CallPath(from, to int64, maxDepth int) ([]PathStep, error) {
	if maxDepth <= 0 {
		maxDepth = 8
	}
	names, err := e.symbolNames([]int64{from, to})
	if err != nil {
		return nil, err
	}
	prev := map[int64]PathStep{}
	visited := map[int64]bool{from: true}
	queue := []int64{from}
	for depth := 0; depth < maxDepth && len(queue) > 0; depth++ {
		var next []int64
		for _, cur := range queue {
			rows, err := e.St.DB().Query(`SELECT dst, kind FROM edges WHERE src = ? AND kind IN ('calls','constructs')`, cur)
			if err != nil {
				return nil, err
			}
			for rows.Next() {
				var dst int64
				var kind string
				if err := rows.Scan(&dst, &kind); err != nil {
					rows.Close()
					return nil, err
				}
				if visited[dst] {
					continue
				}
				visited[dst] = true
				prev[dst] = PathStep{From: cur, To: dst, Kind: kind}
				if dst == to {
					rows.Close()
					return reconstruct(prev, from, to, names), nil
				}
				next = append(next, dst)
			}
			rows.Close()
		}
		queue = next
	}
	return nil, fmt.Errorf("no call path found from %d to %d within %d hops", from, to, maxDepth)
}

func reconstruct(prev map[int64]PathStep, from, to int64, names map[int64]string) []PathStep {
	var rev []PathStep
	cur := to
	for cur != from {
		step, ok := prev[cur]
		if !ok {
			return nil
		}
		step.FromName = names[step.From]
		step.ToName = names[step.To]
		rev = append(rev, step)
		cur = step.From
	}
	out := make([]PathStep, len(rev))
	for i := range rev {
		out[len(rev)-1-i] = rev[i]
	}
	return out
}

func (e *Engine) symbolNames(ids []int64) (map[int64]string, error) {
	out := map[int64]string{}
	if len(ids) == 0 {
		return out, nil
	}
	q := `SELECT id, qualname FROM symbols WHERE id IN (` + placeholders(len(ids)) + `)`
	args := make([]any, len(ids))
	for i, id := range ids {
		args[i] = id
	}
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var id int64
		var qn string
		if err := rows.Scan(&id, &qn); err == nil {
			out[id] = qn
		}
	}
	return out, rows.Err()
}

// Impact reports symbols affected by changing a file or symbol.
type Impact struct {
	Subject   string     `json:"subject"`
	Direct    []EdgeHit  `json:"direct"`
	Transitive []EdgeHit `json:"transitive,omitempty"`
	Files     []string   `json:"affected_files"`
}

// AnalyzeImpact resolves the impact set for a path or symbol id.
func (e *Engine) AnalyzeImpact(symbolID int64, pathFilter string, depth int) (*Impact, error) {
	if symbolID == 0 && pathFilter != "" {
		hits, err := e.FileOutline(pathFilter, "", 2000)
		if err != nil {
			return nil, err
		}
		if len(hits) > 0 {
			symbolID = hits[0].ID
		}
	}
	if symbolID == 0 {
		return nil, fmt.Errorf("impact: need a symbol id or an existing path")
	}
	direct, err := e.Related(symbolID, "in", nil, 1, 500)
	if err != nil {
		return nil, err
	}
	trans, _ := e.Related(symbolID, "in", nil, depth, 1000)
	fileSet := map[string]bool{}
	for _, h := range trans {
		if h.Path != "" {
			fileSet[h.Path] = true
		}
	}
	var files []string
	for f := range fileSet {
		files = append(files, f)
	}
	sort.Strings(files)
	subj := ""
	if h, err := e.GetSymbol(symbolID); err == nil {
		subj = h.Qualname
	} else {
		subj = pathFilter
	}
	return &Impact{Subject: subj, Direct: direct, Transitive: trans, Files: files}, nil
}

// PatternHit is one grep-style match with context.
type PatternHit struct {
	Root    string   `json:"root"`
	Path    string   `json:"path"`
	Line    int      `json:"line"`
	Text    string   `json:"text"`
	Context []string `json:"context,omitempty"`
}

// SearchPattern greps indexed files (optionally restricted by root/glob).
func (e *Engine) SearchPattern(pattern, glob, root string, maxResults, contextLines int) ([]PatternHit, error) {
	re, err := regexp.Compile(pattern)
	if err != nil {
		return nil, fmt.Errorf("invalid regex: %w", err)
	}
	if maxResults <= 0 {
		maxResults = 50
	}
	if contextLines < 0 {
		contextLines = 0
	}
	q := `SELECT root, path FROM files WHERE 1=1`
	var args []any
	if root != "" {
		q += ` AND root = ?`
		args = append(args, root)
	}
	if glob != "" {
		q += ` AND path LIKE ?`
		args = append(args, "%"+strings.ReplaceAll(glob, "*", "%")+"%")
	}
	q += ` AND lang IN ('csharp','go','shader','prefab') ORDER BY path`
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	type fileRow struct{ root, path string }
	var files []fileRow
	for rows.Next() {
		var r fileRow
		if err := rows.Scan(&r.root, &r.path); err == nil {
			files = append(files, r)
		}
	}
	rows.Close()

	var out []PatternHit
	for _, fr := range files {
		data, err := os.ReadFile(e.absOf(fr.root, fr.path))
		if err != nil || len(data) > 32<<20 {
			continue
		}
		lines := strings.Split(string(data), "\n")
		for i, line := range lines {
			if !re.MatchString(line) {
				continue
			}
			hit := PatternHit{Root: fr.root, Path: fr.path, Line: i + 1, Text: strings.TrimRight(line, "\r")}
			if contextLines > 0 {
				lo := i - contextLines
				if lo < 0 {
					lo = 0
				}
				hi := i + contextLines + 1
				if hi > len(lines) {
					hi = len(lines)
				}
				for k := lo; k < hi; k++ {
					hit.Context = append(hit.Context, fmt.Sprintf("%d| %s", k+1, strings.TrimRight(lines[k], "\r")))
				}
			}
			out = append(out, hit)
			if len(out) >= maxResults {
				return out, nil
			}
		}
	}
	return out, nil
}

// ---------------------------------------------------------------- asset layer

// AssetHit is a prefab/scene/shader/config node.
type AssetHit struct {
	Key    string            `json:"key"`
	Kind   string            `json:"kind"`
	Root   string            `json:"root"`
	Path   string            `json:"path"`
	Name   string            `json:"name"`
	UnityID string           `json:"unity_id,omitempty"`
	Line   int               `json:"line,omitempty"`
	Meta   map[string]string `json:"meta,omitempty"`
}

// SearchAssets finds assets by name/path/kind.
func (e *Engine) SearchAssets(name, kind, root, pathFilter string, limit int) ([]AssetHit, error) {
	if limit <= 0 {
		limit = 50
	}
	q := `SELECT asset_key, kind, COALESCE(root,''), COALESCE(path,''), COALESCE(name,''),
		COALESCE(unity_id,''), COALESCE(line,0), COALESCE(meta,'{}') FROM assets WHERE 1=1`
	var args []any
	if name != "" {
		q += ` AND (name LIKE ? OR asset_key LIKE ?)`
		args = append(args, "%"+name+"%", "%"+name+"%")
	}
	if kind != "" {
		q += ` AND kind = ?`
		args = append(args, kind)
	}
	if root != "" {
		q += ` AND root = ?`
		args = append(args, root)
	}
	if pathFilter != "" {
		q += ` AND path LIKE ?`
		args = append(args, "%"+pathFilter+"%")
	}
	q += ` ORDER BY CASE kind WHEN 'prefab' THEN 0 WHEN 'scene' THEN 1 WHEN 'shader' THEN 2 WHEN 'config_table' THEN 3 ELSE 4 END, path, name LIMIT ?`
	args = append(args, limit)
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []AssetHit
	for rows.Next() {
		var h AssetHit
		var meta string
		if err := rows.Scan(&h.Key, &h.Kind, &h.Root, &h.Path, &h.Name, &h.UnityID, &h.Line, &meta); err != nil {
			return nil, err
		}
		_ = json.Unmarshal([]byte(meta), &h.Meta)
		out = append(out, h)
	}
	return out, rows.Err()
}

// AssetEdgeHit is one asset relationship.
type AssetEdgeHit struct {
	Src    string `json:"src"`
	Dst    string `json:"dst"`
	Kind   string `json:"kind"`
	Detail string `json:"detail,omitempty"`
	Line   int    `json:"line,omitempty"`
	SrcName string `json:"src_name,omitempty"`
	DstName string `json:"dst_name,omitempty"`
}

// AssetEdges returns edges in/out for an asset key.
func (e *Engine) AssetEdges(key, direction string, limit int) ([]AssetEdgeHit, error) {
	if limit <= 0 {
		limit = 200
	}
	where := "src = ?"
	if direction == "in" {
		where = "dst = ?"
	}
	q := `SELECT e.src, e.dst, e.kind, COALESCE(e.detail,''), COALESCE(e.line,0),
		COALESCE(a.name,''), COALESCE(b.name,'')
		FROM asset_edges e
		LEFT JOIN assets a ON a.asset_key = e.src
		LEFT JOIN assets b ON b.asset_key = e.dst
		WHERE e.` + where + ` LIMIT ?`
	rows, err := e.St.DB().Query(q, key, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []AssetEdgeHit
	for rows.Next() {
		var h AssetEdgeHit
		if err := rows.Scan(&h.Src, &h.Dst, &h.Kind, &h.Detail, &h.Line, &h.SrcName, &h.DstName); err != nil {
			return nil, err
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

// PrefabTree returns objects of a prefab with parent/child edges.
func (e *Engine) PrefabTree(pathFilter, root string, limit int) ([]AssetHit, []AssetEdgeHit, error) {
	if limit <= 0 {
		limit = 500
	}
	rootAsset, err := e.SearchAssets("", "prefab", root, pathFilter, 1)
	if err != nil {
		return nil, nil, err
	}
	if len(rootAsset) == 0 {
		scene, sErr := e.SearchAssets("", "scene", root, pathFilter, 1)
		if sErr != nil {
			return nil, nil, sErr
		}
		rootAsset = scene
	}
	if len(rootAsset) == 0 {
		return nil, nil, fmt.Errorf("no prefab/scene matched %q", pathFilter)
	}
	base := rootAsset[0]
	objs, err := e.SearchAssets("", "", base.Root, base.Path, limit)
	if err != nil {
		return nil, nil, err
	}
	edges, err := e.AssetEdges(base.Key, "out", limit)
	return objs, edges, err
}

// ConfigTable summarizes one config table.
type ConfigTable struct {
	Name    string            `json:"name"`
	Source  string            `json:"source"`
	Format  string            `json:"format"`
	Root    string            `json:"root"`
	Rows    string            `json:"rows,omitempty"`
	Columns string            `json:"columns,omitempty"`
	Headers []string          `json:"headers,omitempty"`
	Extra   map[string]string `json:"extra,omitempty"`
}

// ConfigTables lists detected config tables.
func (e *Engine) ConfigTables(filter, root string, limit int) ([]ConfigTable, error) {
	if limit <= 0 {
		limit = 200
	}
	assets, err := e.SearchAssets(filter, "config_table", root, "", limit)
	if err != nil {
		return nil, err
	}
	var out []ConfigTable
	for _, a := range assets {
		headers := []string{}
		if h := a.Meta["headers"]; h != "" {
			for _, x := range strings.Split(h, ",") {
				if x = strings.TrimSpace(x); x != "" {
					headers = append(headers, x)
				}
			}
		}
		out = append(out, ConfigTable{
			Name: a.Name, Source: a.Path, Format: a.Meta["format"], Root: a.Root,
			Rows: a.Meta["rows"], Columns: a.Meta["columns"], Headers: headers, Extra: a.Meta,
		})
	}
	return out, nil
}

// ConfigReader is a code location that reads config by id/type.
type ConfigReader struct {
	SymbolHit
	API string `json:"api"`
}

// ConfigReaders finds code that reads configuration (type-driven, not fixed names).
func (e *Engine) ConfigReaders(table string, limit int) ([]ConfigReader, bool, error) {
	if limit <= 0 {
		limit = 100
	}
	apis := []string{"GetDataById", "GetConfig", "LoadConfig", "ConfigManager", "ReloadConfig", "Config233"}
	var out []ConfigReader
	seen := map[int64]bool{}
	for _, api := range apis {
		hits, err := e.SearchSymbols(api, nil, "", "", "", limit)
		if err != nil {
			return nil, false, err
		}
		for _, h := range hits {
			if seen[h.ID] {
				continue
			}
			seen[h.ID] = true
			out = append(out, ConfigReader{SymbolHit: h, API: api})
			if len(out) >= limit {
				break
			}
		}
	}
	// also find call sites of GetDataById
	hot := false
	for _, h := range out {
		if strings.Contains(h.Name, "HotReload") || strings.Contains(h.Name, "ReloadConfig") {
			hot = true
		}
	}
	if !hot {
		// hot reload may be implemented in another symbol name; check refs too
		rows, err := e.St.DB().Query(`SELECT COUNT(*) FROM symbols WHERE name LIKE '%HotReload%' OR qualname LIKE '%ConfigHotReload%'`)
		if err == nil {
			var n int
			if rows.Scan(&n) == nil && n > 0 {
				hot = true
			}
			rows.Close()
		}
	}
	_ = table
	return out, hot, nil
}

// Overview is the project-level summary used by onboarding.
type Overview struct {
	Dir          string            `json:"dir"`
	Strategy     string            `json:"strategy"`
	Engine       string            `json:"engine,omitempty"`
	Notes        string            `json:"notes,omitempty"`
	Roots        []RootSummary     `json:"roots"`
	Files        int               `json:"files"`
	Symbols      int               `json:"symbols"`
	Edges        int               `json:"edges"`
	Assets       int               `json:"assets"`
	Languages    []string          `json:"languages,omitempty"`
	TopKinds     map[string]int    `json:"symbol_kinds,omitempty"`
	TopNamespaces []NamespaceCount `json:"top_namespaces,omitempty"`
	AssetKinds   map[string]int    `json:"asset_kinds,omitempty"`
	ConfigTables int               `json:"config_tables"`
	HotReload    bool              `json:"config_hot_reload"`
	IndexedAt    string            `json:"indexed_at,omitempty"`
}

// RootSummary is one root in the overview.
type RootSummary struct {
	Key       string   `json:"key"`
	Path      string   `json:"path"`
	Role      string   `json:"role,omitempty"`
	Exists    bool     `json:"exists"`
	Strategy  string   `json:"strategy"`
	GoModule  string   `json:"go_module,omitempty"`
	Languages []string `json:"languages,omitempty"`
}

// NamespaceCount is a namespace frequency row.
type NamespaceCount struct {
	Namespace string `json:"namespace"`
	Count     int    `json:"count"`
}

// BuildOverview aggregates the current index.
func (e *Engine) BuildOverview() (*Overview, error) {
	cfg := e.Cfg
	det := detectSummary(cfg)
	o := &Overview{
		Dir: cfg.Dir, Strategy: det.Strategy, Engine: det.Engine, Notes: det.Notes,
		Languages: det.Languages, Roots: det.Roots,
	}
	e.St.DB().QueryRow(`SELECT (SELECT COUNT(*) FROM files), (SELECT COUNT(*) FROM symbols),
		(SELECT COUNT(*) FROM edges), (SELECT COUNT(*) FROM assets)`).Scan(&o.Files, &o.Symbols, &o.Edges, &o.Assets)
	o.IndexedAt = e.St.GetMeta("indexed_at")
	o.TopKinds = e.countBy(`SELECT kind, COUNT(*) FROM symbols GROUP BY kind ORDER BY 2 DESC LIMIT 15`)
	o.AssetKinds = e.countBy(`SELECT kind, COUNT(*) FROM assets GROUP BY kind ORDER BY 2 DESC LIMIT 15`)
	var nsCounts []NamespaceCount
	rows, err := e.St.DB().Query(`SELECT COALESCE(namespace,''), COUNT(*) c FROM symbols WHERE COALESCE(namespace,'') <> ''
		GROUP BY namespace ORDER BY c DESC LIMIT 20`)
	if err == nil {
		for rows.Next() {
			var n NamespaceCount
			if rows.Scan(&n.Namespace, &n.Count) == nil {
				nsCounts = append(nsCounts, n)
			}
		}
		rows.Close()
	}
	o.TopNamespaces = nsCounts
	if n, err := e.ConfigTables("", "", 1); err == nil {
		o.ConfigTables = len(n)
	}
	var totalTables int
	_ = e.St.DB().QueryRow(`SELECT COUNT(*) FROM assets WHERE kind='config_table'`).Scan(&totalTables)
	o.ConfigTables = totalTables
	_, hot, _ := e.ConfigReaders("", 1)
	o.HotReload = hot
	return o, nil
}

func (e *Engine) countBy(q string) map[string]int {
	out := map[string]int{}
	rows, err := e.St.DB().Query(q)
	if err != nil {
		return out
	}
	defer rows.Close()
	for rows.Next() {
		var k string
		var v int
		if rows.Scan(&k, &v) == nil {
			out[k] = v
		}
	}
	return out
}

func placeholders(n int) string {
	if n <= 0 {
		return "?" 
	}
	return strings.TrimSuffix(strings.Repeat("?,", n), ",")
}

// detectInfo is the subset of detection used by overview.
type detectInfo struct {
	Strategy  string
	Engine    string
	Notes     string
	Languages []string
	Roots     []RootSummary
}

// detectSummary adapts detect.Result for overview without importing cycles.
func detectSummary(cfg *config.Config) detectInfo {
	det := detect.Detect(cfg)
	info := detectInfo{
		Strategy: det.Strategy.Name, Engine: det.Strategy.Engine,
		Notes: det.Strategy.Notes, Languages: det.Languages,
	}
	for _, r := range det.Roots {
		info.Roots = append(info.Roots, RootSummary{
			Key: r.Key, Path: r.Path, Role: r.Role, Exists: r.Exists,
			Strategy: r.Strategy, GoModule: r.GoModule, Languages: r.Languages,
		})
	}
	return info
}
