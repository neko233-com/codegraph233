package assets

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"path"
	"strings"
)

// mediaMeta reads lightweight metadata from binary asset headers.  It never
// loads a whole asset: images/audio/video/fonts are inspected through a few
// hundred bytes (plus a small tail probe where the format requires it).
func mediaMeta(absPath, relPath string, size int64) map[string]string {
	f, err := os.Open(absPath)
	if err != nil {
		return map[string]string{"bytes": fmt.Sprint(size)}
	}
	defer f.Close()
	return mediaMetaFromFile(f, relPath, size)
}

func mediaMetaFromFile(f *os.File, relPath string, size int64) map[string]string {
	ext := strings.ToLower(path.Ext(relPath))
	meta := map[string]string{}
	head := make([]byte, 8192)
	n, _ := io.ReadFull(f, head)
	head = head[:n]

	switch ext {
	case ".png":
		if len(head) >= 24 && bytes.HasPrefix(head, []byte{0x89, 'P', 'N', 'G'}) {
			meta["width"] = fmt.Sprint(binary.BigEndian.Uint32(head[16:20]))
			meta["height"] = fmt.Sprint(binary.BigEndian.Uint32(head[20:24]))
		}
	case ".jpg", ".jpeg":
		if w, h, ok := jpegSize(head); ok {
			meta["width"], meta["height"] = fmt.Sprint(w), fmt.Sprint(h)
		}
	case ".gif":
		if len(head) >= 10 && string(head[:3]) == "GIF" {
			meta["width"] = fmt.Sprint(binary.LittleEndian.Uint16(head[6:8]))
			meta["height"] = fmt.Sprint(binary.LittleEndian.Uint16(head[8:10]))
		}
	case ".bmp":
		if len(head) >= 26 && string(head[:2]) == "BM" {
			meta["width"] = fmt.Sprint(binary.LittleEndian.Uint32(head[18:22]))
			meta["height"] = fmt.Sprint(binary.LittleEndian.Uint32(head[22:26]))
		}
	case ".psd":
		if len(head) >= 22 && string(head[:4]) == "8BPS" {
			meta["width"] = fmt.Sprint(binary.BigEndian.Uint32(head[18:22]))
			meta["height"] = fmt.Sprint(binary.BigEndian.Uint32(head[14:18]))
		}
	case ".webp":
		if len(head) >= 30 && string(head[:4]) == "RIFF" && string(head[8:12]) == "WEBP" {
			if string(head[12:16]) == "VP8X" {
				w := 1 + (uint32(head[24]) | uint32(head[25])<<8 | uint32(head[26])<<16)
				h := 1 + (uint32(head[27]) | uint32(head[28])<<8 | uint32(head[29])<<16)
				meta["width"], meta["height"] = fmt.Sprint(w), fmt.Sprint(h)
			}
		}
	case ".tga":
		if len(head) >= 16 {
			meta["width"] = fmt.Sprint(binary.LittleEndian.Uint16(head[12:14]))
			meta["height"] = fmt.Sprint(binary.LittleEndian.Uint16(head[14:16]))
		}
	case ".wav":
		metaOfWav(head, size, meta)
	case ".mp3":
		metaOfMP3(head, size, meta)
	case ".ogg":
		metaOfOgg(f, head, size, meta)
	case ".flac":
		metaOfFlac(head, meta)
	case ".m4a", ".aac", ".mp4", ".mov", ".m4v":
		metaOfMP4File(f, head, size, meta)
	case ".ttf", ".otf", ".ttc":
		metaOfFontFile(f, head, size, meta)
	}
	if _, ok := meta["width"]; ok {
		meta["dimensions"] = meta["width"] + "x" + meta["height"]
	}
	meta["bytes"] = fmt.Sprint(size)
	return meta
}

