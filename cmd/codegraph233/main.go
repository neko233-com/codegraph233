// Command codegraph233 indexes a game project (C#/Go + prefab + shader +
// config tables) and serves the graph over MCP stdio.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"strings"

	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/detect"
	"github.com/neko233-com/codegraph233/internal/indexer"
	"github.com/neko233-com/codegraph233/internal/mcpserver"
	"github.com/neko233-com/codegraph233/internal/priority"
	"github.com/neko233-com/codegraph233/internal/projects"
	"github.com/neko233-com/codegraph233/internal/store"
)

const version = "0.1.0"

func main() {
	args := os.Args[1:]
	cmd := "serve"
	if len(args) > 0 && !strings.HasPrefix(args[0], "-") {
		cmd = args[0]
		args = args[1:]
	}
	switch cmd {
	case "serve", "mcp":
		runServe(args)
	case "index":
		runIndex(args)
	case "status":
		runStatus(args)
	case "init":
		runInit(args)
	case "install-mcp":
		runInstallMCP(args)
	case "compact":
		runCompact(args)
	case "register":
		runRegister(args)
	case "unregister":
		runUnregister(args)
	case "projects":
		runProjects(args)
	case "bench-parse":
		runBenchParse(args)
	case "bench-write":
		runBenchWrite(args)
	case "version", "--version", "-v":
		fmt.Println("codegraph233 " + version)
	case "help", "--help", "-h":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown command %q\n\n", cmd)
		usage()
		os.Exit(2)
	}
}

func usage() {
	fmt.Print(`codegraph233 — AST GraphRAG for game projects (Unity / Tuanjie / UE-ready)

Usage:
  codegraph233 [serve]                 MCP stdio server (default)
  codegraph233 index [--force]         Build/update the graph index
  codegraph233 status [--json]         Show index status and detection
  codegraph233 init [--frontend PATH] [--backend PATH]
  codegraph233 install-mcp [--clients codex,cursor,vscode,air,cowork] [--print]
  codegraph233 version

Config: .codegraph233.json at the project root; index: .codegraph233/index.db
`)
}

func loadConfig() (*config.Config, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return nil, err
	}
	return config.Load(cwd)
}

func runServe(args []string) {
	fs := flag.NewFlagSet("serve", flag.ExitOnError)
	root := fs.String("root", "", "project root (defaults to cwd / CODEGRAPH233_ROOT)")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("CODEGRAPH233_ROOT", *root)
	}
	cfg, err := loadConfig()
	if err != nil {
		fmt.Fprintln(os.Stderr, "config error:", err)
		os.Exit(1)
	}
	if err := mcpserver.Serve(cfg, os.Stdin, os.Stdout); err != nil {
		fmt.Fprintln(os.Stderr, "server error:", err)
		os.Exit(1)
	}
}

