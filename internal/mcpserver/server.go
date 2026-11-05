// Package mcpserver implements a dependency-free MCP server over stdio
// (JSON-RPC 2.0, newline-delimited) exposing the project graph tools.
package mcpserver

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"sync"

	"github.com/neko233-com/game-project-graph/internal/config"
	"github.com/neko233-com/game-project-graph/internal/indexer"
	"github.com/neko233-com/game-project-graph/internal/query"
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

// Server serves one project.
type Server struct {
	cfg *config.Config
	eng *query.Engine
	out *bufio.Writer
	mu  sync.Mutex
}

// Serve runs the stdio loop until EOF.
func Serve(cfg *config.Config, in io.Reader, out io.Writer) error {
	s := &Server{cfg: cfg, out: bufio.NewWriterSize(out, 1<<20)}
	defer s.closeEngine()
	// A missing index is not fatal: gpg_index builds it on demand and other
	// tools return a clear hint.
	if eng, err := query.Open(cfg); err == nil {
		s.eng = eng
	}
	dec := json.NewDecoder(bufio.NewReaderSize(in, 1<<20))
	for {
		var req request
		if err := dec.Decode(&req); err != nil {
			if err == io.EOF {
				return nil
			}
			fmt.Fprintf(os.Stderr, "game-project-graph: decode error: %v\n", err)
			continue
		}
		s.handle(req)
	}
}

func (s *Server) closeEngine() {
	if s.eng != nil {
		s.eng.Close()
		s.eng = nil
	}
}

func (s *Server) handle(req request) {
	if req.ID == nil {
		// notification (initialized, cancelled, ...) — no response
		return
	}
	switch req.Method {
	case "initialize":
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{
			"protocolVersion": "2024-11-05",
			"capabilities": map[string]any{
				"tools": map[string]any{"listChanged": false},
			},
			"serverInfo": map[string]any{
				"name":    "game-project-graph",
				"version": serverVersion,
			},
			"instructions": "AST GraphRAG for game projects. Call gpg_status first; build the index with gpg_index (incremental), then use gpg_find_symbol / gpg_call_graph / gpg_asset_* / gpg_config_*.",
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
		text := result
		s.write(response{JSONRPC: "2.0", ID: req.ID, Result: map[string]any{
			"content": []map[string]any{{"type": "text", "text": text}},
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

// ensureIndex builds the index on demand.
func (s *Server) ensureIndex(args map[string]any) error {
	force := argBool(args, "force", false)
	stats, err := indexer.Run(s.cfg, indexer.Options{Force: force, Quiet: true, Jobs: argInt(args, "jobs", 0)})
	if err != nil {
		return err
	}
	if s.eng != nil {
		s.eng.Close()
	}
	eng, err := query.Open(s.cfg)
	if err != nil {
		return err
	}
	s.eng = eng
	_ = stats
	return nil
}

func (s *Server) requireEngine() (*query.Engine, error) {
	if s.eng == nil {
		if _, err := os.Stat(s.cfg.DBPath()); err != nil {
			return nil, fmt.Errorf("索引尚未建立：请先调用 gpg_index（或运行 `game-project-graph index`）")
		}
		eng, err := query.Open(s.cfg)
		if err != nil {
			return nil, err
		}
		s.eng = eng
	}
	return s.eng, nil
}