func jpegSize(b []byte) (int, int, bool) {
	if len(b) < 4 || b[0] != 0xFF || b[1] != 0xD8 {
		return 0, 0, false
	}
	i := 2
	for i+9 < len(b) {
		if b[i] != 0xFF {
			i++
			continue
		}
		m := b[i+1]
		if m == 0xD8 || m == 0xD9 || (m >= 0xD0 && m <= 0xD7) {
			i += 2
			continue
		}
		if i+3 >= len(b) {
			return 0, 0, false
		}
		segLen := int(binary.BigEndian.Uint16(b[i+2 : i+4]))
		if m >= 0xC0 && m <= 0xCF && m != 0xC4 && m != 0xC8 && m != 0xCC {
			if i+8 >= len(b) {
				return 0, 0, false
			}
			h := int(binary.BigEndian.Uint16(b[i+5 : i+7]))
			w := int(binary.BigEndian.Uint16(b[i+7 : i+9]))
			return w, h, w > 0
		}
		i += 2 + segLen
	}
	return 0, 0, false
}

func metaOfWav(b []byte, size int64, meta map[string]string) {
	if len(b) < 44 || string(b[:4]) != "RIFF" || string(b[8:12]) != "WAVE" {
		return
	}
	i := 12
	var byteRate uint32
	for i+8 <= len(b) {
		id := string(b[i : i+4])
		sz := int(binary.LittleEndian.Uint32(b[i+4 : i+8]))
		if id == "fmt " && i+8+16 <= len(b) {
			ch := binary.LittleEndian.Uint16(b[i+10 : i+12])
			sr := binary.LittleEndian.Uint32(b[i+12 : i+16])
			byteRate = binary.LittleEndian.Uint32(b[i+16 : i+20])
			meta["channels"] = fmt.Sprint(ch)
			meta["sample_rate"] = fmt.Sprint(sr)
		}
		if id == "data" && byteRate > 0 {
			seconds := float64(sz) / float64(byteRate)
			meta["duration_sec"] = fmt.Sprintf("%.3f", seconds)
			break
		}
		i += 8 + sz
		if sz < 0 {
			break
		}
	}
}

var mp3Bitrates = map[int]int{ // kbps for MPEG1 layer III
	1: 32, 2: 40, 3: 48, 4: 56, 5: 64, 6: 80, 7: 96, 8: 112, 9: 128,
	10: 160, 11: 192, 12: 224, 13: 256, 14: 320,
}

func metaOfMP3(b []byte, size int64, meta map[string]string) {
	i := 0
	if len(b) > 10 && string(b[:3]) == "ID3" {
		tagSize := int(b[6]&0x7F)<<21 | int(b[7]&0x7F)<<14 | int(b[8]&0x7F)<<7 | int(b[9]&0x7F)
		i = 10 + tagSize
	}
	for i+4 < len(b) {
		if b[i] != 0xFF || b[i+1]&0xE0 != 0xE0 {
			i++
			continue
		}
		h := b[i+1]
		brIdx := int(h>>4) & 0x0F
		srIdx := int(h>>2) & 0x03
		if brIdx == 0 || brIdx == 15 || srIdx > 2 {
			i++
			continue
		}
		br := mp3Bitrates[brIdx]
		if br == 0 {
			i++
			continue
		}
		sr := []int{44100, 48000, 32000}[srIdx]
		meta["sample_rate"] = fmt.Sprint(sr)
		meta["bitrate_kbps"] = fmt.Sprint(br)
		if br > 0 {
			meta["duration_sec"] = fmt.Sprintf("%.2f", float64(size)*8/float64(br*1000))
		}
		return
	}
}

func metaOfOgg(f *os.File, b []byte, size int64, meta map[string]string) {
	if len(b) < 28 || string(b[:4]) != "OggS" {
		return
	}
	// vorbis identification header is usually the first page's packet
	if idx := bytes.Index(b[:min(len(b), 512)], []byte{1, 'v', 'o', 'r', 'b', 'i', 's'}); idx >= 16 {
		ch := b[idx+11]
		sr := binary.LittleEndian.Uint32(b[idx+12 : idx+16])
		meta["channels"] = fmt.Sprint(ch)
		meta["sample_rate"] = fmt.Sprint(sr)
	}
	// duration: last page granule position / sample rate
	if sr := meta["sample_rate"]; sr != "" {
		{
			const tail = 8192
			off := size - tail
			if off < 0 {
				off = 0
			}
			buf := make([]byte, tail)
			n, _ := f.ReadAt(buf, off)
			buf = buf[:n]
			if p := bytes.LastIndex(buf, []byte("OggS")); p >= 0 && p+14 <= len(buf) {
				gran := binary.LittleEndian.Uint64(buf[p+6 : p+14])
				var rate uint32
				fmt.Sscan(sr, &rate)
				if rate > 0 {
					meta["duration_sec"] = fmt.Sprintf("%.2f", float64(gran)/float64(rate))
				}
			}
		}
	}
}

