//go:build !windows

package priority

// SetLow is a no-op outside Windows (use nice/renice externally).
func SetLow() error { return nil }