func runIndex(args []string) {
	fs := flag.NewFlagSet("index", flag.ExitOnError)
	force := fs.Bool("force", false, "re-parse every file")
	jobs := fs.Int("jobs", 0, "parallel workers")
	quiet := fs.Bool("quiet", false, "no progress output")
	asJSON := fs.Bool("json", false, "print stats as JSON")
	root := fs.String("root", "", "project root (defaults to cwd / CODEGRAPH233_ROOT)")
	safe := fs.Bool("safe", false, "use the durable WAL profile instead of the fast in-memory journal")
	profile := fs.String("profile", "", "override feature profile: full|game|code|minimal")
	writeMode := fs.String("write-mode", "single", "index write mode: single (one transaction, default)|batch|none")
	lowPriority := fs.Bool("low-priority", true, "run the index at below-normal process priority to avoid stealing CPU from the editor")
	cpus := fs.Int("cpus", 0, "limit worker threads (0 = all CPUs); lower it if the editor stutters")
	memLimitMB := fs.Int("mem-limit-mb", 0, "soft heap limit for the index run (0 = unlimited); keeps RSS predictable")
	lite := fs.Bool("lite", false, "low-memory profile: smaller SQLite cache/mmap and half the workers")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("CODEGRAPH233_ROOT", *root)
	}

	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	if *profile != "" {
		cfg.Profile = *profile
	}
	_ = writeMode
	if *lowPriority {
		if err := priority.SetLow(); err != nil {
			fmt.Fprintln(os.Stderr, "warn: set low priority:", err)
		}
	}
	if *cpus > 0 {
		runtime.GOMAXPROCS(*cpus)
		if *jobs == 0 {
			*jobs = *cpus
		}
	}
	if *lite {
		store.SetLite(true)
		if *jobs == 0 {
			n := runtime.NumCPU() / 2
			if n < 1 {
				n = 1
			}
			*jobs = n
		}
	}
	if *memLimitMB > 0 {
		debug.SetMemoryLimit(int64(*memLimitMB) << 20)
	}
	opts := indexer.Options{Force: *force, Jobs: *jobs, Quiet: *quiet, FastWrite: !*safe, WriteMode: *writeMode}
	if !*quiet {
		last := ""
		opts.Progress = func(done, total int, cur string) {
			if total == 0 {
				return
			}
			if done == total || cur == "" {
				fmt.Fprintf(os.Stderr, "\r[%d/%d] %-60s", done, total, "")
				if done == total {
					fmt.Fprintln(os.Stderr)
				}
				return
			}
			if cur != last {
				last = cur
				fmt.Fprintf(os.Stderr, "\r[%d/%d] %-60s", done, total, truncate(cur, 60))
			}
		}
	}
	stats, err := indexer.Run(cfg, opts)
	if err != nil {
		fatal(err)
	}
	if *asJSON {
		writeJSON(stats)
		return
	}
	det := detect.Detect(cfg)
	fmt.Printf("strategy: %s (%s)\n", det.Strategy.Name, det.Strategy.Confidence)
	for _, r := range det.Roots {
		state := "ok"
		if !r.Exists {
			state = "missing"
		}
		fmt.Printf("  root %-10s %-10s %-8s %s\n", r.Key, r.Role, state, r.Path)
	}
	fmt.Printf("files=%d changed=%d removed=%d symbols=%d refs=%d edges=%d assets=%d asset_edges=%d unresolved=%d %dms\n",
		stats.Files, stats.ChangedFiles, stats.RemovedFiles, stats.Symbols, stats.Refs,
		stats.Edges, stats.Assets, stats.AssetEdges, stats.Unresolved, stats.ElapsedMs)
	if len(stats.Errors) > 0 {
		fmt.Printf("errors: %d (first: %s)\n", len(stats.Errors), stats.Errors[0])
	}
}

func runStatus(args []string) {
	fs := flag.NewFlagSet("status", flag.ExitOnError)
	asJSON := fs.Bool("json", false, "print JSON")
	root := fs.String("root", "", "project root (defaults to cwd / CODEGRAPH233_ROOT)")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("CODEGRAPH233_ROOT", *root)
	}
	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	st := indexer.ReadStatus(cfg)
	if *asJSON {
		writeJSON(st)
		return
	}
	fmt.Printf("config:   %s\n", cfg.FilePath)
	fmt.Printf("data:     %s\n", st.DataDir)
	fmt.Printf("strategy: %s  engine: %s\n", st.Detection.Strategy.Name, st.Detection.Strategy.Engine)
	fmt.Printf("languages: %v (policy: csharp=default; go/rust/typescript=optional; lua=planned)\n", st.Languages)
	fmt.Printf("profile:   %s  features: code=%v prefabs=%v shaders=%v assets=%v meta=%v config=%v guid=%v\n",
		orDefault(st.Profile, "full"), st.Features.Code, st.Features.Prefabs, st.Features.Shaders,
		st.Features.Assets, st.Features.Meta, st.Features.ConfigTables, st.Features.GuidMap)
	for _, r := range st.Detection.Roots {
		state := "ok"
		if !r.Exists {
			state = "missing"
		}
		fmt.Printf("  root %-10s role=%-8s %-8s langs=%v path=%s\n", r.Key, r.Role, state, r.Languages, r.Path)
	}
	if !st.Exists {
		fmt.Println("index:    not built (run `codegraph233 index`)")
		return
	}
	fmt.Printf("indexed:  %s\n", st.IndexedAt)
	fmt.Printf("counts:   files=%d symbols=%d refs=%d edges=%d assets=%d asset_edges=%d\n",
		st.Files, st.Symbols, st.Refs, st.Edges, st.Assets, st.AssetEdges)
	if st.GuidEntries > 0 || st.GuidRefs > 0 {
		pct := 0.0
		if st.GuidRefs > 0 {
			pct = 100 * float64(st.GuidMapped) / float64(st.GuidRefs)
		}
		fmt.Printf("guid:     %d mappings, coverage %d/%d (%.1f%%)\n",
			st.GuidEntries, st.GuidMapped, st.GuidRefs, pct)
	}
	if st.Detection.Strategy.Notes != "" {
		fmt.Println("notes:    " + st.Detection.Strategy.Notes)
	}
}

