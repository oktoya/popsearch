use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde::Serialize;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

#[derive(Serialize, Clone, Debug)]
pub struct ImageData {
    pub media_type: String,
    /// base64
    pub data: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Selection {
    pub text: Option<String>,
    pub image: Option<ImageData>,
    pub image_urls: Vec<String>,
}

/// Build a BMP file from a packed DIB (BITMAPINFOHEADER / V4 / V5 + pixels).
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([dib[o], dib[o + 1], dib[o + 2], dib[o + 3]]);
    let header_size = u32_at(0) as usize;
    if header_size < 40 || header_size > dib.len() {
        return None;
    }
    let bit_count = u16::from_le_bytes([dib[14], dib[15]]) as u32;
    let compression = u32_at(16);
    let clr_used = u32_at(32) as usize;
    let mut extra = 0usize;
    // Masks that follow a plain 40-byte header (V4/V5 headers contain them).
    if header_size == 40 {
        if compression == 3 {
            extra += 12;
        } else if compression == 6 {
            extra += 16;
        }
    }
    let table = if clr_used > 0 {
        clr_used * 4
    } else if bit_count <= 8 {
        (1usize << bit_count) * 4
    } else {
        0
    };
    let off_bits = 14 + header_size + extra + table;
    if off_bits > 14 + dib.len() {
        return None;
    }
    let total = 14 + dib.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(off_bits as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

fn to_base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | (*c.get(2).unwrap_or(&0) as u32);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Extracts the HTML fragment from CF_HTML clipboard data (StartFragment/EndFragment byte offsets).
#[cfg_attr(not(windows), allow(dead_code))]
fn cf_html_fragment(data: &[u8]) -> Option<String> {
    let head_len = data.len().min(1024);
    let head = String::from_utf8_lossy(&data[..head_len]);
    let num = |key: &str| -> Option<usize> {
        let i = head.find(key)? + key.len();
        let digits: String = head[i..].chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    let start = num("StartFragment:")?;
    let end = num("EndFragment:")?;
    if start > end || end > data.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&data[start..end]).into_owned())
}

/// `src` values of `<img>` tags in document order, de-duplicated, at most 5.
#[cfg_attr(not(windows), allow(dead_code))]
fn extract_img_srcs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase();
    let b = html.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<img") {
        let tag_start = pos + i + 4;
        pos = tag_start;
        match b.get(tag_start) {
            Some(c) if c.is_ascii_whitespace() || *c == b'/' || *c == b'>' => {}
            _ => continue,
        }
        let mut j = tag_start;
        while j < b.len() && b[j] != b'>' {
            while j < b.len() && (b[j].is_ascii_whitespace() || b[j] == b'/') {
                j += 1;
            }
            if j >= b.len() || b[j] == b'>' {
                break;
            }
            let name_start = j;
            while j < b.len()
                && !b[j].is_ascii_whitespace()
                && b[j] != b'='
                && b[j] != b'>'
                && b[j] != b'/'
            {
                j += 1;
            }
            let name = &html[name_start..j];
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut value: Option<&str> = None;
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    let vs = j + 1;
                    let mut k = vs;
                    while k < b.len() && b[k] != q {
                        k += 1;
                    }
                    value = Some(&html[vs..k]);
                    j = (k + 1).min(b.len());
                } else {
                    let vs = j;
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    value = Some(&html[vs..j]);
                }
            }
            if name.eq_ignore_ascii_case("src") {
                if let Some(v) = value {
                    let url = v.trim().replace("&amp;", "&");
                    if !url.is_empty() && !out.contains(&url) {
                        out.push(url);
                    }
                }
            }
        }
        pos = j.max(pos);
        if out.len() >= 5 {
            break;
        }
    }
    out.truncate(5);
    out
}

