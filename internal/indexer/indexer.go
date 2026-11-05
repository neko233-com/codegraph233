package indexer

import (
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/neko233-com/codegraph233/internal/assets"
	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/detect"
	"github.com/neko233-com/codegraph233/internal/lang/csharp"
	"github.com/neko233-com/codegraph233/internal/lang/golang"
	"github.com/neko233-com/codegraph233/internal/lang/rust"
	"github.com/neko233-com/codegraph233/internal/lang/typescript"
	"github.com/neko233-com/codegraph233/internal/model"
	"github.com/neko233-com/codegraph233/internal/store"
)

// Options tunes an indexing run.
type Options struct {
	Force    bool
	Jobs     int
	Quiet    bool
	Progress func(done, total int, current string)
}

type scanFile struct {
	Root    string
	Abs     string
	Rel     string
	Lang    string
	Size    int64
	ModTime int64
}

type rootScan struct {
	Key       string
	Dir       string
	GoModule  string
	ConfigDir bool
}

const (
	maxCodeBytes   int64 = 4 << 20
	maxPrefabBytes int64 = 24 << 20
)

// Run indexes every configured root and rebuilds the symbol graph.
func Run(cfg *config.Config, opts Options) (*model.IndexStats, error) {
	started := time.Now()
	det := detect.Detect(cfg)
	stats := &model.IndexStats{Roots: len(cfg.Roots), ByLang: map[string]int{}, ByRoot: map[string]int{}}

	st, err := store.Open(cfg.DBPath())
	if err != nil {
		return nil, err
	}
	defer st.Close()

	roots := make([]rootScan, 0, len(cfg.Roots))
	for _, r := range cfg.Roots {
		abs := cfg.AbsPath(r)
		if fi, err := os.Stat(abs); err != nil || !fi.IsDir() {
			continue
		}
		rs := rootScan{Key: r.Key, Dir: abs}
		for _, info := range det.Roots {
			if info.Key == r.Key {
				rs.GoModule = info.GoModule
			}
		}
		roots = append(roots, rs)
	}

	files, present := scanRoots(cfg, roots, st, opts)
	if !opts.Quiet && opts.Progress != nil {
		opts.Progress(0, len(files), "")
	}

	states, err := st.LoadFileStates()
	if err != nil {
		return nil, err
	}

	var changed []scanFile
	for _, f := range files {
		key := f.Root + "|" + f.Rel
		if prev, ok := states[key]; ok && !opts.Force && prev.Size == f.Size && prev.ModTime == f.ModTime && prev.Lang == f.Lang {
			continue
		}
		changed = append(changed, f)
	}
	removed, err := st.PruneFiles(present)
	if err != nil {
		return nil, err
	}
	stats.RemovedFiles = removed

	// Parse and persist in bounded batches so peak memory stays flat on very
	// large projects (client + backend + assets).
	const writeBatch = 400
	var parseErrors []string
	for start := 0; start < len(changed); start += writeBatch {
		end := start + writeBatch
		if end > len(changed) {
			end = len(changed)
		}
		part := changed[start:end]
		results := parseAll(part, opts)
		for _, r := range results {
			if r != nil && len(r.Errors) > 0 && len(parseErrors) < 50 {
				parseErrors = append(parseErrors, r.Errors...)
			}
		}
		if err := writeResults(st, part, results); err != nil {
			return nil, err
		}
		if !opts.Quiet {
			fmt.Fprintf(os.Stderr, "\n[write] %d/%d files (%.1fs)\n", end, len(changed), time.Since(started).Seconds())
		}
	}

	if !opts.Quiet {
		fmt.Fprintf(os.Stderr, "[bind] resolving references…\n")
	}
	edges, unresolved, err := rebind(st)
	if err != nil {
		return nil, err
	}

	for _, r := range roots {
		_ = st.UpdateSymbolParents(r.Key)
	}

	stats.Files = len(files)
	stats.ChangedFiles = len(changed)
	stats.Edges = edges
	stats.Unresolved = unresolved
	for _, f := range files {
		stats.ByLang[f.Lang]++
		stats.ByRoot[f.Root]++
	}
	sf, sy, rf, _, as, ae := st.Counts()
	stats.Files = sf
	stats.Symbols = sy
	stats.Refs = rf
	stats.Assets = as
	stats.AssetEdges = ae
	stats.ElapsedMs = time.Since(started).Milliseconds()
	stats.Errors = parseErrors

	detJSON, _ := json.Marshal(det)
	_ = st.SetMeta("strategy", det.Strategy.Name)
	_ = st.SetMeta("detection", string(detJSON))
	_ = st.SetMeta("indexed_at", time.Now().Format(time.RFC3339))
	_ = st.SetMeta("indexed_unix", fmt.Sprint(store.NowUnix()))
	statsJSON, _ := json.Marshal(stats)
	_ = st.SetMeta("stats", string(statsJSON))
	return stats, nil
}