func runInit(args []string) {
	fs := flag.NewFlagSet("init", flag.ExitOnError)
	frontend := fs.String("frontend", ".", "frontend/client root path")
	backend := fs.String("backend", "", "backend root path (optional)")
	shared := fs.String("shared", "", "shared root path (optional)")
	strategy := fs.String("strategy", "auto", "auto|unity|tuanjie|ue|generic")
	force := fs.Bool("force", false, "overwrite an existing config")
	profile := fs.String("profile", "full", "feature profile: full|game|code|minimal")
	_ = fs.Parse(args)

	cwd, err := os.Getwd()
	if err != nil {
		fatal(err)
	}
	path := filepath.Join(cwd, config.FileName)
	if _, err := os.Stat(path); err == nil && !*force {
		fmt.Printf("config already exists: %s (use --force to overwrite)\n", path)
		return
	}
	cfg := &config.Config{Version: 1, Strategy: *strategy, Dir: cwd, FilePath: path, Profile: *profile}
	cfg.Roots = append(cfg.Roots, config.Root{Key: "client", Path: *frontend, Role: "frontend"})
	if *backend != "" {
		cfg.Roots = append(cfg.Roots, config.Root{Key: "server", Path: *backend, Role: "backend"})
	} else {
		guess := filepath.Join("..", "server-project-sf-go")
		if st, err := os.Stat(filepath.Join(cwd, guess)); err == nil && st.IsDir() {
			cfg.Roots = append(cfg.Roots, config.Root{Key: "server", Path: guess, Role: "backend"})
		}
	}
	if *shared != "" {
		cfg.Roots = append(cfg.Roots, config.Root{Key: "shared", Path: *shared, Role: "shared"})
	}
	// record the detected language set so init-produced configs are ready to use
	det := detect.Detect(cfg)
	if len(det.Languages) > 0 {
		cfg.Languages = det.Languages
	}
	if err := config.Save(cfg); err != nil {
		fatal(err)
	}
	ensureGitignore(cwd)

	fmt.Printf("wrote %s\n", path)
	fmt.Printf("detected strategy: %s (%s)\n", det.Strategy.Name, det.Strategy.Confidence)
	for _, r := range det.Roots {
		fmt.Printf("  root %-8s exists=%-5v langs=%v\n", r.Key, r.Exists, r.Languages)
	}
	fmt.Println("next: codegraph233 index && codegraph233 install-mcp")
}

// ensureGitignore adds the index directory to .gitignore when missing.
func ensureGitignore(dir string) {
	path := filepath.Join(dir, ".gitignore")
	const entry = "/.codegraph233/"
	data, err := os.ReadFile(path)
	if err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			if strings.TrimSpace(line) == entry || strings.TrimSpace(line) == ".codegraph233/" {
				return
			}
		}
		if len(data) > 0 && !strings.HasSuffix(string(data), "\n") {
			data = append(data, '\n')
		}
		data = append(data, []byte("\n# codegraph233 本地索引（勿提交）\n"+entry+"\n")...)
		if err := os.WriteFile(path, data, 0o644); err != nil {
			fmt.Fprintln(os.Stderr, "warn: update .gitignore:", err)
		}
		return
	}
	_ = os.WriteFile(path, []byte("# codegraph233 本地索引（勿提交）\n"+entry+"\n"), 0o644)
}

