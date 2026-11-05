//go:build !windows

package indexer

import (
	"os"
	"strconv"
	"strings"
)

// currentRSSKB reads /proc/self/statm (pages) and converts to KiB.
func currentRSSKB() int64 {
	data, err := os.ReadFile("/proc/self/statm")
	if err != nil {
		return 0
	}
	fields := strings.Fields(string(data))
	if len(fields) < 2 {
		return 0
	}
	pages, err := strconv.ParseInt(fields[1], 10, 64)
	if err != nil {
		return 0
	}
	return pages * int64(os.Getpagesize()) / 1024
}