func scanRoots(cfg *config.Config, roots []rootScan, st *store.Store, opts Options) ([]scanFile, map[string]bool) {
	excludes := cfg.Excludes()
	prefabsOn := cfg.Enabled(cfg.Index.Prefabs)
	shadersOn := cfg.Enabled(cfg.Index.Shaders)
	configsOn := cfg.Enabled(cfg.Index.ConfigTables)
	configDirs := detect.Detect(cfg).ConfigDirs

	var out []scanFile
	present := map[string]bool{}
	var mu sync.Mutex
	var wg sync.WaitGroup

	for _, r := range roots {
		wg.Add(1)
		go func(rs rootScan) {
			defer wg.Done()
			_ = filepath.WalkDir(rs.Dir, func(p string, d fs.DirEntry, err error) error {
				if err != nil {
					return nil
				}
				rel, relErr := filepath.Rel(rs.Dir, p)
				if relErr != nil {
					return nil
				}
				rel = filepath.ToSlash(rel)
				if d.IsDir() {
					if p != rs.Dir && isExcluded(rel, d.Name(), excludes) {
						return filepath.SkipDir
					}
					return nil
				}
				info, err := d.Info()
				if err != nil {
					return nil
				}
				lang := langOf(rel, p, configDirs, r.Key, prefabsOn, shadersOn, configsOn, cfg)
				if (lang == "csharp" || lang == "go" || lang == "rust" || lang == "typescript") && !cfg.LanguageEnabled(lang) {
					// Code languages follow the policy matrix: csharp default, go optional, lua planned.
					return nil
				}
				if lang == "" {
					return nil
				}
				limit := int64(2 << 20)
				switch lang {
				case "prefab":
					limit = maxPrefabBytes
				case "config":
					limit = 32 << 20
				default:
					limit = maxCodeBytes
				}
				if info.Size() > limit || info.Size() == 0 {
					return nil
				}
				f := scanFile{
					Root: rs.Key, Abs: p, Rel: rel, Lang: lang,
					Size: info.Size(), ModTime: info.ModTime().UnixNano(),
				}
				mu.Lock()
				out = append(out, f)
				present[rs.Key+"|"+rel] = true
				mu.Unlock()
				return nil
			})
		}(r)
	}
	wg.Wait()
	sort.Slice(out, func(i, j int) bool {
		if out[i].Root != out[j].Root {
			return out[i].Root < out[j].Root
		}
		return out[i].Rel < out[j].Rel
	})
	return out, present
}

func isExcluded(rel, base string, excludes []string) bool {
	for _, ex := range excludes {
		if ex == "" {
			continue
		}
		if strings.Contains(ex, "/") || strings.Contains(ex, "\\") {
			ex = filepath.ToSlash(ex)
			ex = strings.TrimSuffix(ex, "/")
			if rel == ex || strings.HasPrefix(rel, ex+"/") {
				return true
			}
			continue
		}
		if base == ex {
			return true
		}
	}
	return false
}

func langOf(rel, abs string, configDirs []string, rootKey string, prefabs, shaders, configs bool, cfg *config.Config) string {
	ext := strings.ToLower(filepath.Ext(rel))
	switch ext {
	case ".cs":
		return "csharp"
	case ".go":
		return "go"
	case ".rs":
		return "rust"
	case ".ts", ".tsx", ".js", ".jsx":
		return "typescript"
	case ".prefab", ".unity":
		if prefabs {
			return "prefab"
		}
	case ".shader", ".hlsl", ".cginc", ".compute":
		if shaders {
			return "shader"
		}
	case ".xlsx", ".csv", ".tsv":
		if configs {
			return "config"
		}
	case ".json":
		if configs && isUnderConfigDir(abs, configDirs) {
			return "config"
		}
	}
	return ""
}

func isUnderConfigDir(abs string, dirs []string) bool {
	clean := filepath.Clean(abs)
	for _, d := range dirs {
		d = filepath.Clean(d)
		if strings.HasPrefix(clean, d+string(os.PathSeparator)) || clean == d {
			return true
		}
	}
	return false
}