// runCompact reclaims space and refreshes planner statistics.  It is optional:
// the index stays correct without it, it just gets physically smaller.
// runRegister adds a project root to the user-level registry so one global MCP
// process can serve it.
// runBenchParse parses all C# files and prints timing (no DB writes).  Used to
// compare front-end cost between implementations.
// runBenchWrite measures the storage engine by inserting synthetic rows with
// the same schema the indexer uses (Go: modernc pure-Go SQLite).
func runBenchWrite(args []string) {
	fs := flag.NewFlagSet("bench-write", flag.ExitOnError)
	refs := fs.Int("refs", 1000000, "reference rows")
	edges := fs.Int("edges", 800000, "edge rows")
	dir := fs.String("dir", "", "output directory (defaults to Temp)")
	_ = fs.Parse(args)
	out := *dir
	if out == "" {
		out = filepath.Join(os.TempDir(), "codegraph233-bench")
	}
	if err := os.MkdirAll(out, 0o755); err != nil {
		fatal(err)
	}
	res, err := indexer.BenchWrite(out, *refs, *edges)
	if err != nil {
		fatal(err)
	}
	data, _ := json.Marshal(res)
	fmt.Println(string(data))
}

func runBenchParse(args []string) {
	fs := flag.NewFlagSet("bench-parse", flag.ExitOnError)
	root := fs.String("root", "", "project root (defaults to cwd / CODEGRAPH233_ROOT)")
	jobs := fs.Int("jobs", 0, "parallel workers")
	typeDeps := fs.Bool("type-deps", false, "include type dependency edges")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("CODEGRAPH233_ROOT", *root)
	}
	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	res, err := indexer.BenchParse(cfg, *jobs, *typeDeps)
	if err != nil {
		fatal(err)
	}
	indexer.PrintBenchJSON(res)
}

func runRegister(args []string) {
	fs := flag.NewFlagSet("register", flag.ExitOnError)
	root := fs.String("root", "", "project root (defaults to cwd)")
	_ = fs.Parse(args)
	dir := *root
	if dir == "" {
		dir, _ = os.Getwd()
	}
	cfg, err := config.Load(dir)
	if err != nil {
		fatal(err)
	}
	if _, err := os.Stat(cfg.DBPath()); err != nil {
		fmt.Fprintln(os.Stderr, "提示: 该项目还没有索引，稍后运行 `codegraph233 index`")
	}
	reg, err := projects.Load()
	if err != nil {
		fatal(err)
	}
	added := reg.Add(cfg.Dir)
	if err := reg.Save(); err != nil {
		fatal(err)
	}
	if added {
		fmt.Printf("registered %s\n", cfg.Dir)
	} else {
		fmt.Printf("already registered %s\n", cfg.Dir)
	}
}

// runUnregister removes a project from the registry.
func runUnregister(args []string) {
	fs := flag.NewFlagSet("unregister", flag.ExitOnError)
	root := fs.String("root", "", "project root (defaults to cwd)")
	_ = fs.Parse(args)
	dir := *root
	if dir == "" {
		dir, _ = os.Getwd()
	}
	reg, err := projects.Load()
	if err != nil {
		fatal(err)
	}
	if !reg.Remove(dir) {
		fmt.Printf("not registered: %s\n", dir)
		return
	}
	if err := reg.Save(); err != nil {
		fatal(err)
	}
	fmt.Printf("unregistered %s\n", dir)
}

