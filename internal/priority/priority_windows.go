//go:build windows

// Package priority lowers the indexing process priority so a background index
// run does not compete with the editor or the game for CPU.
package priority

import "syscall"

var (
	kernel32         = syscall.NewLazyDLL("kernel32.dll")
	procSetPriority  = kernel32.NewProc("SetPriorityClass")
	procGetCurrent   = kernel32.NewProc("GetCurrentProcess")
)

const belowNormalPriorityClass = 0x00004000

// SetLow switches this process to BELOW_NORMAL_PRIORITY_CLASS.
func SetLow() error {
	h, _, _ := procGetCurrent.Call()
	r, _, err := procSetPriority.Call(h, uintptr(belowNormalPriorityClass))
	if r == 0 {
		return err
	}
	return nil
}
