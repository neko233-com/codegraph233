// Package mcpserver implements a dependency-free MCP server over stdio
// (JSON-RPC 2.0, newline-delimited) exposing the project graph tools.
//
// One server process can serve several projects: the registry at
// ~/.codegraph233/projects.json is loaded at startup and every tool accepts an
// optional `project` selector (name or root path).  Each project gets its own
// lazily-opened index database, so a single global MCP entry works for all
// repositories.
package mcpserver

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/indexer"
	"github.com/neko233-com/codegraph233/internal/projects"
	"github.com/neko233-com/codegraph233/internal/query"
)

const serverVersion = "0.1.0"

type request struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params,omitempty"`
}

type response struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Result  any             `json:"result,omitempty"`
	Error   *rpcError       `json:"error,omitempty"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

// Server serves one or more projects.
type Server struct {
	cfg         *config.Config
	eng         *query.Engine
	engines     map[string]*query.Engine
	cfgs        map[string]*config.Config
	registered  []projects.Project
	defaultRoot string
	out         *bufio.Writer
	mu          sync.Mutex
}

// Serve runs the stdio loop until EOF.
func Serve(cfg *config.Config, in io.Reader, out io.Writer) error {
	s := &Server{
		cfg:         cfg,
		engines:     map[string]*query.Engine{},
		cfgs:        map[string]*config.Config{},
		defaultRoot: cfg.Dir,
		out:         bufio.NewWriterSize(out, 1<<20),
	}
	defer s.closeEngines()

	if reg, err := projects.Load(); err == nil {
		s.registered = reg.Projects
	}
	// Optional env supplement: CODEGRAPH233_PROJECTS=A;B;C
	if env := os.Getenv("CODEGRAPH233_PROJECTS"); env != "" {
		for _, r := range strings.Split(env, string(os.PathListSeparator)) {
			r = strings.TrimSpace(r)
			if r == "" {
				continue
			}
			s.registered = append(s.registered, projects.Project{Root: r, Name: filepath.Base(filepath.Clean(r))})
		}
	}
	// The launch project is always available even when not registered.
	if eng, err := query.Open(cfg); err == nil {
		s.eng = eng
		s.engines[fileKey(cfg.Dir)] = eng
		s.cfgs[fileKey(cfg.Dir)] = cfg
	}

	dec := json.NewDecoder(bufio.NewReaderSize(in, 1<<20))
	for {
		var req request
		if err := dec.Decode(&req); err != nil {
			if err == io.EOF {
				return nil
			}
			fmt.Fprintf(os.Stderr, "codegraph233: decode error: %v\n", err)
			continue
		}
		s.handle(req)
	}
}

func fileKey(p string) string {
	if abs, err := filepath.Abs(p); err == nil {
		p = abs
	}
	return strings.ToLower(filepath.Clean(p))
}

func (s *Server) closeEngines() {
	for _, e := range s.engines {
		e.Close()
	}
	s.engines = map[string]*query.Engine{}
	s.eng = nil
}

func (s *Server) handle(req request) {
	if req.ID == nil {
		return
	}
	switch req.Method {
	case "initialize":
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{
			"protocolVersion": "2024-11-05",
			"capabilities":    map[string]any{"tools": map[string]any{"listChanged": false}},
			"serverInfo":      map[string]any{"name": "codegraph233", "version": serverVersion},
			"instructions": "AST GraphRAG for game projects. Call cg233_projects to see available projects, " +
				"cg233_status for the current one, cg233_index (incremental) to build. " +
				"Every tool accepts an optional `project` selector (name or root path).",
		}})
	case "ping":
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{}})
	case "tools/list":
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{"tools": toolDefs()}})
	case "tools/call":
		var p struct {
			Name      string         `json:"name"`
			Arguments map[string]any `json:"arguments"`
		}
		if err := json.Unmarshal(req.Params, &p); err != nil {
			s.write(response{JSONRPC: "2.0", ID: req.ID, Error: &rpcError{Code: -32602, Message: "invalid params: " + err.Error()}})
			return
		}
		result, err := s.call(p.Name, p.Arguments)
		if err != nil {
			s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{
				"content": []map[string]any{{"type": "text", "text": "ERROR: " + err.Error()}},
				"isError": true,
			}})
			return
		}
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{
			"content": []map[string]any{{"type": "text", "text": result}},
		}})
	default:
		s.write(response{JSONRPC: "2.0", ID: req.ID, Error: &rpcError{Code: -32601, Message: "method not found: " + req.Method}})
	}
}

func (s *Server) write(resp response) {
	s.mu.Lock()
	defer s.mu.Unlock()
	data, err := json.Marshal(resp)
	if err != nil {
		return
	}
	s.out.Write(data)
	s.out.WriteByte('\n')
	s.out.Flush()
}

// resolveRoot maps a selector (empty | name | root | path fragment) to a root.
func (s *Server) resolveRoot(sel string) string {
	sel = strings.TrimSpace(sel)
	if sel == "" {
		return s.defaultRoot
	}
	lower := strings.ToLower(sel)
	for _, p := range s.registered {
		if strings.EqualFold(p.Name, sel) || fileKey(p.Root) == fileKey(sel) {
			return p.Root
		}
	}
	for _, p := range s.registered {
		if strings.Contains(strings.ToLower(p.Root), lower) {
			return p.Root
		}
	}
	if fileKey(s.defaultRoot) == fileKey(sel) {
		return s.defaultRoot
	}
	// Treat it as a path fragment of the default project's siblings.
	if dir, ok := config.Find(sel); ok {
		return dir
	}
	return sel
}

// cfgFor loads (and caches) the config of a project.
func (s *Server) cfgFor(root string) (*config.Config, error) {
	key := fileKey(root)
	if c, ok := s.cfgs[key]; ok {
		return c, nil
	}
	cfg, err := config.LoadDir(root)
	if err != nil {
		return nil, fmt.Errorf("加载项目配置失败 %s: %w", root, err)
	}
	s.cfgs[key] = cfg
	if s.defaultRoot == "" {
		s.defaultRoot = cfg.Dir
	}
	return cfg, nil
}

// engineFor opens (and caches) the index of the selected project.
func (s *Server) engineFor(sel string) (*query.Engine, error) {
	root := s.resolveRoot(sel)
	key := fileKey(root)
	if e, ok := s.engines[key]; ok {
		return e, nil
	}
	if _, err := os.Stat(root); err != nil {
		return nil, fmt.Errorf("项目不存在: %s（用 cg233_projects 查看已注册项目）", root)
	}
	cfg, err := s.cfgFor(root)
	if err != nil {
		return nil, err
	}
	eng, err := query.Open(cfg)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", cfg.Dir, err)
	}
	s.engines[key] = eng
	if s.eng == nil {
		s.eng = eng
	}
	return eng, nil
}

// requireEngine keeps the launch project's engine (compat for handlers that do
// not take a selector).
func (s *Server) requireEngine() (*query.Engine, error) {
	return s.engineFor("")
}

// ensureIndex builds the selected project's index on demand.
func (s *Server) ensureIndex(args map[string]any) error {
	cfg, err := s.cfgFor(s.resolveRoot(argStr(args, "project", "")))
	if err != nil {
		return err
	}
	force := argBool(args, "force", false)
	_, err = indexer.Run(cfg, indexer.Options{
		Force: force, Quiet: true, Jobs: argInt(args, "jobs", 0), FastWrite: false,
	})
	if err != nil {
		return err
	}
	key := fileKey(cfg.Dir)
	if old, ok := s.engines[key]; ok {
		old.Close()
		delete(s.engines, key)
	}
	eng, err := query.Open(cfg)
	if err != nil {
		return err
	}
	s.engines[key] = eng
	return nil
}

// projectRow is one entry of cg233_projects.
type projectRow struct {
	Name      string `json:"name"`
	Root      string `json:"root"`
	Default   bool   `json:"default"`
	Indexed   bool   `json:"indexed"`
	IndexedAt string `json:"indexed_at,omitempty"`
	Files     int    `json:"files"`
	Symbols   int    `json:"symbols"`
}

func (s *Server) projectRows() []projectRow {
	seen := map[string]bool{}
	var out []projectRow
	add := func(root string) {
		key := fileKey(root)
		if seen[key] {
			return
		}
		seen[key] = true
		row := projectRow{Name: filepath.Base(filepath.Clean(root)), Root: root, Default: key == fileKey(s.defaultRoot)}
		cfg, err := s.cfgFor(root)
		if err == nil {
			if st, sErr := os.Stat(cfg.DBPath()); sErr == nil && !st.IsDir() {
				row.Indexed = true
			}
			if row.Indexed {
				if eng, eErr := s.engineFor(row.Name); eErr == nil {
					if h, hErr := eng.HealthReport(""); hErr == nil {
						row.Files, row.Symbols, row.IndexedAt = h.Files, h.Symbols, h.IndexedAt
					}
				}
			}
		}
		out = append(out, row)
	}
	add(s.defaultRoot)
	for _, p := range s.registered {
		add(p.Root)
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Default != out[j].Default {
			return out[i].Default
		}
		return strings.ToLower(out[i].Name) < strings.ToLower(out[j].Name)
	})
	return out
}
