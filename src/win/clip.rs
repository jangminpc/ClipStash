//! 클립보드 읽기/쓰기.

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT};
use windows_sys::Win32::UI::Shell::{DragQueryFileW, DROPFILES, HDROP};

use super::util::wide;

/// 직접 등록하는 클립보드 형식들
pub struct Formats {
    /// ClipStash 가 직접 넣은 데이터라는 표시(자기 자신을 다시 기록하지 않기 위해)
    pub own: u32,
    pub html: u32,
    pub drop_effect: u32,
    /// 비밀번호 관리자 등이 "기록하지 말라"고 붙이는 형식들
    pub exclude_monitor: u32,
    pub viewer_ignore: u32,
    pub can_include_history: u32,
}

impl Formats {
    pub fn register() -> Formats {
        let reg = |n: &str| unsafe { RegisterClipboardFormatW(wide(n).as_ptr()) };
        Formats {
            own: reg("ClipStash.Owned"),
            html: reg("HTML Format"),
            drop_effect: reg("Preferred DropEffect"),
            exclude_monitor: reg("ExcludeClipboardContentFromMonitorProcessing"),
            viewer_ignore: reg("Clipboard Viewer Ignore"),
            can_include_history: reg("CanIncludeInClipboardHistory"),
        }
    }
}

/// 클립보드를 여는 동안만 살아 있는 잠금. 다른 프로그램이 쥐고 있으면 잠깐 기다렸다 다시 시도합니다.
pub struct Opened;

impl Opened {
    pub fn open(owner: HWND) -> Option<Opened> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Some(Opened);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        None
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn has(fmt: u32) -> bool {
    fmt != 0 && unsafe { IsClipboardFormatAvailable(fmt) } != 0
}

/// 전역 메모리 블록의 내용을 복사해 옵니다.
unsafe fn global_bytes(h: HANDLE) -> Option<Vec<u8>> {
    if h.is_null() {
        return None;
    }
    let size = GlobalSize(h);
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, size).to_vec();
    GlobalUnlock(h);
    Some(v)
}

unsafe fn get_bytes(fmt: u32) -> Option<Vec<u8>> {
    global_bytes(GetClipboardData(fmt))
}

fn utf16_le_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let len = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..len])
}

pub enum Captured {
    Text { text: String, html: Option<String> },
    Image { dib: Vec<u8> },
    Files { paths: Vec<String> },
}

pub struct ReadOptions {
    pub text: bool,
    pub images: bool,
    pub files: bool,
    pub html: bool,
    pub max_text_bytes: usize,
    pub max_image_bytes: usize,
}

pub enum ReadResult {
    Got(Captured),
    /// 기록하면 안 되는 내용(자기 자신, 비밀번호 관리자, 너무 큰 데이터 등)
    Skip,
    /// 기록할 만한 형식이 없음
    Nothing,
    /// 클립보드를 열지 못함
    Busy,
}

/// 민감한 내용 표시가 붙어 있는지(클립보드를 연 상태에서 호출)
unsafe fn is_marked_private(f: &Formats) -> bool {
    if has(f.own) || has(f.exclude_monitor) || has(f.viewer_ignore) {
        return true;
    }
    // CanIncludeInClipboardHistory = 0 이면 "기록 금지"
    if has(f.can_include_history) {
        if let Some(b) = get_bytes(f.can_include_history) {
            if b.len() >= 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) == 0 {
                return true;
            }
        }
    }
    false
}