func metaOfFlac(b []byte, meta map[string]string) {
	if len(b) < 42 || string(b[:4]) != "fLaC" {
		return
	}
	// STREAMINFO starts at 8: 16-bit min/max block, 24-bit min/max frame,
	// then sampleRate(20) channels(3) bps(5) totalSamples(36)
	if len(b) >= 34 {
		p := b[18:26]
		sr := uint32(p[0])<<12 | uint32(p[1])<<4 | uint32(p[2])>>4
		ch := ((p[2] >> 1) & 0x07) + 1
		total := uint64(p[3]&0x0F)<<32 | uint64(p[4])<<24 | uint64(p[5])<<16 | uint64(p[6])<<8 | uint64(p[7])
		meta["channels"] = fmt.Sprint(ch)
		meta["sample_rate"] = fmt.Sprint(sr)
		if sr > 0 && total > 0 {
			meta["duration_sec"] = fmt.Sprintf("%.2f", float64(total)/float64(sr))
		}
	}
}

// metaOfMP4File reads moov/mvhd for duration and tkhd for dimensions, probing
// the file tail when the moov box is not at the front (non-faststart files).
func metaOfMP4File(f *os.File, head []byte, size int64, meta map[string]string) {
	b := head
	if !bytes.Contains(b, []byte("mvhd")) && size > int64(len(b)) {
		const tail = 256 << 10
		off := size - tail
		if off < 0 {
			off = 0
		}
		buf := make([]byte, tail)
		n, _ := f.ReadAt(buf, off)
		b = buf[:n]
	}
	metaOfMP4(b, size, meta)
}

func metaOfMP4(b []byte, size int64, meta map[string]string) {
	if len(b) < 12 || string(b[4:8]) != "ftyp" {
		return
	}
	// mvhd: find "mvhd" then version at +4
	if idx := bytes.Index(b, []byte("mvhd")); idx >= 0 && idx+32 < len(b) {
		ver := b[idx+4]
		if ver == 0 {
			timescale := binary.BigEndian.Uint32(b[idx+16 : idx+20])
			duration := binary.BigEndian.Uint32(b[idx+20 : idx+24])
			if timescale > 0 {
				meta["duration_sec"] = fmt.Sprintf("%.2f", float64(duration)/float64(timescale))
			}
		} else if idx+44 < len(b) {
			timescale := binary.BigEndian.Uint32(b[idx+28 : idx+32])
			duration := binary.BigEndian.Uint64(b[idx+32 : idx+40])
			if timescale > 0 {
				meta["duration_sec"] = fmt.Sprintf("%.2f", float64(duration)/float64(timescale))
			}
		}
	}
	if idx := bytes.Index(b, []byte("tkhd")); idx >= 0 && idx+92 < len(b) {
		ver := b[idx+4]
		base := idx + 4 + 4
		if ver == 1 {
			base += 16
		} else {
			base += 8
		}
		if base+8 <= len(b) {
			w := binary.BigEndian.Uint32(b[base+56 : base+60]) >> 16
			h := binary.BigEndian.Uint32(b[base+60 : base+64]) >> 16
			if w > 0 && h > 0 {
				meta["width"], meta["height"] = fmt.Sprint(w), fmt.Sprint(h)
			}
		}
	}
}

