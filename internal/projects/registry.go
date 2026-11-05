// Package projects keeps a user-level registry of indexed projects so one MCP
// server can serve several repositories (each with its own .codegraph233.json
// and index database).
package projects

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// Project is one registered root.
type Project struct {
	Root    string `json:"root"`
	Name    string `json:"name"`
	AddedAt string `json:"added_at,omitempty"`
}

// Registry is the on-disk file.
type Registry struct {
	Projects []Project `json:"projects"`
}

// Path returns the registry location (~/.codegraph233/projects.json).
func Path() string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		home = "."
	}
	return filepath.Join(home, ".codegraph233", "projects.json")
}

// Load reads the registry (missing file -> empty registry).
func Load() (*Registry, error) {
	data, err := os.ReadFile(Path())
	if err != nil {
		if os.IsNotExist(err) {
			return &Registry{}, nil
		}
		return nil, err
	}
	r := &Registry{}
	if err := json.Unmarshal(data, r); err != nil {
		return nil, err
	}
	return r, nil
}

// Save writes the registry back.
func (r *Registry) Save() error {
	p := Path()
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(p, append(data, '\n'), 0o644)
}

func normalize(root string) string {
	if abs, err := filepath.Abs(root); err == nil {
		root = abs
	}
	return filepath.Clean(root)
}

// Add registers a project root (idempotent).  Returns true when it was new.
func (r *Registry) Add(root string) bool {
	root = normalize(root)
	for _, p := range r.Projects {
		if strings.EqualFold(normalize(p.Root), root) {
			return false
		}
	}
	name := filepath.Base(root)
	r.Projects = append(r.Projects, Project{Root: root, Name: name, AddedAt: time.Now().Format(time.RFC3339)})
	sort.Slice(r.Projects, func(i, j int) bool { return strings.ToLower(r.Projects[i].Name) < strings.ToLower(r.Projects[j].Name) })
	return true
}

// Remove unregisters a root.  Returns true when it existed.
func (r *Registry) Remove(root string) bool {
	root = normalize(root)
	out := r.Projects[:0]
	removed := false
	for _, p := range r.Projects {
		if strings.EqualFold(normalize(p.Root), root) {
			removed = true
			continue
		}
		out = append(out, p)
	}
	r.Projects = out
	return removed
}

// Roots returns every registered root.
func (r *Registry) Roots() []string {
	out := make([]string, 0, len(r.Projects))
	for _, p := range r.Projects {
		out = append(out, p.Root)
	}
	return out
}
