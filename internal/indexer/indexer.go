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
	Force     bool
	Jobs      int
	Quiet     bool
	FastWrite bool
	Progress  func(done, total int, current string)
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
	Strategy  string
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

	var st *store.Store
	var err error
	if opts.FastWrite {
		st, err = store.OpenFast(cfg.DBPath())
	} else {
		st, err = store.Open(cfg.DBPath())
	}
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
				rs.Strategy = info.Strategy
			}
		}
		roots = append(roots, rs)
	}

	// guid -> path mapping (from the Unity/Tuanjie exporter or plaintext .meta)
	feats := cfg.ResolveFeatures()
	var guidRows []store.GuidRow
	var guidFiles []string
	var gErr error
	if feats.GuidMap {
		guidRows, guidFiles, gErr = LoadGuidMaps(cfg)
	}
	if gErr != nil && !opts.Quiet {
		fmt.Fprintf(os.Stderr, "[guid] %v\n", gErr)
	}
	if len(guidRows) > 0 {
		if err := st.ReplaceGuidMap(guidRows); err != nil {
			return nil, err
		}
	}
	stats.GuidEntries = len(guidRows)
	if !opts.Quiet && len(guidRows) > 0 {
		fmt.Fprintf(os.Stderr, "[guid] %d mappings from %d file(s)\n", len(guidRows), len(guidFiles))
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
	edges, unresolved, err := rebindIndex(st, opts.Jobs, opts.Quiet)
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
	feats := cfg.ResolveFeatures()
	prefabsOn := feats.Prefabs && cfg.Enabled(cfg.Index.Prefabs)
	shadersOn := feats.Shaders && cfg.Enabled(cfg.Index.Shaders)
	configsOn := feats.ConfigTables && cfg.Enabled(cfg.Index.ConfigTables)
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
				lang := langOf(rel, p, configDirs, feats, prefabsOn, shadersOn, configsOn)
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
				case "asset":
					limit = 64 << 20
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

func langOf(rel, abs string, configDirs []string, feats config.Features, prefabs, shaders, configs bool) string {
	ext := strings.ToLower(filepath.Ext(rel))

	// code languages
	switch ext {
	case ".cs":
		if feats.Code {
			return "csharp"
		}
		return ""
	case ".go":
		if feats.Code {
			return "go"
		}
		return ""
	case ".rs":
		if feats.Code {
			return "rust"
		}
		return ""
	case ".ts", ".tsx", ".js", ".jsx":
		if feats.Code {
			return "typescript"
		}
		return ""
	}

	// game data families
	switch ext {
	case ".prefab", ".unity":
		if prefabs {
			return "prefab"
		}
		return ""
	case ".shader", ".hlsl", ".cginc", ".compute":
		if shaders {
			return "shader"
		}
		return ""
	case ".xlsx", ".csv", ".tsv":
		if configs {
			return "config"
		}
		return ""
	case ".json":
		if configs && isUnderConfigDir(abs, configDirs) {
			return "config"
		}
	}

	if ext == ".meta" {
		if feats.Meta {
			return "asset"
		}
		return ""
	}
	if feats.Assets && assets.AssetExt(rel) {
		return "asset"
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
				results[j.idx] = safeParse(j.file, opts)
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

// safeParse isolates a single malformed file so one panic cannot kill the run.
func safeParse(f scanFile, opts Options) (res *model.FileResult) {
	defer func() {
		if r := recover(); r != nil {
			res = &model.FileResult{
				RelPath: f.Rel, Root: f.Root, Lang: f.Lang,
				Errors: []string{fmt.Sprintf("panic: %v", r)},
			}
		}
	}()
	return parseOne(f, opts)
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
	case "asset":
		return assets.ParseAsset(f.Abs, f.Rel, f.Root, 4<<20)
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
		var metaGuids []store.GuidRow
		for _, a := range res.Assets {
			if a.Kind != "guid_ref" {
				continue
			}
			if tp := a.Meta["target_path"]; tp != "" {
				g := a.Meta["guid"]
				if g == "" {
					g = a.FileID
				}
				metaGuids = append(metaGuids, store.GuidRow{GUID: g, Path: tp})
			}
		}
		if err := tx.AddGuidRows(metaGuids); err != nil {
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
	GuidEntries int           `json:"guid_map_entries"`
	GuidRefs    int           `json:"guid_refs"`
	GuidMapped  int           `json:"guid_refs_mapped"`
	Profile     string        `json:"profile,omitempty"`
	Features    config.Features `json:"features"`
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
		Profile:    cfgData.Profile,
		Features:   cfgData.ResolveFeatures(),
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
	s.GuidEntries = st.GuidCount()
	_ = st.DB().QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges WHERE dst LIKE 'guid:%')`).Scan(&s.GuidRefs)
	_ = st.DB().QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges e
		WHERE dst LIKE 'guid:%' AND EXISTS (SELECT 1 FROM guid_map g WHERE g.guid = SUBSTR(e.dst, 6)))`).Scan(&s.GuidMapped)
	return s
}