/// Media type from magic bytes. None for unsupported data (SVG, HTML, ...).
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// `data:image/(png|jpeg|gif|webp);base64,...` -> bytes.
fn decode_data_url(url: &str) -> Option<Vec<u8>> {
    if !url.get(..5)?.eq_ignore_ascii_case("data:") {
        return None;
    }
    let (meta, data) = url[5..].split_once(',')?;
    let meta = meta.to_ascii_lowercase();
    let mut parts = meta.split(';');
    let mt = parts.next()?;
    if !matches!(mt, "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
        return None;
    }
    if !parts.any(|p| p == "base64") {
        return None;
    }
    base64_decode(data)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `file:///C:/a%20b.png` -> `C:/a b.png` (Windows drive) or `/home/x.png`.
fn file_url_to_path(url: &str) -> Option<String> {
    if !url.get(..8)?.eq_ignore_ascii_case("file:///") {
        return None;
    }
    let decoded = percent_decode(&url[8..])?;
    let bytes = decoded.as_bytes();
    let path = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        decoded
    } else {
        format!("/{}", decoded)
    };
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp") {
        return None;
    }
    Some(path)
}

const MAX_FETCH_BYTES: u64 = 10 * 1024 * 1024;

/// Blocking. Fetches one image URL (data:, file:, http(s):). None on any failure or unsupported type.
pub fn fetch_image(url: &str) -> Option<ImageData> {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let bytes: Vec<u8> = if lower.starts_with("data:") {
        decode_data_url(url)?
    } else if lower.starts_with("file:") {
        let path = file_url_to_path(url)?;
        if std::fs::metadata(&path).ok()?.len() > MAX_FETCH_BYTES {
            return None;
        }
        std::fs::read(path).ok()?
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        use std::io::Read;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(4))
            .build();
        let resp = agent.get(url).call().ok()?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(MAX_FETCH_BYTES + 1)
            .read_to_end(&mut buf)
            .ok()?;
        if buf.len() as u64 > MAX_FETCH_BYTES {
            return None;
        }
        buf
    } else {
        return None;
    };
    let media_type = sniff_image(&bytes)?;
    Some(ImageData {
        media_type: media_type.into(),
        data: to_base64(&bytes),
    })
}

/// Window classes where the clipboard fallback (Ctrl+C) must not be used.
#[cfg_attr(not(windows), allow(dead_code))]
const EXCLUDED_CLASSES: &[&str] = &["ConsoleWindowClass", "CASCADIA_HOSTING_WINDOW_CLASS"];

/// Process names (lowercase) where the clipboard fallback must not be used
/// (terminals and code editors: Ctrl+C = interrupt / copy current line).
#[cfg_attr(not(windows), allow(dead_code))]
const EXCLUDED_EXES: &[&str] = &[
    "windowsterminal.exe",
    "openconsole.exe",
    "conhost.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "mintty.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "code.exe",
    "code - insiders.exe",
    "antigravity.exe",
    "antigravity ide.exe",
    "cursor.exe",
    "windsurf.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "rider64.exe",
    "clion64.exe",
    "goland64.exe",
    "rustrover64.exe",
    "datagrip64.exe",
    "phpstorm64.exe",
    "studio64.exe",
];

#[cfg_attr(not(windows), allow(dead_code))]
fn is_excluded(class: &str, exe_name: &str) -> bool {
    if EXCLUDED_CLASSES.iter().any(|c| c.eq_ignore_ascii_case(class)) {
        return true;
    }
    let exe = exe_name.trim().to_lowercase();
    !exe.is_empty() && EXCLUDED_EXES.contains(&exe.as_str())
}

/// True when the clipboard carries a "copied from an empty selection (whole line)" marker.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_line_copy(format_names: &[String], vscode_data: Option<&str>) -> bool {
    if format_names
        .iter()
        .any(|n| n == "MSDEVLineSelect" || n == "VisualStudioEditorOperationsLineCutCopyClipboardTag")
    {
        return true;
    }
    if let Some(data) = vscode_data {
        let compact: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        return compact.contains("\"isFromEmptySelection\":true");
    }
    false
}

/// Decodes marker text that may be UTF-16LE (BOM or NUL bytes) or UTF-8.
#[cfg_attr(not(windows), allow(dead_code))]
fn decode_marker_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes.len() % 2 == 0 && (bytes[0] == 0xFF && bytes[1] == 0xFE || bytes[1] == 0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let b: Vec<u8> = bytes.iter().copied().take_while(|&x| x != 0).collect();
    String::from_utf8_lossy(&b).into_owned()
}

