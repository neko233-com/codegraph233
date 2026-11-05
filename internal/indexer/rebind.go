package indexer

import (
	"fmt"
	"os"
	"runtime"
	"sync"
	"time"

	"github.com/neko233-com/codegraph233/internal/model"
	"github.com/neko233-com/codegraph233/internal/store"
)

const refPageSize = 100000

// refResolution is the outcome of binding one reference.
type refResolution struct {
	target int64
	conf   string
	ok     bool
}

// parallelResolve binds refs in parallel chunks.  binder and srcByID are
// read-only, so no locking is needed.
func parallelResolve(b *binder, srcByID map[int64]store.SymbolRow, refs []store.LoadedRef, jobs int) []refResolution {
	out := make([]refResolution, len(refs))
	if len(refs) == 0 {
		return out
	}
	if jobs <= 0 {
		jobs = runtime.NumCPU()
	}
	if jobs > len(refs) {
		jobs = len(refs)
	}
	chunk := (len(refs) + jobs - 1) / jobs
	var wg sync.WaitGroup
	for w := 0; w < jobs; w++ {
		start := w * chunk
		end := start + chunk
		if start >= len(refs) {
			break
		}
		if end > len(refs) {
			end = len(refs)
		}
		wg.Add(1)
		go func(s, e int) {
			defer wg.Done()
			for i := s; i < e; i++ {
				r := refs[i]
				if r.SrcID == 0 {
					continue
				}
				src, ok := srcByID[r.SrcID]
				if !ok {
					continue
				}
				res, ok := b.resolve(r, src)
				if !ok {
					continue
				}
				out[i] = refResolution{target: res.target.ID, conf: res.confidence, ok: true}
			}
		}(start, end)
	}
	wg.Wait()
	return out
}

// rebindIndex rebuilds the symbol edge table from stored references without
// touching unchanged ref targets.  Work is paged (flat memory) and resolved in
// parallel; only rows whose target actually changed are written back.
func rebindIndex(st *store.Store, jobs int, quiet bool) (int, int, error) {
	t0 := time.Now()
	if err := st.ClearEdges(); err != nil {
		return 0, 0, err
	}
	syms, err := st.LoadSymbolRows()
	if err != nil {
		return 0, 0, err
	}
	if !quiet {
		fmt.Fprintf(os.Stderr, "[bind] loaded %d symbols (%.1fs)\n", len(syms), time.Since(t0).Seconds())
	}
	if len(syms) == 0 {
		return 0, 0, nil
	}
	b := newBinder(syms)
	srcByID := make(map[int64]store.SymbolRow, len(syms))
	for _, s := range syms {
		srcByID[s.ID] = s
	}

	kindIdx := map[string]int64{}
	seen := map[[3]int64]bool{}
	cursor := int64(0)
	totalEdges, unresolved, totalRefs, changedTargets := 0, 0, 0, 0

	for {
		refs, err := st.LoadRefsPage(cursor, refPageSize)
		if err != nil {
			return 0, 0, err
		}
		if len(refs) == 0 {
			break
		}
		cursor = refs[len(refs)-1].ID
		totalRefs += len(refs)

		results := parallelResolve(b, srcByID, refs, jobs)
		updates := make([]store.RefTargetUpdate, 0, 1024)
		edges := make([]model.ResolvedEdge, 0, len(refs))

		for i := range refs {
			r := refs[i]
			res := results[i]
			if !res.ok {
				unresolved++
				if r.Target != 0 {
					updates = append(updates, store.RefTargetUpdate{ID: r.ID, Target: 0, Confidence: "unresolved"})
					changedTargets++
				}
				continue
			}
			if r.Target != res.target {
				updates = append(updates, store.RefTargetUpdate{ID: r.ID, Target: res.target, Confidence: res.conf})
				changedTargets++
			}
			if res.target == r.SrcID {
				continue
			}
			ki, ok := kindIdx[r.Kind]
			if !ok {
				ki = int64(len(kindIdx) + 1)
				kindIdx[r.Kind] = ki
			}
			k := [3]int64{r.SrcID, res.target, ki}
			if seen[k] {
				continue
			}
			seen[k] = true
			edges = append(edges, model.ResolvedEdge{
				Src: r.SrcID, Dst: res.target, Kind: r.Kind, Line: r.Line, Confidence: res.conf,
			})
		}

		if err := st.UpdateRefTarget(updates); err != nil {
			return 0, 0, err
		}
		if err := st.InsertEdges(edges); err != nil {
			return 0, 0, err
		}
		totalEdges += len(edges)
		if !quiet {
			fmt.Fprintf(os.Stderr, "[bind] %d refs, %d edges, %d target updates (%.1fs)\n",
				totalRefs, totalEdges, changedTargets, time.Since(t0).Seconds())
		}
	}

	if !quiet {
		fmt.Fprintf(os.Stderr, "[bind] done: %d refs, %d edges, %d unresolved, %d target updates (%.1fs)\n",
			totalRefs, totalEdges, unresolved, changedTargets, time.Since(t0).Seconds())
	}
	return totalEdges, unresolved, nil
}
