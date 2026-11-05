// Package config loads .codegraph233.json and resolves multi-root layouts.
package config

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const FileName = ".codegraph233.json"

// Root is one indexed directory.  Frontend and backend are both optional: an
// empty or missing path is reported as "missing" instead of failing the run.
type Root struct {
	Key   string `json:"key"`
	Path  string `json:"path"`
	Role  string `json:"role,omitempty"` // frontend | backend | shared | assets
	Lang  string `json:"lang,omitempty"` // force csharp | go | auto
	Notes string `json:"notes,omitempty"`
}

// IndexOptions tunes the scan.
type IndexOptions struct {
	MaxFileBytes int64 `json:"max_file_bytes,omitempty"`
	Jobs         int   `json:"jobs,omitempty"`
	Prefabs      *bool `json:"prefabs,omitempty"`
	Shaders      *bool `json:"shaders,omitempty"`
	ConfigTables *bool `json:"config_tables,omitempty"`
}

// Config is the committed project descriptor.
type Config struct {
	Version     int          `json:"version"`
	Strategy    string       `json:"strategy,omitempty"` // auto | unity | tuanjie | ue | generic
	Roots       []Root       `json:"roots"`
	Languages   []string     `json:"languages,omitempty"`
	Exclude     []string     `json:"exclude,omitempty"`
	ConfigDirs  []string     `json:"config_dirs,omitempty"`
	Index       IndexOptions `json:"index,omitempty"`
	Description string       `json:"description,omitempty"`

	// runtime-only fields
	Dir      string `json:"-"`
	FilePath string `json:"-"`
}

// DefaultExclude contains heavy generated directories for game projects.
var DefaultExclude = []string{
	".git", ".svn", ".hg", "Library", "Temp", "Logs", "obj", "Build", "Builds",
	"node_modules", ".codely-cli", ".codex", ".cursor", ".codegraph233",
	"Bundles", "StreamingAssets", "__pycache__", ".vs", ".idea", "DerivedDataCache",
	"Binaries", "Intermediate", "Saved", ".gradle",
	// local copies / tool caches that duplicate real sources
	".local", ".cache", ".venv", "venv", ".tmp",
}

// Find walks up from start to the first directory containing FileName.
func Find(start string) (string, bool) {
	dir, err := filepath.Abs(start)
	if err != nil {
		return start, false
	}
	for {
		if st, err := os.Stat(filepath.Join(dir, FileName)); err == nil && !st.IsDir() {
			return dir, true
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", false
		}
		dir = parent
	}
}

// Load reads the config for start (or discovers it).  A missing config yields a
// minimal single-root "current directory" config so that the index can run
// before `init` is executed.
func Load(start string) (*Config, error) {
	if env := os.Getenv("CODEGRAPH233_ROOT"); env != "" {
		start = env
	}
	dir, ok := Find(start)
	if !ok {
		abs, err := filepath.Abs(start)
		if err != nil {
			return nil, err
		}
		return &Config{
			Version:  1,
			Strategy: "auto",
			Roots:    []Root{{Key: "project", Path: ".", Role: "frontend"}},
			Dir:      abs,
		}, nil
	}
	path := filepath.Join(dir, FileName)
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	cfg := &Config{}
	if err := json.Unmarshal(data, cfg); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	cfg.Dir = dir
	cfg.FilePath = path
	if cfg.Version == 0 {
		cfg.Version = 1
	}
	if cfg.Strategy == "" {
		cfg.Strategy = "auto"
	}
	if len(cfg.Roots) == 0 {
		cfg.Roots = []Root{{Key: "project", Path: ".", Role: "frontend"}}
	}
	return cfg, nil
}

// Save writes the config with stable formatting.
func Save(cfg *Config) error {
	if cfg.Dir == "" {
		return errors.New("config dir is empty")
	}
	path := filepath.Join(cfg.Dir, FileName)
	data, err := json.MarshalIndent(cfg, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	return os.WriteFile(path, data, 0o644)
}

// DataDir is where the index and derived data live (git-ignored).
func (c *Config) DataDir() string {
	return filepath.Join(c.Dir, ".codegraph233")
}

// DBPath is the SQLite index file.
func (c *Config) DBPath() string {
	return filepath.Join(c.DataDir(), "index.db")
}

// AbsPath resolves a root path against the config directory.
func (c *Config) AbsPath(r Root) string {
	if filepath.IsAbs(r.Path) {
		return filepath.Clean(r.Path)
	}
	return filepath.Clean(filepath.Join(c.Dir, r.Path))
}

// Excludes returns the effective exclude set, relative glob or base names.
func (c *Config) Excludes() []string {
	out := append([]string{}, DefaultExclude...)
	out = append(out, c.Exclude...)
	return out
}

// Enabled reports whether an optional asset family is indexed.  Defaults are on.
func (c *Config) Enabled(getter *bool) bool {
	if getter == nil {
		return true
	}
	return *getter
}

// NormalizeLanguages lowercases and deduplicates the language list.
func (c *Config) NormalizeLanguages() []string {
	seen := map[string]bool{}
	var out []string
	for _, l := range c.Languages {
		l = strings.ToLower(strings.TrimSpace(l))
		if l == "" || seen[l] {
			continue
		}
		seen[l] = true
		out = append(out, l)
	}
	return out
}

// LanguagePolicy documents the support level of a language front-end.
type LanguagePolicy struct {
	Name  string `json:"name"`
	Level string `json:"level"` // default | optional | planned
	Note  string `json:"note"`
}

// LanguagePolicies is the committed policy matrix.  C# is the default and fully
// supported front-end; Go covers the backend scenario when explicitly enabled;
// Lua is a reserved strategy and is not implemented yet.
var LanguagePolicies = []LanguagePolicy{
	{Name: "csharp", Level: "default", Note: "一级支持：类型/成员/调用/引用/继承图 + prefab/shader/配表"},
	{Name: "go", Level: "optional", Note: "前后端场景可用，需在 languages 中显式开启"},
	{Name: "rust", Level: "optional", Note: "Rust 模块/trait/impl/调用图，需显式开启"},
	{Name: "typescript", Level: "optional", Note: "TypeScript/JavaScript class/interface/函数/调用图，需显式开启"},
	{Name: "lua", Level: "planned", Note: "策略已预留，尚未实现（不会索引）"},
}

// EffectiveLanguages returns the configured language set; an empty list means
// the default policy, which is C# only.
func (c *Config) EffectiveLanguages() []string {
	langs := c.NormalizeLanguages()
	if len(langs) == 0 {
		return []string{"csharp"}
	}
	return langs
}

// LanguageEnabled reports whether a code language should be indexed.  Languages
// whose support level is "planned" are always rejected.
func (c *Config) LanguageEnabled(lang string) bool {
	lang = strings.ToLower(strings.TrimSpace(lang))
	for _, p := range LanguagePolicies {
		if p.Name == lang && p.Level == "planned" {
			return false
		}
	}
	for _, l := range c.EffectiveLanguages() {
		if l == lang {
			return true
		}
	}
	return false
}

// NearestExisting finds the closest ancestor of path that exists, for helpers.
func NearestExisting(path string) (string, bool) {
	cur := path
	for {
		if st, err := os.Stat(cur); err == nil && st.IsDir() {
			return cur, true
		}
		parent := filepath.Dir(cur)
		if parent == cur {
			return "", false
		}
		cur = parent
	}
}