// metaOfFontFile reads the sfnt name/maxp tables via the file handle so tables
// beyond the head window are still parsed.
func metaOfFontFile(f *os.File, b []byte, size int64, meta map[string]string) {
	if len(b) < 12 {
		return
	}
	numTables := int(binary.BigEndian.Uint16(b[4:6]))
	if len(b) < 12+numTables*16 {
		return
	}
	var nameOff, nameLen, maxpOff int
	for i := 0; i < numTables; i++ {
		rec := 12 + i*16
		tag := string(b[rec : rec+4])
		off := int(binary.BigEndian.Uint32(b[rec+8 : rec+12]))
		length := int(binary.BigEndian.Uint32(b[rec+12 : rec+16]))
		switch tag {
		case "name":
			nameOff, nameLen = off, length
		case "maxp":
			maxpOff = off
		}
	}
	if maxpOff > 0 && int64(maxpOff)+6 <= size {
		buf := make([]byte, 6)
		if n, _ := f.ReadAt(buf, int64(maxpOff)); n == 6 {
			meta["glyphs"] = fmt.Sprint(binary.BigEndian.Uint16(buf[4:6]))
		}
	}
	if nameOff > 0 && nameLen > 0 && int64(nameOff)+int64(nameLen) <= size {
		if nameLen > 1<<20 {
			nameLen = 1 << 20
		}
		buf := make([]byte, nameLen)
		n, _ := f.ReadAt(buf, int64(nameOff))
		if s := findFontName(buf[:n]); s != "" {
			meta["font_name"] = s
		}
	}
}

// metaOfFont is kept for tests operating on in-memory buffers.
func metaOfFont(b []byte, meta map[string]string) {
	if len(b) < 12 {
		return
	}
	sig := binary.BigEndian.Uint32(b[:4])
	if sig != 0x00010000 && sig != 0x4F54544F && sig != 0x74727565 && sig != 0x74746366 {
		return
	}
	numTables := int(binary.BigEndian.Uint16(b[4:6]))
	if len(b) < 12+numTables*16 {
		return
	}
	var nameOff, nameLen, maxpOff int
	for i := 0; i < numTables; i++ {
		rec := 12 + i*16
		tag := string(b[rec : rec+4])
		off := int(binary.BigEndian.Uint32(b[rec+8 : rec+12]))
		length := int(binary.BigEndian.Uint32(b[rec+12 : rec+16]))
		switch tag {
		case "name":
			nameOff, nameLen = off, length
		case "maxp":
			maxpOff = off
		}
	}
	if maxpOff > 0 && maxpOff+6 <= len(b) {
		meta["glyphs"] = fmt.Sprint(binary.BigEndian.Uint16(b[maxpOff+4 : maxpOff+6]))
	}
	if nameOff > 0 && nameLen > 0 && nameOff+6 <= len(b) {
		// best effort: scan the name table for a UTF-16BE "full font name"
		end := nameOff + nameLen
		if end > len(b) {
			end = len(b)
		}
		if s := findFontName(b[nameOff:end]); s != "" {
			meta["font_name"] = s
		}
	}
}

func findFontName(tbl []byte) string {
	if len(tbl) < 6 {
		return ""
	}
	count := int(binary.BigEndian.Uint16(tbl[2:4]))
	stringsOff := int(binary.BigEndian.Uint16(tbl[4:6]))
	for i := 0; i < count; i++ {
		rec := 6 + i*12
		if rec+12 > len(tbl) {
			break
		}
		platform := binary.BigEndian.Uint16(tbl[rec : rec+2])
		nameID := binary.BigEndian.Uint16(tbl[rec+6 : rec+8])
		length := int(binary.BigEndian.Uint16(tbl[rec+8 : rec+10]))
		off := int(binary.BigEndian.Uint16(tbl[rec+10 : rec+12]))
		if nameID != 4 && nameID != 1 {
			continue
		}
		start := stringsOff + off
		if start < 0 || start+length > len(tbl) || length == 0 {
			continue
		}
		raw := tbl[start : start+length]
		if platform == 0 || platform == 3 {
			var sb strings.Builder
			for k := 0; k+1 < len(raw); k += 2 {
				r := rune(binary.BigEndian.Uint16(raw[k : k+2]))
				if r == 0 {
					continue
				}
				if r < 32 || r > 0x2FFF {
					r = '?'
				}
				sb.WriteRune(r)
			}
			if s := strings.TrimSpace(sb.String()); s != "" {
				return s
			}
		} else {
			s := strings.TrimSpace(string(raw))
			if s != "" {
				return s
			}
		}
	}
	return ""
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
