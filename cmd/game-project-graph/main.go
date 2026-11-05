// Command game-project-graph indexes a game project (C#/Go + prefab + shader +
// config tables) and serves the graph over MCP stdio.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/neko233-com/game-project-graph/internal/config"
	"github.com/neko233-com/game-project-graph/internal/detect"
	"github.com/neko233-com/game-project-graph/internal/indexer"
	"github.com/neko233-com/game-project-graph/internal/mcpserver"
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
	case "version", "--version", "-v":
		fmt.Println("game-project-graph " + version)
	case "help", "--help", "-h":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown command %q\n\n", cmd)
		usage()
		os.Exit(2)
	}
}

func usage() {
	fmt.Print(`game-project-graph — AST GraphRAG for game projects (Unity / Tuanjie / UE-ready)

Usage:
  game-project-graph [serve]                 MCP stdio server (default)
  game-project-graph index [--force]         Build/update the graph index
  game-project-graph status [--json]         Show index status and detection
  game-project-graph init [--frontend PATH] [--backend PATH]
  game-project-graph install-mcp [--clients codex,cursor,vscode,air,cowork] [--print]
  game-project-graph version

Config: .game-project-graph.json at the project root; index: .game-project-graph/index.db
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
	root := fs.String("root", "", "project root (defaults to cwd / GAME_PROJECT_GRAPH_ROOT)")
	_ = fs.Parse(args)
	if *root != "" {
		os.Setenv("GAME_PROJECT_GRAPH_ROOT", *root)
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
	_ = fs.Parse(args)

	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	opts := indexer.Options{Force: *force, Jobs: *jobs, Quiet: *quiet}
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
	_ = fs.Parse(args)
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
	for _, r := range st.Detection.Roots {
		state := "ok"
		if !r.Exists {
			state = "missing"
		}
		fmt.Printf("  root %-10s role=%-8s %-8s langs=%v path=%s\n", r.Key, r.Role, state, r.Languages, r.Path)
	}
	if !st.Exists {
		fmt.Println("index:    not built (run `game-project-graph index`)")
		return
	}
	fmt.Printf("indexed:  %s\n", st.IndexedAt)
	fmt.Printf("counts:   files=%d symbols=%d refs=%d edges=%d assets=%d asset_edges=%d\n",
		st.Files, st.Symbols, st.Refs, st.Edges, st.Assets, st.AssetEdges)
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
	cfg := &config.Config{Version: 1, Strategy: *strategy, Dir: cwd, FilePath: path}
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
	if err := config.Save(cfg); err != nil {
		fatal(err)
	}
	ensureGitignore(cwd)

	det := detect.Detect(cfg)
	fmt.Printf("wrote %s\n", path)
	fmt.Printf("detected strategy: %s (%s)\n", det.Strategy.Name, det.Strategy.Confidence)
	for _, r := range det.Roots {
		fmt.Printf("  root %-8s exists=%-5v langs=%v\n", r.Key, r.Exists, r.Languages)
	}
	fmt.Println("next: game-project-graph index && game-project-graph install-mcp")
}

// ensureGitignore adds the index directory to .gitignore when missing.
func ensureGitignore(dir string) {
	path := filepath.Join(dir, ".gitignore")
	const entry = "/.game-project-graph/"
	data, err := os.ReadFile(path)
	if err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			if strings.TrimSpace(line) == entry || strings.TrimSpace(line) == ".game-project-graph/" {
				return
			}
		}
		if len(data) > 0 && !strings.HasSuffix(string(data), "\n") {
			data = append(data, '\n')
		}
		data = append(data, []byte("\n# game-project-graph 本地索引（勿提交）\n"+entry+"\n")...)
		if err := os.WriteFile(path, data, 0o644); err != nil {
			fmt.Fprintln(os.Stderr, "warn: update .gitignore:", err)
		}
		return
	}
	_ = os.WriteFile(path, []byte("# game-project-graph 本地索引（勿提交）\n"+entry+"\n"), 0o644)
}

func runInstallMCP(args []string) {
	fs := flag.NewFlagSet("install-mcp", flag.ExitOnError)
	clients := fs.String("clients", "codex,cursor,vscode,air,cowork,workbuddy,project", "comma separated clients (codex,cursor,vscode,air,cowork,workbuddy,project)")
	printOnly := fs.Bool("print", false, "print snippets instead of writing files")
	exe := fs.String("exe", "game-project-graph", "command used in MCP configs")
	_ = fs.Parse(args)

	cfg, err := loadConfig()
	if err != nil {
		fatal(err)
	}
	rootDir := cfg.Dir
	server := map[string]any{
		"type":    "stdio",
		"command": *exe,
		"args":    []string{"serve", "--root", rootDir},
	}
	vsServer := map[string]any{
		"type":    "stdio",
		"command": *exe,
		"args":    []string{"serve", "--root", rootDir},
	}
	for _, raw := range strings.Split(*clients, ",") {
		client := strings.TrimSpace(strings.ToLower(raw))
		if client == "" {
			continue
		}
		var target string
		switch client {
		case "codex":
			target = filepath.Join(rootDir, ".codex", "config.toml")
		case "cursor":
			target = filepath.Join(rootDir, ".cursor", "mcp.json")
		case "vscode":
			target = filepath.Join(rootDir, ".vscode", "mcp.json")
		case "air":
			target = filepath.Join(rootDir, ".air", "mcp.json")
		case "cowork", "codely":
			target = filepath.Join(rootDir, ".codely-cli", "settings.json")
		case "workbuddy":
			target = filepath.Join(rootDir, ".workbuddy", "mcp.json")
		case "project":
			target = filepath.Join(rootDir, ".mcp.json")
		default:
			fmt.Fprintf(os.Stderr, "skip unknown client %q\n", client)
			continue
		}
		if *printOnly {
			fmt.Printf("# %s (%s)\n%s\n\n", client, target, snippet(client, server, vsServer))
			continue
		}
		if err := writeClientConfig(client, target, server, vsServer); err != nil {
			fmt.Fprintf(os.Stderr, "warn: %s: %v\n", client, err)
			continue
		}
		fmt.Printf("updated %s (%s)\n", target, client)
	}
}

func snippet(client string, server, vsServer map[string]any) string {
	switch client {
	case "codex":
		return `[mcp_servers.game_project_graph]
command = "` + server["command"].(string) + `"
args = ["serve", "--root", "` + strings.ReplaceAll(server["args"].([]string)[2], `\`, `\\`) + `"]`
	case "vscode":
		b, _ := json.MarshalIndent(map[string]any{"servers": map[string]any{"game-project-graph": vsServer}}, "", "  ")
		return string(b)
	default:
		b, _ := json.MarshalIndent(map[string]any{"mcpServers": map[string]any{"game-project-graph": server}}, "", "  ")
		return string(b)
	}
}

func writeClientConfig(client, path string, server, vsServer map[string]any) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	if client == "codex" {
		data, _ := os.ReadFile(path)
		text := string(data)
		if strings.Contains(text, "[mcp_servers.game_project_graph]") {
			return nil
		}
		root := server["args"].([]string)[2]
		block := "\n[mcp_servers.game_project_graph]\ncommand = \"" + server["command"].(string) +
			"\"\nargs = [\"serve\", \"--root\", \"" + strings.ReplaceAll(root, `\`, `\\`) + "\"]\n"
		return os.WriteFile(path, append(data, []byte(block)...), 0o644)
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
	if client == "vscode" {
		section["game-project-graph"] = vsServer
	} else {
		section["game-project-graph"] = server
	}
	data, err := json.MarshalIndent(existing, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0o644)
}

func writeJSON(v any) {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		fatal(err)
	}
	fmt.Println(string(data))
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
