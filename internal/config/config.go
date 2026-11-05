// Package config loads .game-project-graph.json and resolves multi-root layouts.
package config

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const FileName = ".game-project-graph.json"

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
	"node_modules", ".codely-cli", ".codex", ".cursor", ".game-project-graph",
	"Bundles", "StreamingAssets", "__pycache__", ".vs", ".idea", "DerivedDataCache",
	"Binaries", "Intermediate", "Saved", ".gradle",
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
	if env := os.Getenv("GAME_PROJECT_GRAPH_ROOT"); env != "" {
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
	return filepath.Join(c.Dir, ".game-project-graph")
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

// LanguageEnabled reports whether a language was restricted away.
func (c *Config) LanguageEnabled(lang string) bool {
	langs := c.NormalizeLanguages()
	if len(langs) == 0 {
		return true
	}
	for _, l := range langs {
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
