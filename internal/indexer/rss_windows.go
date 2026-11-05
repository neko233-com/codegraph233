//go:build windows

package indexer

import (
	"syscall"
	"unsafe"
)

type processMemoryCounters struct {
	cb                         uint32
	PageFaultCount             uint32
	PeakWorkingSetSize         uintptr
	WorkingSetSize             uintptr
	QuotaPeakPagedPoolUsage    uintptr
	QuotaPagedPoolUsage        uintptr
	QuotaPeakNonPagedPoolUsage uintptr
	QuotaNonPagedPoolUsage     uintptr
	PagefileUsage              uintptr
	PeakPagefileUsage          uintptr
}

var (
	psapi                   = syscall.NewLazyDLL("psapi.dll")
	procGetProcessMemoryInfo = psapi.NewProc("GetProcessMemoryInfo")
	kernel32proc            = syscall.NewLazyDLL("kernel32.dll")
	procGetCurrentProcess   = kernel32proc.NewProc("GetCurrentProcess")
)

// currentRSSKB returns the process working set in KB.
func currentRSSKB() int64 {
	h, _, _ := procGetCurrentProcess.Call()
	pmc := processMemoryCounters{}
	pmc.cb = uint32(unsafe.Sizeof(pmc))
	r, _, _ := procGetProcessMemoryInfo.Call(h, uintptr(unsafe.Pointer(&pmc)), uintptr(pmc.cb))
	if r == 0 {
		return 0
	}
	return int64(pmc.WorkingSetSize / 1024)
}