// runProjects lists every registered project with its index status.
func runProjects(args []string) {
	fs := flag.NewFlagSet("projects", flag.ExitOnError)
	asJSON := fs.Bool("json", false, "print JSON")
	_ = fs.Parse(args)
	reg, err := projects.Load()
	if err != nil {
		fatal(err)
	}
	type row struct {
		Name      string `json:"name"`
		Root      string `json:"root"`
		Indexed   bool   `json:"indexed"`
		IndexedAt string `json:"indexed_at,omitempty"`
		Files     int    `json:"files"`
		Symbols   int    `json:"symbols"`
	}
	var rows []row
	for _, p := range reg.Projects {
		cfg, cErr := config.Load(p.Root)
		if cErr != nil {
			continue
		}
		st := indexer.ReadStatus(cfg)
		rows = append(rows, row{
			Name: p.Name, Root: p.Root, Indexed: st.Exists,
			IndexedAt: st.IndexedAt, Files: st.Files, Symbols: st.Symbols,
		})
	}
	if *asJSON {
		writeJSON(rows)
		return
	}
	if len(rows) == 0 {
		fmt.Println("no registered projects; run `codegraph233 register` inside a project")
		return
	}
	for _, r := range rows {
		state := "not indexed"
		if r.Indexed {
			state = fmt.Sprintf("%d files / %d symbols, %s", r.Files, r.Symbols, r.IndexedAt)
		}
		fmt.Printf("%-24s %-52s %s\n", r.Name, r.Root, state)
	}
}

func runCompact(args []string) {
	fs := flag.NewFlagSet("compact", flag.ExitOnError)
	root := fs.String("root", "", "project root (defaults to cwd / CODEGRAPH233_ROOT)")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("CODEGRAPH233_ROOT", *root)
	}
	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	st, err := store.Open(cfg.DBPath())
	if err != nil {
		fatal(err)
	}
	defer st.Close()
	before, _ := os.Stat(cfg.DBPath())
	if _, err := st.DB().Exec(`PRAGMA optimize`); err != nil {
		fmt.Fprintln(os.Stderr, "warn optimize:", err)
	}
	if _, err := st.DB().Exec(`ANALYZE`); err != nil {
		fmt.Fprintln(os.Stderr, "warn analyze:", err)
	}
	if _, err := st.DB().Exec(`VACUUM`); err != nil {
		fmt.Fprintln(os.Stderr, "warn vacuum:", err)
	}
	after, _ := os.Stat(cfg.DBPath())
	if before != nil && after != nil {
		fmt.Printf("compacted: %.1f MB -> %.1f MB\n", float64(before.Size())/(1<<20), float64(after.Size())/(1<<20))
	}
}

func runInstallMCP(args []string) {
	fs := flag.NewFlagSet("install-mcp", flag.ExitOnError)
	clients := fs.String("clients", "codex,cursor,vscode,air,cowork,workbuddy,project", "comma separated clients (codex,cursor,vscode,air,cowork,workbuddy,project)")
	scope := fs.String("scope", "global", "global (recommended)|project|both")
	cleanProject := fs.Bool("clean-project", false, "remove project-level codegraph233 entries (other servers are kept)")
	printOnly := fs.Bool("print", false, "print snippets instead of writing files")
	exe := fs.String("exe", "codegraph233", "command used in MCP configs")
	_ = fs.Parse(args)

	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	projectServer := serverEntry(*exe, []string{"serve", "--root", cfg.Dir})
	globalServer := serverEntry(*exe, []string{"serve"})
	targets := mcpTargets(cfg.Dir, *scope, *clients)
	if len(targets) == 0 {
		fmt.Fprintln(os.Stderr, "no targets for scope="+*scope)
		return
	}
	for _, target := range targets {
		server := projectServer
		if target.global {
			server = globalServer
		}
		if *printOnly {
			fmt.Printf("# %s (%s, %s)\n%s\n\n", target.client, target.path, target.scopeName, snippet(target.client, server))
			continue
		}
		if err := writeClientConfig(target.client, target.path, server); err != nil {
			fmt.Fprintf(os.Stderr, "warn: %s: %v\n", target.client, err)
			continue
		}
		fmt.Printf("updated %s (%s, %s)\n", target.path, target.client, target.scopeName)
	}
	if *cleanProject {
		for _, raw := range strings.Split(*clients, ",") {
			client := strings.ToLower(strings.TrimSpace(raw))
			if client == "" {
				continue
			}
			p := projectConfigPath(client, cfg.Dir)
			if p == "" {
				continue
			}
			removed, err := removeClientConfig(client, p)
			if err != nil {
				fmt.Fprintf(os.Stderr, "warn: clean %s: %v\n", client, err)
				continue
			}
			if removed {
				fmt.Printf("removed project entry %s (%s)\n", p, client)
			}
		}
	}
}

