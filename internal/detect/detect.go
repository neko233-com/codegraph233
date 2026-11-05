// Package detect recognizes the engine/strategy and languages of each root.
package detect

import (
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/neko233-com/game-project-graph/internal/config"
	"github.com/neko233-com/game-project-graph/internal/model"
)

// RootInfo is the resolved state of one configured root.
type RootInfo struct {
	Key       string   `json:"key"`
	Path      string   `json:"path"`
	Role      string   `json:"role"`
	Exists    bool     `json:"exists"`
	Strategy  string   `json:"strategy"`
	Engine    string   `json:"engine,omitempty"`
	GoModule  string   `json:"go_module,omitempty"`
	Languages []string `json:"languages"`
	Evidence  []string `json:"evidence,omitempty"`
	Notes     string   `json:"notes,omitempty"`
}

// Result is the aggregate detection for all roots.
type Result struct {
	Strategy   model.Strategy `json:"strategy"`
	Roots      []RootInfo     `json:"roots"`
	Languages  []string       `json:"languages"`
	Missing    []string       `json:"missing,omitempty"`
	ConfigDirs []string       `json:"config_dirs,omitempty"`
}

// Detect resolves every configured root, tolerating missing directories.
func Detect(cfg *config.Config) Result {
	res := Result{}
	seenLang := map[string]bool{}
	for _, r := range cfg.Roots {
		abs := cfg.AbsPath(r)
		info := RootInfo{Key: r.Key, Path: abs, Role: r.Role}
		st, err := os.Stat(abs)
		if err != nil || !st.IsDir() {
			info.Exists = false
			info.Strategy = "missing"
			info.Notes = "目录不存在（允许后端/共享目录留空）"
			res.Missing = append(res.Missing, r.Key)
			res.Roots = append(res.Roots, info)
			continue
		}
		info.Exists = true
		info.Strategy, info.Engine, info.Evidence = detectEngine(abs)
		if r.Lang != "" && r.Lang != "auto" {
			info.Languages = []string{strings.ToLower(r.Lang)}
		} else {
			info.Languages = detectLanguages(abs)
		}
		if gm := readGoModule(abs); gm != "" {
			info.GoModule = gm
		}
		for _, l := range info.Languages {
			seenLang[l] = true
		}
		res.Roots = append(res.Roots, info)
	}

	for l := range seenLang {
		res.Languages = append(res.Languages, l)
	}
	sort.Strings(res.Languages)

	res.Strategy = aggregate(cfg.Strategy, res)
	res.ConfigDirs = discoverConfigDirs(cfg)
	return res
}

func detectEngine(dir string) (strategy, engine string, evidence []string) {
	if matches, _ := filepath.Glob(filepath.Join(dir, "*.uproject")); len(matches) > 0 {
		return "ue", "Unreal Engine", []string{"found " + filepath.Base(matches[0])}
	}
	if st, err := os.Stat(filepath.Join(dir, "Content")); err == nil && st.IsDir() {
		if hasExt(filepath.Join(dir, "Content"), ".uasset", 4000) {
			return "ue", "Unreal Engine", []string{"Content/*.uasset"}
		}
	}

	versionFile := filepath.Join(dir, "ProjectSettings", "ProjectVersion.txt")
	if data, err := os.ReadFile(versionFile); err == nil {
		text := string(data)
		ver := extractVersion(text)
		evidence = append(evidence, "ProjectSettings/ProjectVersion.txt: "+ver)
		if isTuanjieVersion(ver) {
			return "tuanjie", "Tuanjie " + ver, evidence
		}
		if ver != "" {
			return "unity", "Unity " + ver, evidence
		}
	}
	if st, err := os.Stat(filepath.Join(dir, "Assets")); err == nil && st.IsDir() {
		evidence = append(evidence, "Assets/ directory")
		if st2, err := os.Stat(filepath.Join(dir, "ProjectSettings")); err == nil && st2.IsDir() {
			evidence = append(evidence, "ProjectSettings/ directory")
		}
		return "unity", "Unity", evidence
	}
	if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
		return "generic", "Go module", []string{"go.mod"}
	}
	return "generic", "", nil
}

func extractVersion(text string) string {
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if v, ok := strings.CutPrefix(line, "m_EditorVersion:"); ok {
			return strings.TrimSpace(v)
		}
	}
	return ""
}

// Tuanjie builds append a "t" patch level, e.g. 2022.3.62t15 (Unity: 2022.3.62f1).
func isTuanjieVersion(ver string) bool {
	if ver == "" {
		return false
	}
	for i := 0; i < len(ver); i++ {
		if ver[i] == 't' {
			return true
		}
		if ver[i] == 'f' || ver[i] == 'a' || ver[i] == 'b' || ver[i] == 'p' {
			return false
		}
	}
	return false
}