pub fn read(owner: HWND, f: &Formats, opt: &ReadOptions) -> ReadResult {
    let Some(_guard) = Opened::open(owner) else { return ReadResult::Busy };
    unsafe {
        if is_marked_private(f) {
            return ReadResult::Skip;
        }
        if opt.files && has(CF_HDROP as u32) {
            let h = GetClipboardData(CF_HDROP as u32) as HDROP;
            if !h.is_null() {
                let count = DragQueryFileW(h, u32::MAX, null_mut(), 0);
                let mut paths = Vec::new();
                for i in 0..count {
                    let len = DragQueryFileW(h, i, null_mut(), 0);
                    let mut buf = vec![0u16; len as usize + 1];
                    DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
                    paths.push(String::from_utf16_lossy(&buf[..len as usize]));
                }
                if !paths.is_empty() {
                    return ReadResult::Got(Captured::Files { paths });
                }
            }
        }
        if opt.text && has(CF_UNICODETEXT as u32) {
            if let Some(bytes) = get_bytes(CF_UNICODETEXT as u32) {
                if bytes.len() > opt.max_text_bytes * 2 {
                    return ReadResult::Skip;
                }
                let text = utf16_le_to_string(&bytes);
                let html = if opt.html && has(f.html) {
                    get_bytes(f.html).map(|b| {
                        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                        String::from_utf8_lossy(&b[..end]).into_owned()
                    })
                } else {
                    None
                };
                return ReadResult::Got(Captured::Text { text, html });
            }
        }
        if opt.images && has(CF_DIB as u32) {
            if let Some(dib) = get_bytes(CF_DIB as u32) {
                if dib.len() > opt.max_image_bytes {
                    return ReadResult::Skip;
                }
                if clipstash::bmp::dib_size(&dib).is_some() {
                    return ReadResult::Got(Captured::Image { dib });
                }
            }
        }
    }
    ReadResult::Nothing
}

/// 지금 클립보드의 텍스트만 읽기({clipboard} 템플릿용)
pub fn read_text(owner: HWND) -> Option<String> {
    let _g = Opened::open(owner)?;
    unsafe { get_bytes(CF_UNICODETEXT as u32).map(|b| utf16_le_to_string(&b)) }
}

/// 이 앱이 쓰는 클립보드 데이터
pub enum Payload<'a> {
    Text { text: &'a str, html: Option<&'a str> },
    Image { dib: &'a [u8] },
    Files { paths: &'a [String] },
}

unsafe fn set_bytes(fmt: u32, data: &[u8]) -> bool {
    let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
    if h.is_null() {
        return false;
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return false;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
    GlobalUnlock(h);
    if SetClipboardData(fmt, h).is_null() {
        GlobalFree(h);
        return false;
    }
    true
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()).collect()
}

/// 혼자 있는 `\n` 을 Windows 줄바꿈 `\r\n` 으로 바꿉니다.
pub fn crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let mut prev = '\0';
    for c in s.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    out
}

pub fn write(owner: HWND, f: &Formats, payload: Payload) -> bool {
    let Some(_guard) = Opened::open(owner) else { return false };
    unsafe {
        EmptyClipboard();
        let ok = match payload {
            Payload::Text { text, html } => {
                let ok = set_bytes(CF_UNICODETEXT as u32, &utf16_bytes(&crlf(text)));
                if let Some(h) = html {
                    let mut b = h.as_bytes().to_vec();
                    b.push(0);
                    set_bytes(f.html, &b);
                }
                ok
            }
            Payload::Image { dib } => set_bytes(CF_DIB as u32, dib),
            Payload::Files { paths } => {
                // DROPFILES 구조체 뒤에 "경로\0경로\0\0" (UTF-16)
                let header = std::mem::size_of::<DROPFILES>();
                let mut data = vec![0u8; header];
                let df = DROPFILES { pFiles: header as u32, pt: POINT { x: 0, y: 0 }, fNC: 0, fWide: 1 };
                std::ptr::copy_nonoverlapping(&df as *const DROPFILES as *const u8, data.as_mut_ptr(), header);
                for p in paths {
                    data.extend(p.encode_utf16().chain(std::iter::once(0)).flat_map(|u| u.to_le_bytes()));
                }
                data.extend([0, 0]);
                let ok = set_bytes(CF_HDROP as u32, &data);
                set_bytes(f.drop_effect, &1u32.to_le_bytes()); // DROPEFFECT_COPY
                set_bytes(CF_UNICODETEXT as u32, &utf16_bytes(&paths.join("\r\n")));
                ok
            }
        };
        // 자기 자신이 넣은 것이라는 표시
        set_bytes(f.own, &[1]);
        ok
    }
}