// removeClientConfig deletes only the codegraph233 entry from a client config.
func removeClientConfig(client, path string) (bool, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return false, nil
	}
	if client == "codex" {
		text := stripTomlSection(string(data), "codegraph233")
		if text == string(data) {
			return false, nil
		}
		return true, os.WriteFile(path, []byte(text), 0o644)
	}
	existing := map[string]any{}
	if err := json.Unmarshal(data, &existing); err != nil {
		return false, err
	}
	key := "mcpServers"
	if client == "vscode" {
		key = "servers"
	}
	section, _ := existing[key].(map[string]any)
	if section == nil {
		return false, nil
	}
	if _, ok := section["codegraph233"]; !ok {
		return false, nil
	}
	delete(section, "codegraph233")
	out, err := json.MarshalIndent(existing, "", "  ")
	if err != nil {
		return false, err
	}
	return true, os.WriteFile(path, append(out, '\n'), 0o644)
}

func serverEntry(exe string, args []string) map[string]any {
	return map[string]any{"type": "stdio", "command": exe, "args": args}
}

type mcpTarget struct {
	client      string
	path        string
	scopeName   string
	global      bool
}

// mcpTargets resolves project-level and user-level config paths per client.
// Global entries omit --root so the server discovers the config from the
// client's working directory (project root).
func mcpTargets(projectDir, scope, clients string) []mcpTarget {
	home, _ := os.UserHomeDir()
	var out []mcpTarget
	for _, raw := range strings.Split(clients, ",") {
		client := strings.ToLower(strings.TrimSpace(raw))
		if client == "" {
			continue
		}
		if scope == "project" || scope == "both" {
			if p := projectConfigPath(client, projectDir); p != "" {
				out = append(out, mcpTarget{client: client, path: p, scopeName: "project"})
			}
		}
		if scope == "global" || scope == "both" {
			if p := globalConfigPath(client, home); p != "" {
				out = append(out, mcpTarget{client: client, path: p, scopeName: "global", global: true})
			}
		}
	}
	return out
}

func projectConfigPath(client, dir string) string {
	switch client {
	case "codex":
		return filepath.Join(dir, ".codex", "config.toml")
	case "cursor":
		return filepath.Join(dir, ".cursor", "mcp.json")
	case "vscode":
		return filepath.Join(dir, ".vscode", "mcp.json")
	case "air":
		return filepath.Join(dir, ".air", "mcp.json")
	case "cowork", "codely":
		return filepath.Join(dir, ".codely-cli", "settings.json")
	case "workbuddy":
		return filepath.Join(dir, ".workbuddy", "mcp.json")
	case "project":
		return filepath.Join(dir, ".mcp.json")
	}
	return ""
}

func globalConfigPath(client, home string) string {
	switch client {
	case "codex":
		return filepath.Join(home, ".codex", "config.toml")
	case "cursor":
		return filepath.Join(home, ".cursor", "mcp.json")
	case "vscode":
		return filepath.Join(vscodeUserDir(home), "mcp.json")
	case "air":
		return filepath.Join(home, ".air", "mcp.json")
	case "cowork", "codely":
		return filepath.Join(home, ".codely-cli", "settings.json")
	case "workbuddy":
		return filepath.Join(home, ".workbuddy", "mcp.json")
	}
	return ""
}