func parseAll(files []scanFile, opts Options) []*model.FileResult {
	results := make([]*model.FileResult, len(files))
	if len(files) == 0 {
		return results
	}
	jobs := opts.Jobs
	if jobs <= 0 {
		jobs = runtime.NumCPU()
	}
	if jobs < 1 {
		jobs = 1
	}
	type job struct {
		idx  int
		file scanFile
	}
	ch := make(chan job, jobs*2)
	var wg sync.WaitGroup
	var done int64
	var mu sync.Mutex
	for w := 0; w < jobs; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := range ch {
				results[j.idx] = parseOne(j.file, opts)
				mu.Lock()
				done++
				if opts.Progress != nil && !opts.Quiet {
					opts.Progress(int(done), len(files), j.file.Rel)
				}
				mu.Unlock()
			}
		}()
	}
	for i, f := range files {
		ch <- job{idx: i, file: f}
	}
	close(ch)
	wg.Wait()
	return results
}

func parseOne(f scanFile, opts Options) *model.FileResult {
	switch f.Lang {
	case "csharp":
		data, err := os.ReadFile(f.Abs)
		if err != nil {
			return &model.FileResult{RelPath: f.Rel, Root: f.Root, Lang: f.Lang, Errors: []string{err.Error()}}
		}
		return csharp.Parse(f.Rel, f.Root, data)
	case "go":
		data, err := os.ReadFile(f.Abs)
		if err != nil {
			return &model.FileResult{RelPath: f.Rel, Root: f.Root, Lang: f.Lang, Errors: []string{err.Error()}}
		}
		modulePath := goModuleOf(f.Abs)
		relDir := filepath.ToSlash(filepath.Dir(f.Rel))
		if relDir == "." {
			relDir = ""
		}
		return golang.Parse(f.Rel, f.Root, modulePath, relDir, data)
	case "rust":
		data, err := os.ReadFile(f.Abs)
		if err != nil {
			return &model.FileResult{RelPath: f.Rel, Root: f.Root, Lang: f.Lang, Errors: []string{err.Error()}}
		}
		return rust.Parse(f.Rel, f.Root, data)
	case "typescript":
		data, err := os.ReadFile(f.Abs)
		if err != nil {
			return &model.FileResult{RelPath: f.Rel, Root: f.Root, Lang: f.Lang, Errors: []string{err.Error()}}
		}
		return typescript.Parse(f.Rel, f.Root, data)
	case "prefab":
		return assets.ParsePrefab(f.Abs, f.Rel, f.Root)
	case "shader":
		res, _ := assets.ParseShader(f.Abs, f.Rel, f.Root)
		return res
	case "config":
		return assets.ParseConfigFile(f.Abs, f.Rel, f.Root)
	}
	return &model.FileResult{RelPath: f.Rel, Root: f.Root, Lang: f.Lang}
}

