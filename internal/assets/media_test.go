package assets

import (
	"encoding/binary"
	"os"
	"path/filepath"
	"testing"
)

func writeFile(t *testing.T, dir, name string, data []byte) string {
	t.Helper()
	p := filepath.Join(dir, name)
	if err := os.WriteFile(p, data, 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestPNGDimensions(t *testing.T) {
	dir := t.TempDir()
	head := make([]byte, 24)
	copy(head, []byte{0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A})
	binary.BigEndian.PutUint32(head[16:20], 128)
	binary.BigEndian.PutUint32(head[20:24], 64)
	p := writeFile(t, dir, "icon.png", head)
	m := mediaMeta(p, "icon.png", int64(len(head)))
	if m["width"] != "128" || m["height"] != "64" {
		t.Fatalf("png dims = %v", m)
	}
	if m["dimensions"] != "128x64" {
		t.Errorf("dimensions = %q", m["dimensions"])
	}
}

func TestWAVDuration(t *testing.T) {
	dir := t.TempDir()
	// RIFF header + fmt chunk (mono, 44100Hz, 16bit) + data chunk of 88200 bytes (=1s)
	data := make([]byte, 44)
	copy(data[0:], "RIFF")
	binary.LittleEndian.PutUint32(data[4:8], 36)
	copy(data[8:], "WAVE")
	copy(data[12:], "fmt ")
	binary.LittleEndian.PutUint32(data[16:20], 16)
	binary.LittleEndian.PutUint16(data[20:22], 1)
	binary.LittleEndian.PutUint16(data[22:24], 1)
	binary.LittleEndian.PutUint32(data[24:28], 44100)
	binary.LittleEndian.PutUint32(data[28:32], 88200)
	binary.LittleEndian.PutUint16(data[32:34], 2)
	binary.LittleEndian.PutUint16(data[34:36], 16)
	copy(data[36:], "data")
	binary.LittleEndian.PutUint32(data[40:44], 88200)
	p := writeFile(t, dir, "hit.wav", data)
	m := mediaMeta(p, "hit.wav", int64(len(data)))
	if m["sample_rate"] != "44100" || m["channels"] != "1" {
		t.Fatalf("wav meta = %v", m)
	}
	if m["duration_sec"] != "1.000" {
		t.Errorf("duration = %q, want 1.000", m["duration_sec"])
	}
}

func TestMP4Duration(t *testing.T) {
	dir := t.TempDir()
	data := make([]byte, 64)
	data[3] = 8
	copy(data[4:], "ftyp")
	idx := 20
	copy(data[idx:], "mvhd")
	// version 0 at idx+4, timescale at idx+16, duration at idx+20
	binary.BigEndian.PutUint32(data[idx+16:idx+20], 1000)
	binary.BigEndian.PutUint32(data[idx+20:idx+24], 2500)
	p := writeFile(t, dir, "op.mp4", data)
	m := mediaMeta(p, "op.mp4", int64(len(data)))
	if m["duration_sec"] != "2.50" {
		t.Fatalf("mp4 duration = %v", m)
	}
}

func TestFontHeader(t *testing.T) {
	// Use a real system font when available; skip otherwise (CI friendliness).
	candidates := []string{
		`C:\Windows\Fonts\arial.ttf`,
		`C:\Windows\Fonts\segoeui.ttf`,
		"/System/Library/Fonts/Supplemental/Arial.ttf",
		"/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
	}
	var p string
	for _, c := range candidates {
		if _, err := os.Stat(c); err == nil {
			p = c
			break
		}
	}
	if p == "" {
		t.Skip("no system TTF available")
	}
	st, err := os.Stat(p)
	if err != nil {
		t.Skip(err)
	}
	m := mediaMeta(p, "font.ttf", st.Size())
	if m["glyphs"] == "" {
		t.Fatalf("glyphs not parsed: %v", m)
	}
	if m["font_name"] == "" {
		t.Errorf("font_name not parsed: %v", m)
	}
}

func TestAssetKind(t *testing.T) {
	cases := map[string]string{
		"a/b/c.png": "image", "a/hit.wav": "audio", "a/op.mp4": "video",
		"a/font.ttf": "font", "a/skel.atlas": "spine_atlas", "a/skel.skel": "spine_data",
		"a/UI.mat": "material", "a/run.anim": "animation_clip",
		"a/Hero.controller": "animator_controller", "a/mix.mixer": "audio_mixer",
		"a/cut.playable": "timeline", "a/Game.asmdef": "assembly_def",
	}
	for rel, want := range cases {
		if got := AssetKind(rel); got != want {
			t.Errorf("%s: kind=%q want %q", rel, got, want)
		}
	}
	if !AssetExt("a/b/c.png") || !AssetExt("x/UI.mat") || AssetExt("a/main.cs") {
		t.Error("AssetExt classification wrong")
	}
}