func vscodeUserDir(home string) string {
	if runtime.GOOS == "windows" {
		if app := os.Getenv("APPDATA"); app != "" {
			return filepath.Join(app, "Code", "User")
		}
	}
	if runtime.GOOS == "darwin" {
		return filepath.Join(home, "Library", "Application Support", "Code", "User")
	}
	return filepath.Join(home, ".config", "Code", "User")
}

func snippet(client string, server map[string]any) string {
	switch client {
	case "codex":
		root := ""
		args := server["args"].([]string)
		if len(args) >= 3 {
			root = args[2]
		}
		line := `[mcp_servers.codegraph233]`
		body := "command = \"" + server["command"].(string) + "\""
		if root == "" {
			return line + "\n" + body + "\nargs = [\"serve\"]"
		}
		return line + "\n" + body + "\nargs = [\"serve\", \"--root\", \"" + strings.ReplaceAll(root, `\`, `\\`) + "\"]"
	case "vscode":
		b, _ := json.MarshalIndent(map[string]any{"servers": map[string]any{"codegraph233": server}}, "", "  ")
		return string(b)
	default:
		b, _ := json.MarshalIndent(map[string]any{"mcpServers": map[string]any{"codegraph233": server}}, "", "  ")
		return string(b)
	}
}
func writeClientConfig(client, path string, server map[string]any) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	if client == "codex" {
		data, _ := os.ReadFile(path)
		text := stripTomlSection(stripLegacyToml(string(data)), "codegraph233")
		args := server["args"].([]string)
		line := "\n[mcp_servers.codegraph233]\ncommand = \"" + server["command"].(string) + "\"\nargs = ["
		for i, a := range args {
			if i > 0 {
				line += ", "
			}
			line += "\"" + strings.ReplaceAll(a, `\`, `\\`) + "\""
		}
		line += "]\n"
		if text != "" && !strings.HasSuffix(text, "\n") {
			text += "\n"
		}
		return os.WriteFile(path, []byte(text+line), 0o644)
	}
	existing := map[string]any{}
	if data, err := os.ReadFile(path); err == nil && len(data) > 0 {
		if err := json.Unmarshal(data, &existing); err != nil {
			return fmt.Errorf("existing file is not valid JSON: %w", err)
		}
	}
	key := "mcpServers"
	if client == "vscode" {
		key = "servers"
	}
	section, _ := existing[key].(map[string]any)
	if section == nil {
		section = map[string]any{}
		existing[key] = section
	}
	delete(section, "game-project-graph")
	delete(section, "game_project_graph")
	section["codegraph233"] = server
	data, err := json.MarshalIndent(existing, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0o644)
}

// stripTomlSection removes [mcp_servers.<name>] and its child tables so
// re-installing (project <-> global) updates args instead of duplicating.
func stripTomlSection(text, name string) string {
	lines := strings.Split(text, "\n")
	out := make([]string, 0, len(lines))
	skip := false
	prefix := "[mcp_servers." + name
	for _, line := range lines {
		t := strings.TrimSpace(line)
		if strings.HasPrefix(t, "[") {
			skip = strings.HasPrefix(t, prefix)
			if skip {
				continue
			}
		}
		if !skip {
			out = append(out, line)
		}
	}
	return strings.TrimRight(strings.Join(out, "\n"), "\n") + "\n"
}
func writeJSON(v any) {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		fatal(err)
	}
	fmt.Println(string(data))
}

func orDefault(s, def string) string {
	if s == "" {
		return def
	}
	return s
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n-3] + "..."
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, "error:", err)
	os.Exit(1)
}

var _ = runtime.GOOS

// stripLegacyToml removes the pre-rename [mcp_servers.game_project_graph] section.
func stripLegacyToml(text string) string {
	lines := strings.Split(text, "\n")
	out := make([]string, 0, len(lines))
	skip := false
	for _, line := range lines {
		t := strings.TrimSpace(line)
		if strings.HasPrefix(t, "[") {
			skip = strings.HasPrefix(t, "[mcp_servers.game_project_graph")
			if skip {
				continue
			}
		}
		if !skip {
			out = append(out, line)
		}
	}
	return strings.Join(out, "\n")
}