// goModuleOf reads the nearest go.mod, returning the module path.
func goModuleOf(abs string) string {
	dir := filepath.Dir(abs)
	for i := 0; i < 8; i++ {
		data, err := os.ReadFile(filepath.Join(dir, "go.mod"))
		if err == nil {
			for _, line := range strings.Split(string(data), "\n") {
				line = strings.TrimSpace(line)
				if v, ok := strings.CutPrefix(line, "module "); ok {
					return strings.TrimSpace(v)
				}
			}
			return ""
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return ""
}

func writeResults(st *store.Store, files []scanFile, results []*model.FileResult) error {
	tx, err := st.Begin()
	if err != nil {
		return err
	}
	pending := 0
	for i, res := range results {
		if res == nil {
			continue
		}
		f := files[i]
		res.RelPath = f.Rel
		res.Root = f.Root
		// remove the previous row (and its refs/symbols/assets) if it exists
		oldID, idErr := tx.FileID(f.Root, f.Rel)
		if idErr != nil {
			tx.Rollback()
			return idErr
		}
		if oldID > 0 {
			if err := tx.DeleteFile(oldID, f.Rel); err != nil {
				tx.Rollback()
				return err
			}
		}
		fileID, err := tx.UpsertFile(model.FileRec{
			Root: f.Root, Path: f.Rel, Lang: f.Lang, Module: res.Module, Size: f.Size, ModTime: f.ModTime,
		})
		if err != nil {
			tx.Rollback()
			return err
		}
		ids, err := tx.InsertSymbols(fileID, res.Symbols)
		if err != nil {
			tx.Rollback()
			return err
		}
		keyToID := make(map[string]int64, len(res.Symbols))
		for j, s := range res.Symbols {
			key := s.Qualname + "#" + s.Kind + "@" + itoa(s.StartLine)
			keyToID[key] = ids[j]
		}
		if err := tx.InsertRefs(fileID, res.Refs, keyToID); err != nil {
			tx.Rollback()
			return err
		}
		if err := tx.InsertAssets(res.Assets, 0); err != nil {
			tx.Rollback()
			return err
		}
		if err := tx.InsertAssetEdges(res.AssetEdges); err != nil {
			tx.Rollback()
			return err
		}
		pending++
		if pending >= 200 {
			if err := tx.Commit(); err != nil {
				return err
			}
			pending = 0
			if tx, err = st.Begin(); err != nil {
				return err
			}
		}
	}
	if pending > 0 {
		return tx.Commit()
	}
	return tx.Rollback()
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

// rebind rebuilds all symbol edges from stored references.
func rebind(st *store.Store) (int, int, error) {
	t0 := time.Now()
	if err := st.ClearGraph(); err != nil {
		return 0, 0, err
	}
	syms, err := st.LoadSymbolRows()
	if err != nil {
		return 0, 0, err
	}
	refs, err := st.LoadRefs()
	if err != nil {
		return 0, 0, err
	}
	fmt.Fprintf(os.Stderr, "[bind] loaded %d symbols, %d refs (%.1fs)\n", len(syms), len(refs), time.Since(t0).Seconds())
	if len(syms) == 0 || len(refs) == 0 {
		return 0, 0, nil
	}
	b := newBinder(syms)
	srcByID := make(map[int64]store.SymbolRow, len(syms))
	for _, s := range syms {
		srcByID[s.ID] = s
	}
	kindIdx := map[string]int64{}
	var edges []model.ResolvedEdge
	seen := map[[3]int64]bool{}
	unresolved := 0
	var updates []store.RefTargetUpdate
	for _, r := range refs {
		src, ok := srcByID[r.SrcID]
		if !ok || r.SrcID == 0 {
			unresolved++
			continue
		}
		res, ok := b.resolve(r, src)
		if !ok {
			unresolved++
			continue
		}
		updates = append(updates, store.RefTargetUpdate{ID: r.ID, Target: res.target.ID, Confidence: res.confidence})
		if res.target.ID == r.SrcID {
			continue
		}
		ki, ok := kindIdx[r.Kind]
		if !ok {
			ki = int64(len(kindIdx) + 1)
			kindIdx[r.Kind] = ki
		}
		k := [3]int64{r.SrcID, res.target.ID, ki}
		if seen[k] {
			continue
		}
		seen[k] = true
		edges = append(edges, model.ResolvedEdge{
			Src: r.SrcID, Dst: res.target.ID, Kind: r.Kind, Line: r.Line, Confidence: res.confidence,
		})
	}
	fmt.Fprintf(os.Stderr, "[bind] resolved %d edges, %d unresolved (%.1fs)\n", len(edges), unresolved, time.Since(t0).Seconds())
	if err := st.UpdateRefTarget(updates); err != nil {
		return 0, 0, err
	}
	fmt.Fprintf(os.Stderr, "[bind] ref targets written (%.1fs)\n", time.Since(t0).Seconds())
	if err := st.InsertEdges(edges); err != nil {
		return 0, 0, err
	}
	fmt.Fprintf(os.Stderr, "[bind] edges written (%.1fs)\n", time.Since(t0).Seconds())
	return len(edges), unresolved, nil
}

// Status reports the index state without modifying it.
type Status struct {
	Exists     bool           `json:"exists"`
	DBPath     string         `json:"db_path"`
	DataDir    string         `json:"data_dir"`
	IndexedAt  string         `json:"indexed_at,omitempty"`
	Strategy   string         `json:"strategy,omitempty"`
	Detection  detect.Result  `json:"detection"`
	Files      int            `json:"files"`
	Symbols    int            `json:"symbols"`
	Refs       int            `json:"refs"`
	Edges      int            `json:"edges"`
	Assets     int            `json:"assets"`
	AssetEdges int            `json:"asset_edges"`
	ConfigDirs []string       `json:"config_dirs,omitempty"`
	Languages  []string       `json:"languages,omitempty"`
	LanguagePolicy []config.LanguagePolicy `json:"language_policy"`
}

// ReadStatus loads index metadata for status/overview tools.
func ReadStatus(cfg *config.Config) Status {
	cfgData := cfg
	s := Status{
		DBPath:     cfg.DBPath(),
		DataDir:    cfg.DataDir(),
		Detection:  detect.Detect(cfgData),
		ConfigDirs: detect.Detect(cfgData).ConfigDirs,
		Languages:  cfgData.EffectiveLanguages(),
		LanguagePolicy: config.LanguagePolicies,
	}
	if _, err := os.Stat(cfg.DBPath()); err != nil {
		return s
	}
	s.Exists = true
	st, err := store.Open(cfg.DBPath())
	if err != nil {
		return s
	}
	defer st.Close()
	s.IndexedAt = st.GetMeta("indexed_at")
	s.Strategy = st.GetMeta("strategy")
	s.Files, s.Symbols, s.Refs, s.Edges, s.Assets, s.AssetEdges = st.Counts()
	return s
}