func detectLanguages(dir string) []string {
	cs, goFiles := 0, 0
	var walk func(string, int)
	walk = func(cur string, depth int) {
		if depth > 6 || cs+goFiles > 8000 {
			return
		}
		entries, err := os.ReadDir(cur)
		if err != nil {
			return
		}
		for _, e := range entries {
			name := e.Name()
			if isSkipDir(name) {
				continue
			}
			full := filepath.Join(cur, name)
			if e.IsDir() {
				walk(full, depth+1)
				continue
			}
			switch strings.ToLower(filepath.Ext(name)) {
			case ".cs":
				cs++
			case ".go":
				goFiles++
			}
		}
	}
	walk(dir, 0)
	out := []string{}
	if cs > 0 {
		out = append(out, "csharp")
	}
	if goFiles > 0 {
		out = append(out, "go")
	}
	return out
}

func isSkipDir(name string) bool {
	switch name {
	case ".git", "Library", "Temp", "Logs", "obj", "node_modules", ".codely-cli",
		"Bundles", "Intermediate", "Binaries", "Saved", "DerivedDataCache", ".vs",
		".cursor", ".codex", ".game-project-graph":
		return true
	}
	return false
}

func hasExt(dir, ext string, limit int) bool {
	found := false
	count := 0
	_ = filepath.WalkDir(dir, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		count++
		if count > limit {
			return filepath.SkipAll
		}
		if !d.IsDir() && strings.EqualFold(filepath.Ext(path), ext) {
			found = true
			return filepath.SkipAll
		}
		return nil
	})
	return found
}

func readGoModule(dir string) string {
	data, err := os.ReadFile(filepath.Join(dir, "go.mod"))
	if err != nil {
		return ""
	}
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if v, ok := strings.CutPrefix(line, "module "); ok {
			return strings.TrimSpace(v)
		}
	}
	return ""
}

func aggregate(requested string, res Result) model.Strategy {
	requested = strings.ToLower(strings.TrimSpace(requested))
	if requested != "" && requested != "auto" {
		st := model.Strategy{Name: requested, Confidence: "explicit", Languages: res.Languages}
		st.Evidence = append(st.Evidence, "配置 strategy="+requested)
		for _, r := range res.Roots {
			if r.Exists && (r.Strategy == "tuanjie" || r.Strategy == "unity" || r.Strategy == "ue") {
				st.Engine = r.Engine
				break
			}
		}
		addStrategyNotes(&st)
		return st
	}
	for _, r := range res.Roots {
		if !r.Exists {
			continue
		}
		if r.Strategy == "tuanjie" || r.Strategy == "unity" || r.Strategy == "ue" {
			st := model.Strategy{
				Name: r.Strategy, Confidence: "high", Engine: r.Engine,
				Languages: res.Languages, Evidence: append([]string{}, r.Evidence...),
			}
			addStrategyNotes(&st)
			return st
		}
	}
	for _, r := range res.Roots {
		if r.Exists && r.Strategy != "missing" {
			return model.Strategy{Name: r.Strategy, Confidence: "low", Languages: res.Languages}
		}
	}
	return model.Strategy{Name: "generic", Confidence: "low", Languages: res.Languages}
}

func addStrategyNotes(st *model.Strategy) {
	switch st.Name {
	case "tuanjie":
		st.Notes = "团结引擎：C# 源码可完整索引；.meta 的 guid 为加密形式，资产 guid 反查需 Unity/团结 编辑器桥，本工具保留 guid 关系但无法直接映射到资产路径。"
	case "unity":
		st.Notes = "Unity：C# 源码可完整索引；.meta guid 为明文，可按 guid 反查资产路径。"
	case "ue":
		st.Notes = "UE 策略为预留：当前版本只做项目识别，C++/蓝图/资产图尚未实现。"
	}
}

// discoverConfigDirs finds likely config-table directories across roots.
func discoverConfigDirs(cfg *config.Config) []string {
	seen := map[string]bool{}
	var out []string
	add := func(p string) {
		p = filepath.Clean(p)
		if seen[p] {
			return
		}
		seen[p] = true
		out = append(out, p)
	}
	for _, d := range cfg.ConfigDirs {
		p := d
		if !filepath.IsAbs(p) {
			p = filepath.Join(cfg.Dir, d)
		}
		if st, err := os.Stat(p); err == nil && st.IsDir() {
			add(p)
		}
	}
	candidates := []string{
		"BusinessConfig", "Config", "Configs", "config", "配置",
		filepath.Join("..", "Team-Resources", "BusinessConfig"),
		filepath.Join("..", "team-resources-project-sf", "BusinessConfig"),
	}
	base := []string{cfg.Dir}
	for _, r := range cfg.Roots {
		base = append(base, cfg.AbsPath(r))
	}
	for _, b := range base {
		for _, c := range candidates {
			p := filepath.Join(b, c)
			if st, err := os.Stat(p); err == nil && st.IsDir() {
				add(p)
			}
		}
	}
	return out
}
