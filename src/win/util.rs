//! 자잘한 Win32 도우미 함수들.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use clipstash::template::LocalTime;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Rust 문자열 → NUL 로 끝나는 UTF-16
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 고정 크기 버퍼(szTip 등)에 문자열을 잘라서 복사
pub fn copy_to_buf(buf: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(buf.len() - 1).collect();
    buf[..w.len()].copy_from_slice(&w);
    buf[w.len()] = 0;
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn local_time() -> LocalTime {
    let st = unsafe {
        let mut st = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    LocalTime {
        year: st.wYear,
        month: st.wMonth as u8,
        day: st.wDay as u8,
        hour: st.wHour as u8,
        minute: st.wMinute as u8,
        second: st.wSecond as u8,
        weekday: st.wDayOfWeek as u8,
    }
}

pub fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("clipstash.exe"))
}

/// 데이터 폴더. 실행 파일 옆에 `data` 폴더나 `portable.txt` 가 있으면 휴대용 모드로
/// 그 폴더를 쓰고, 아니면 `%APPDATA%\ClipStash` 를 씁니다.
pub fn data_dir() -> PathBuf {
    let exe = exe_path();
    if let Some(dir) = exe.parent() {
        if dir.join("data").is_dir() || dir.join("portable.txt").exists() {
            return dir.join("data");
        }
    }
    match std::env::var_os("APPDATA") {
        Some(a) => PathBuf::from(a).join("ClipStash"),
        None => exe.parent().map(|d| d.join("data")).unwrap_or_else(|| PathBuf::from("data")),
    }
}

/// 창을 만든 프로세스의 실행 파일 이름(예: "chrome.exe")
pub fn process_name_of_window(hwnd: HWND) -> Option<String> {
    if hwnd.is_null() {
        return None;
    }
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        if ok == 0 {
            return None;
        }
        let full = OsString::from_wide(&buf[..len as usize]);
        Path::new(&full).file_name().map(|n| n.to_string_lossy().into_owned())
    }
}

pub fn is_own_window(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    pid == std::process::id()
}

pub fn shell_open(file: &str, params: Option<&str>) {
    let op = wide("open");
    let f = wide(file);
    let p = params.map(wide);
    unsafe {
        ShellExecuteW(
            null_mut(),
            op.as_ptr(),
            f.as_ptr(),
            p.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn message_box(hwnd: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = wide(text);
    let c = wide("ClipStash");
    unsafe { MessageBoxW(hwnd, t.as_ptr(), c.as_ptr(), flags) }
}

// ── 시작 프로그램 등록(HKCU\...\Run) ───────────────────────────

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "ClipStash";

fn open_run_key(access: REG_SAM_FLAGS) -> Option<HKEY> {
    let key = wide(RUN_KEY);
    let mut h: HKEY = null_mut();
    let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, access, &mut h) };
    (r == ERROR_SUCCESS).then_some(h)
}

fn autostart_command() -> String {
    format!("\"{}\"", exe_path().display())
}

pub fn autostart_enabled() -> bool {
    let Some(h) = open_run_key(KEY_QUERY_VALUE) else { return false };
    let name = wide(RUN_VALUE);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        let r = RegQueryValueExW(
            h,
            name.as_ptr(),
            null_mut(),
            null_mut(),
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(h);
        r
    };
    // 실행 파일을 옮겼다면 꺼진 것으로 봅니다(다시 켜면 새 경로로 등록).
    r == ERROR_SUCCESS && from_wide(&buf).eq_ignore_ascii_case(&autostart_command())
}

pub fn set_autostart(enable: bool) -> bool {
    let Some(h) = open_run_key(KEY_SET_VALUE) else { return false };
    let name = wide(RUN_VALUE);
    let r = unsafe {
        let r = if enable {
            let v = wide(&autostart_command());
            RegSetValueExW(h, name.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32)
        } else {
            let r = RegDeleteValueW(h, name.as_ptr());
            if r == ERROR_FILE_NOT_FOUND { ERROR_SUCCESS } else { r }
        };
        RegCloseKey(h);
        r
    };
    r == ERROR_SUCCESS
}