/// Blocking. Reads the current selection of the foreground app.
pub fn capture_selection() -> Selection {
    #[cfg(windows)]
    {
        win::capture()
    }
    #[cfg(not(windows))]
    {
        capture_other()
    }
}

#[cfg(not(windows))]
fn capture_other() -> Selection {
    let text = (|| -> Option<String> {
        std::thread::sleep(Duration::from_millis(150));
        let mut cb = arboard::Clipboard::new().ok()?;
        let before = cb.get_text().ok();

        let mut enigo = Enigo::new(&Settings::default()).ok()?;
        enigo.key(MODIFIER, Direction::Press).ok()?;
        #[cfg(target_os = "macos")]
        let click = enigo.raw(8, Direction::Click); // kVK_ANSI_C: physical key, independent of input source
        #[cfg(not(target_os = "macos"))]
        let click = enigo.key(Key::Unicode('c'), Direction::Click);
        let release = enigo.key(MODIFIER, Direction::Release);
        if click.is_err() || release.is_err() {
            return None;
        }

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            std::thread::sleep(Duration::from_millis(20));
            if let Ok(after) = cb.get_text() {
                if Some(&after) != before.as_ref() && !after.trim().is_empty() {
                    return Some(after);
                }
            }
        }
        None
    })();
    Selection { text, image: None, image_urls: Vec::new() }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
        GetClipboardFormatNameW, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
        OpenClipboard,
        RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };

    const CF_UNICODETEXT: u32 = 13;
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;

    pub fn capture() -> Selection {
        // Step 1: UI Automation, 500 ms timeout (no fallback on timeout).
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(uia_selection());
        });
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Some(text)) => {
                // Copy only to read the HTML (images); UIA text wins.
                let urls = clipboard_fallback()
                    .map(|s| s.image_urls)
                    .unwrap_or_default();
                return Selection {
                    text: Some(text),
                    image: None,
                    image_urls: urls,
                };
            }
            Ok(None) => {}
            Err(_) => return Selection::default(),
        }
        // Step 2: clipboard fallback with restore.
        clipboard_fallback().unwrap_or_default()
    }

    fn uia_selection() -> Option<String> {
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        };
        use windows::Win32::UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
        };
        use windows::core::Interface;

        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            let inited = hr.is_ok();
            let result = (|| -> Option<String> {
                let uia: IUIAutomation =
                    CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
                let el = uia.GetFocusedElement().ok()?;
                let pattern = el.GetCurrentPattern(UIA_TextPatternId).ok()?;
                let tp: IUIAutomationTextPattern = pattern.cast().ok()?;
                let ranges = tp.GetSelection().ok()?;
                let n = ranges.Length().ok()?;
                let mut parts: Vec<String> = Vec::new();
                for i in 0..n {
                    let r = ranges.GetElement(i).ok()?;
                    let t = r.GetText(-1).ok()?;
                    parts.push(t.to_string());
                }
                let joined = parts.join("\n");
                if joined.trim().is_empty() {
                    None
                } else {
                    Some(joined)
                }
            })();
            if inited {
                CoUninitialize();
            }
            result
        }
    }

    fn open_clipboard() -> bool {
        for _ in 0..10 {
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Copies the bytes behind an HGLOBAL clipboard handle. None if it cannot be locked.
    unsafe fn read_hglobal(handle: *mut c_void) -> Option<Vec<u8>> {
        unsafe {
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle);
            if size == 0 {
                return None;
            }
            let ptr = GlobalLock(handle);
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        }
    }

    fn is_skipped_format(f: u32) -> bool {
        // BITMAP, METAFILEPICT, PALETTE, ENHMETAFILE, OWNERDISPLAY, DSPBITMAP,
        // DSPMETAFILEPICT, DSPENHMETAFILE, and the GDI object range.
        matches!(f, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E) || (0x300..=0x3FF).contains(&f)
    }

    /// Clipboard must be open.
    /// None if enumeration failed (not just ended).
    unsafe fn snapshot() -> Option<Vec<(u32, Vec<u8>)>> {
        unsafe {
            use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
            let mut out = Vec::new();
            SetLastError(0);
            let mut f = EnumClipboardFormats(0);
            while f != 0 {
                if !is_skipped_format(f) {
                    let h = GetClipboardData(f);
                    if let Some(bytes) = read_hglobal(h) {
                        out.push((f, bytes));
                    }
                }
                f = EnumClipboardFormats(f);
            }
            if GetLastError() != 0 {
                return None;
            }
            Some(out)
        }
    }

    /// Foreground window class and process exe file name (lowercase may be applied by caller).
    fn foreground_info() -> (String, String) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
        };
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return (String::new(), String::new());
            }
            let mut buf = [0u16; 256];
            let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            let class = if n > 0 {
                String::from_utf16_lossy(&buf[..n as usize])
            } else {
                String::new()
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut exe = String::new();
            if pid != 0 {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !h.is_null() {
                    let mut pbuf = [0u16; 1024];
                    let mut len = pbuf.len() as u32;
                    if QueryFullProcessImageNameW(h, 0, pbuf.as_mut_ptr(), &mut len) != 0 {
                        let full = String::from_utf16_lossy(&pbuf[..len as usize]);
                        exe = full.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                    }
                    CloseHandle(h);
                }
            }
            (class, exe)
        }
    }

    fn restore(snap: &[(u32, Vec<u8>)]) {
        if !open_clipboard() {
            return;
        }
        unsafe {
            EmptyClipboard();
            for (fmt, bytes) in snap {
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
                if h.is_null() {
                    continue;
                }
                let p = GlobalLock(h);
                if p.is_null() {
                    GlobalFree(h);
                    continue;
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
                GlobalUnlock(h);
                if SetClipboardData(*fmt, h).is_null() {
                    // ownership was not taken; free it
                    GlobalFree(h);
                }
            }
            CloseClipboard();
        }
    }

    /// Reads the new clipboard content (text first, image only when there is no text).
    fn read_new() -> Selection {
        if !open_clipboard() {
            return Selection::default();
        }
        let mut sel = Selection::default();
        unsafe {
            // Editors copy the whole current line when nothing is selected; they leave a marker format.
            let mut names: Vec<String> = Vec::new();
            let mut vscode_data: Option<String> = None;
            let mut f = EnumClipboardFormats(0);
            while f != 0 {
                if f >= 0xC000 {
                    let mut buf = [0u16; 128];
                    let n = GetClipboardFormatNameW(f, buf.as_mut_ptr(), buf.len() as i32);
                    if n > 0 {
                        let name = String::from_utf16_lossy(&buf[..n as usize]);
                        if name == "vscode-editor-data" {
                            if let Some(bytes) = read_hglobal(GetClipboardData(f)) {
                                vscode_data = Some(decode_marker_text(&bytes));
                            }
                        }
                        names.push(name);
                    }
                }
                f = EnumClipboardFormats(f);
            }
            if is_line_copy(&names, vscode_data.as_deref()) {
                CloseClipboard();
                return Selection::default();
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
                if let Some(bytes) = read_hglobal(GetClipboardData(CF_UNICODETEXT)) {
                    let units: Vec<u16> = bytes
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&u| u != 0)
                        .collect();
                    let text = String::from_utf16_lossy(&units);
                    if !text.trim().is_empty() {
                        sel.text = Some(text);
                    }
                }
            }
            let html_name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
            let html_fmt = RegisterClipboardFormatW(html_name.as_ptr());
            if html_fmt != 0 && IsClipboardFormatAvailable(html_fmt) != 0 {
                if let Some(bytes) = read_hglobal(GetClipboardData(html_fmt)) {
                    let bytes: Vec<u8> = bytes.into_iter().take_while(|&b| b != 0).collect();
                    if let Some(frag) = cf_html_fragment(&bytes) {
                        sel.image_urls = extract_img_srcs(&frag);
                    }
                }
            }
            if sel.text.is_none() {
                let name: Vec<u16> = "PNG\0".encode_utf16().collect();
                let png_fmt = RegisterClipboardFormatW(name.as_ptr());
                if png_fmt != 0 && IsClipboardFormatAvailable(png_fmt) != 0 {
                    if let Some(bytes) = read_hglobal(GetClipboardData(png_fmt)) {
                        sel.image = Some(ImageData {
                            media_type: "image/png".into(),
                            data: to_base64(&bytes),
                        });
                    }
                }
                if sel.image.is_none() {
                    for fmt in [CF_DIBV5, CF_DIB] {
                        if IsClipboardFormatAvailable(fmt) != 0 {
                            if let Some(dib) = read_hglobal(GetClipboardData(fmt)) {
                                if let Some(bmp) = dib_to_bmp(&dib) {
                                    sel.image = Some(ImageData {
                                        media_type: "image/bmp".into(),
                                        data: to_base64(&bmp),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            CloseClipboard();
        }
        sel
    }

    fn clipboard_fallback() -> Option<Selection> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        };
        let any_down = || {
            [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
                .iter()
                .any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } < 0)
        };
        let wait_start = Instant::now();
        while any_down() {
            if wait_start.elapsed() >= Duration::from_millis(600) {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let (class, exe) = foreground_info();
        if is_excluded(&class, &exe) {
            return None;
        }

        let seq_before = unsafe { GetClipboardSequenceNumber() };
        if !open_clipboard() {
            return None;
        }
        let snap = unsafe { snapshot() };
        unsafe { CloseClipboard() };
        // Never send a key (or restore) without a successful snapshot.
        let snap = snap?;

        let mut enigo = Enigo::new(&Settings::default()).ok()?;
        enigo.key(MODIFIER, Direction::Press).ok()?;
        let click = enigo.key(Key::Other(0x43), Direction::Click);
        let release = enigo.key(MODIFIER, Direction::Release);
        if click.is_err() || release.is_err() {
            return None;
        }

        let start = Instant::now();
        let mut changed = false;
        while start.elapsed() < Duration::from_millis(300) {
            std::thread::sleep(Duration::from_millis(20));
            if unsafe { GetClipboardSequenceNumber() } != seq_before {
                changed = true;
                break;
            }
        }
        if !changed {
            return None;
        }
        // give the source app a moment to finish writing all formats
        std::thread::sleep(Duration::from_millis(30));

        let sel = read_new();
        if !snap.is_empty() {
            restore(&snap);
        } else if open_clipboard() {
            unsafe {
                EmptyClipboard();
                CloseClipboard();
            }
        }
        Some(sel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_to_bmp_plain_24bit() {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[1, 2, 3, 4]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()), 14 + 44);
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        assert_eq!(&bmp[14..], &dib[..]);
    }

    #[test]
    fn dib_to_bmp_color_table_and_bitfields() {
        // 8-bit, clr_used 0 -> 256 entries
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&8u16.to_le_bytes());
        dib.extend_from_slice(&vec![0u8; 1024 + 2]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 14 + 40 + 1024);

        // 32-bit BI_BITFIELDS -> 12 mask bytes
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        dib.extend_from_slice(&vec![0u8; 12 + 4]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 14 + 40 + 12);
        assert_eq!(u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize, bmp.len());

        // V5 header (124), 32-bit: masks already inside header
        let mut dib = vec![0u8; 124];
        dib[0..4].copy_from_slice(&124u32.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 4]);
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 14 + 124);
    }

    #[test]
    fn dib_to_bmp_rejects_garbage() {
        assert!(dib_to_bmp(&[0u8; 10]).is_none());
    }

    #[test]
    fn is_excluded_cases() {
        assert!(is_excluded("CASCADIA_HOSTING_WINDOW_CLASS", ""));
        assert!(is_excluded("consolewindowclass", "whatever.exe"));
        assert!(is_excluded("Chrome_WidgetWin_1", "Code.EXE"));
        assert!(is_excluded("", "WindowsTerminal.exe"));
        assert!(is_excluded("x", "Code - Insiders.exe"));
        assert!(is_excluded("x", "Antigravity IDE.exe"));
        assert!(!is_excluded("Notepad", "notepad.exe"));
        assert!(!is_excluded("", ""));
    }

    #[test]
    fn cf_html_fragment_extracts() {
        let html = "<html><body><!--StartFragment--><b>hi</b><!--EndFragment--></body></html>";
        let start = html.find("<b>").unwrap();
        let end = html.find("<!--EndFragment-->").unwrap();
        let header_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
        let data = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n{}",
            header_len,
            header_len + html.len(),
            header_len + start,
            header_len + end,
            html
        );
        assert_eq!(cf_html_fragment(data.as_bytes()).unwrap(), "<b>hi</b>");
        assert!(cf_html_fragment(b"garbage").is_none());
        assert!(cf_html_fragment(b"StartFragment:0000000900\r\nEndFragment:0000000999\r\n").is_none());
    }

    #[test]
    fn img_srcs_extraction() {
        let h = r#"<p>a <IMG class=x SRC="http://a/x.png?a=1&amp;b=2"> <img data-src="no" src='b.png'/> <img src=c.png> <img src="http://a/x.png?a=1&amp;b=2"> <imgx src="no"> <img alt="s>" src="d.png">"#;
        assert_eq!(
            extract_img_srcs(h),
            vec!["http://a/x.png?a=1&b=2", "b.png", "c.png", "d.png"]
        );
        let many: String = (0..8).map(|i| format!("<img src=\"{i}.png\">")).collect();
        assert_eq!(extract_img_srcs(&many).len(), 5);
        assert!(extract_img_srcs("<p>none</p>").is_empty());
    }

    #[test]
    fn sniff_image_magic() {
        assert_eq!(sniff_image(&[0x89, b'P', b'N', b'G', 1]), Some("image/png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"BM\0\0"), Some("image/bmp"));
        assert_eq!(sniff_image(b"<svg xmlns"), None);
        assert_eq!(sniff_image(b"<!DOCTYPE html>"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn data_url_decoding() {
        assert_eq!(
            decode_data_url("data:image/png;base64,TWFu").unwrap(),
            b"Man".to_vec()
        );
        assert_eq!(decode_data_url("DATA:image/jpeg;base64,TWE=").unwrap(), b"Ma".to_vec());
        assert!(decode_data_url("data:image/svg+xml;base64,TWFu").is_none());
        assert!(decode_data_url("data:image/png,abc").is_none());
        assert!(decode_data_url("data:image/png;base64,@@@").is_none());
    }

    #[test]
    fn file_url_paths() {
        assert_eq!(
            file_url_to_path("file:///C:/Users/a%20b/x.PNG").unwrap(),
            "C:/Users/a b/x.PNG"
        );
        assert_eq!(file_url_to_path("file:///home/u/p.jpg").unwrap(), "/home/u/p.jpg");
        assert!(file_url_to_path("file:///C:/a/b.txt").is_none());
        assert!(file_url_to_path("file:///C:/a%zz.png").is_none());
        assert!(file_url_to_path("http://x/a.png").is_none());
    }

    #[test]
    fn base64_basic() {
        assert_eq!(to_base64(b"Man"), "TWFu");
        assert_eq!(to_base64(b"Ma"), "TWE=");
        assert_eq!(to_base64(b"M"), "TQ==");
    }

    #[test]
    fn line_copy_markers() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(is_line_copy(&s(&["MSDEVLineSelect"]), None));
        assert!(is_line_copy(
            &s(&["VisualStudioEditorOperationsLineCutCopyClipboardTag"]),
            None
        ));
        assert!(is_line_copy(
            &s(&["vscode-editor-data"]),
            Some("{\"version\":1,\"isFromEmptySelection\":true}")
        ));
        assert!(is_line_copy(
            &s(&["vscode-editor-data"]),
            Some("{\"isFromEmptySelection\": true}")
        ));
        assert!(!is_line_copy(
            &s(&["vscode-editor-data"]),
            Some("{\"isFromEmptySelection\":false}")
        ));
        assert!(!is_line_copy(&s(&["HTML Format", "PNG"]), None));
    }
}
