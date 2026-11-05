package indexer

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/lang/csharp"
)

// BenchResult is the parse-only benchmark output (no DB writes).
type BenchResult struct {
	Files    int   `json:"files"`
	Symbols  int   `json:"symbols"`
	Refs     int   `json:"refs"`
	Bytes    int64 `json:"bytes"`
	ElapsedMS int64 `json:"elapsed_ms"`
	Jobs     int   `json:"jobs"`
	MaxRSSKB int64 `json:"max_rss_kb,omitempty"`
}

// BenchParse walks a project's C# files and parses them without touching the
// database.  It exists to compare front-end cost across implementations.
func BenchParse(cfg *config.Config, jobs int, typeDeps bool) (*BenchResult, error) {
	if jobs <= 0 {
		jobs = runtime.NumCPU()
	}
	var files []string
	var totalBytes int64
	// Use the generic default excludes only (not project overrides) so the
	// benchmark parses the same file set in every implementation.
	excludes := config.DefaultExclude
	roots := cfg.Roots
	if len(roots) > 1 {
		roots = roots[:1]
	}
	for _, r := range roots {
		rootDir := cfg.AbsPath(r)
		_ = filepath.WalkDir(rootDir, func(p string, d os.DirEntry, err error) error {
			if err != nil {
				return nil
			}
			rel, relErr := filepath.Rel(rootDir, p)
			if relErr != nil {
				return nil
			}
			rel = filepath.ToSlash(rel)
			if d.IsDir() {
				if p != rootDir && isExcluded(rel, d.Name(), excludes) {
					return filepath.SkipDir
				}
				return nil
			}
			if strings.EqualFold(filepath.Ext(p), ".cs") {
				if info, e := d.Info(); e == nil {
					files = append(files, p)
					totalBytes += info.Size()
				}
			}
			return nil
		})
	}

	res := &BenchResult{Files: len(files), Jobs: jobs}
	start := time.Now()
	var mu sync.Mutex
	var wg sync.WaitGroup
	ch := make(chan string, jobs*4)
	for w := 0; w < jobs; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for p := range ch {
				data, err := os.ReadFile(p)
				if err != nil {
					continue
				}
				rel, _ := filepath.Rel(cfg.Dir, p)
				r := csharp.Parse(filepath.ToSlash(rel), "bench", data, typeDeps)
				mu.Lock()
				res.Symbols += len(r.Symbols)
				res.Refs += len(r.Refs)
				mu.Unlock()
			}
		}()
	}
	for _, f := range files {
		ch <- f
	}
	close(ch)
	wg.Wait()
	res.ElapsedMS = time.Since(start).Milliseconds()
	res.Bytes = totalBytes
	res.MaxRSSKB = currentRSSKB()
	return res, nil
}

// PrintBenchJSON marshals the benchmark result.
func PrintBenchJSON(r *BenchResult) {
	data, _ := json.Marshal(r)
	fmt.Println(string(data))
}
