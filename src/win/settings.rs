//! 설정 창: 자주 바꾸는 설정을 클릭으로 바꿉니다. 저장하면 config.toml 의 해당 줄만 바꾸고
//! (주석·템플릿·제외 목록은 그대로) 곧바로 다시 읽어 적용합니다.

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};

use clipstash::config::{set_top_level_values, Config, PopupPosition, Theme, DEFAULT_CONFIG_TOML};
use clipstash::hotkey;
use toml::Value;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::{self, wide};
use super::{app, App};

const CLASS: &str = "ClipStashSettings";

// STATIC 스타일(windows-sys 의 SystemServices 기능 없이 씀)
const SS_LEFT: u32 = 0x0;
const SS_CENTER: u32 = 0x1;
const SS_CENTERIMAGE: u32 = 0x200;

static HWND_NOW: AtomicIsize = AtomicIsize::new(0);
static FONT: AtomicIsize = AtomicIsize::new(0);

// 컨트롤 id
const ID_HOTKEY: i32 = 101;
const ID_CAPTURE: i32 = 102;
const ID_POSITION: i32 = 103;
const ID_THEME: i32 = 104;
const ID_OPACITY: i32 = 105;
const ID_WIDTH: i32 = 106;
const ID_HEIGHT: i32 = 107;
const ID_FONT_SIZE: i32 = 108;
const ID_MAX_ITEMS: i32 = 109;
const ID_POPUP_ROWS: i32 = 113;
const ID_SEPARATOR: i32 = 110;
const ID_AI_KEY: i32 = 111;
const ID_SYNC: i32 = 112;
const ID_PASTE_AFTER: i32 = 121;
const ID_PLAIN: i32 = 122;
const ID_TO_TOP: i32 = 123;
const ID_PREVIEW: i32 = 124;
const ID_DOUBLE_COPY: i32 = 125;
const ID_IMAGES: i32 = 126;
const ID_AUTOSTART: i32 = 127;
const ID_OPEN_FILE: i32 = 131;

const POSITIONS: [(&str, PopupPosition, &str); 3] = [
    ("입력 커서 옆", PopupPosition::Caret, "caret"),
    ("마우스 위치", PopupPosition::Mouse, "mouse"),
    ("화면 가운데", PopupPosition::Center, "center"),
];
const THEMES: [(&str, Theme, &str); 2] = [("어둡게", Theme::Dark, "dark"), ("밝게", Theme::Light, "light")];
const SEPARATORS: [(&str, &str); 5] =
    [("줄바꿈", "\n"), ("빈 줄 하나", "\n\n"), ("쉼표 (, )", ", "), ("탭", "\t"), ("띄어쓰기", " ")];

/// 설정 창이 떠 있으면 그 창(메시지 루프에서 Tab/Enter/Esc 처리용)
pub fn dialog_hwnd() -> HWND {
    HWND_NOW.load(Ordering::Relaxed) as HWND
}

pub fn register(hinst: HINSTANCE) {
    unsafe {
        let cls = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc_),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            hIcon: LoadIconW(hinst, 1 as _),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
}

pub fn open(a: &mut App) {
    super::popup::hide(a);
    let existing = dialog_hwnd();
    unsafe {
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let dpi = GetDpiForSystem().max(96) as i32;
        let s = |v: i32| v * dpi / 96;

        let font = CreateFontW(
            -(9 * dpi / 72),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("맑은 고딕").as_ptr(),
        );
        FONT.store(font as isize, Ordering::Relaxed);

        let rows = 12;
        let checks = 7;
        let row_h = s(30);
        let client_w = s(470);
        let client_h = s(14) + rows * row_h + s(8) + checks * s(26) + s(16) + s(30) + s(14);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
        AdjustWindowRectEx(&mut r, style, 0, 0);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;

        let cls = wide(CLASS);
        let title = wide("ClipStash 설정");
        let hwnd = CreateWindowExW(0, cls.as_ptr(), title.as_ptr(), style, x, y, w, h, null_mut(), null_mut(), a.hinst, null());
        if hwnd.is_null() {
            return;
        }
        HWND_NOW.store(hwnd as isize, Ordering::Relaxed);

        let c = &a.config;
        let label_x = s(16);
        let label_w = s(170);
        let ctl_x = s(190);
        let ctl_w = client_w - ctl_x - s(16);
        let mut y = s(14);
        let row = |text: &str, y: &mut i32| {
            child(hwnd, "STATIC", text, SS_LEFT | SS_CENTERIMAGE, label_x, *y, label_w, s(24), 0);
            let at = *y;
            *y += row_h;
            at
        };
        let edit = |id: i32, text: &str, x: i32, y: i32, w: i32, extra: u32| {
            child(hwnd, "EDIT", text, ES_AUTOHSCROLL as u32 | WS_TABSTOP | WS_BORDER | extra, x, y, w, s(24), id)
        };
        let combo = |id: i32, items: &[&str], sel: usize, y: i32| {
            let cb = child(hwnd, "COMBOBOX", "", CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL, ctl_x, y, ctl_w, s(200), id);
            for it in items {
                let w = wide(it);
                SendMessageW(cb, CB_ADDSTRING, 0, w.as_ptr() as LPARAM);
            }
            SendMessageW(cb, CB_SETCURSEL, sel, 0);
        };

        let yy = row("팝업 여는 단축키", &mut y);
        edit(ID_HOTKEY, &c.hotkey, ctl_x, yy, ctl_w, 0);
        let yy = row("화면 캡처 단축키", &mut y);
        edit(ID_CAPTURE, &c.capture_hotkey, ctl_x, yy, ctl_w, 0);
        let yy = row("팝업 위치", &mut y);
        let pos = POSITIONS.iter().position(|p| p.1 == c.popup_position).unwrap_or(0);
        combo(ID_POSITION, &POSITIONS.map(|p| p.0), pos, yy);
        let yy = row("테마", &mut y);
        let th = THEMES.iter().position(|t| t.1 == c.theme).unwrap_or(0);
        combo(ID_THEME, &THEMES.map(|t| t.0), th, yy);
        let yy = row("투명도 (50~100%, 낮을수록 투명)", &mut y);
        edit(ID_OPACITY, &c.popup_opacity.to_string(), ctl_x, yy, s(70), ES_NUMBER as u32);
        let yy = row("팝업 크기 (가로 × 세로)", &mut y);
        edit(ID_WIDTH, &c.popup_width.to_string(), ctl_x, yy, s(70), ES_NUMBER as u32);
        child(hwnd, "STATIC", "×", SS_CENTER | SS_CENTERIMAGE, ctl_x + s(72), yy, s(20), s(24), 0);
        edit(ID_HEIGHT, &c.popup_height.to_string(), ctl_x + s(94), yy, s(70), ES_NUMBER as u32);
        let yy = row("글자 크기", &mut y);
        edit(ID_FONT_SIZE, &c.font_size.to_string(), ctl_x, yy, s(70), ES_NUMBER as u32);
        let yy = row("팝업에 보일 최근 항목 수", &mut y);
        edit(ID_POPUP_ROWS, &c.popup_rows.to_string(), ctl_x, yy, s(70), ES_NUMBER as u32);
        let yy = row("보관할 기록 개수", &mut y);
        edit(ID_MAX_ITEMS, &c.max_items.to_string(), ctl_x, yy, s(70), ES_NUMBER as u32);
        let yy = row("여러 개 합칠 때 사이에", &mut y);
        let mut seps: Vec<&str> = SEPARATORS.iter().map(|s| s.0).collect();
        let sep = match SEPARATORS.iter().position(|s| s.1 == c.merge_separator) {
            Some(i) => i,
            None => {
                seps.push("지금 값 그대로 (설정 파일)");
                seps.len() - 1
            }
        };
        combo(ID_SEPARATOR, &seps, sep, yy);
        let yy = row("AI 글자 추출 키 (선택)", &mut y);
        edit(ID_AI_KEY, &c.anthropic_api_key, ctl_x, yy, ctl_w, ES_PASSWORD as u32);
        let yy = row("동기화 폴더 (선택)", &mut y);
        edit(ID_SYNC, &c.sync_folder, ctl_x, yy, ctl_w, 0);

        y += s(8);
        let checks: [(i32, &str, bool); 7] = [
            (ID_PASTE_AFTER, "고르면 바로 붙여넣기 (끄면 복사만)", c.paste_after_select),
            (ID_PLAIN, "기본으로 서식 없이 텍스트만 붙여넣기", c.plain_text_by_default),
            (ID_TO_TOP, "붙여넣은 항목을 목록 맨 위로 올리기", c.move_to_top_on_paste),
            (ID_PREVIEW, "팝업을 열 때 미리보기 칸 켜기", c.show_preview),
            (ID_DOUBLE_COPY, "Ctrl+C 를 빠르게 두 번 눌러도 팝업 열기", c.double_copy_ms > 0),
            (ID_IMAGES, "이미지도 기록하기", c.capture_images),
            (ID_AUTOSTART, "Windows 시작할 때 자동 실행", util::autostart_enabled()),
        ];
        for (id, text, on) in checks {
            let b = child(hwnd, "BUTTON", text, BS_AUTOCHECKBOX as u32 | WS_TABSTOP, label_x, y, client_w - 2 * label_x, s(24), id);
            SendMessageW(b, BM_SETCHECK, if on { BST_CHECKED as usize } else { 0 }, 0);
            y += s(26);
        }

        let by = client_h - s(14) - s(30);
        child(hwnd, "BUTTON", "설정 파일 직접 열기", BS_PUSHBUTTON as u32 | WS_TABSTOP, label_x, by, s(150), s(30), ID_OPEN_FILE);
        child(hwnd, "BUTTON", "저장", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, client_w - s(16) - s(90) * 2 - s(8), by, s(90), s(30), IDOK);
        child(hwnd, "BUTTON", "취소", BS_PUSHBUTTON as u32 | WS_TABSTOP, client_w - s(16) - s(90), by, s(90), s(30), IDCANCEL);

        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(GetDlgItem(hwnd, ID_HOTKEY));
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn child(parent: HWND, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    let c = wide(class);
    let t = wide(text);
    let hwnd = CreateWindowExW(
        0,
        c.as_ptr(),
        t.as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as isize as HMENU,
        null_mut(),
        null(),
    );
    SendMessageW(hwnd, WM_SETFONT, FONT.load(Ordering::Relaxed) as usize, 1);
    hwnd
}

unsafe fn text_of(hwnd: HWND, id: i32) -> String {
    let h = GetDlgItem(hwnd, id);
    let n = GetWindowTextLengthW(h).max(0) as usize;
    let mut buf = vec![0u16; n + 1];
    GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    util::from_wide(&buf).trim().to_string()
}

unsafe fn checked(hwnd: HWND, id: i32) -> bool {
    SendMessageW(GetDlgItem(hwnd, id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize
}

unsafe fn combo_sel(hwnd: HWND, id: i32) -> usize {
    SendMessageW(GetDlgItem(hwnd, id), CB_GETCURSEL, 0, 0).max(0) as usize
}

unsafe fn number(hwnd: HWND, id: i32, name: &str, lo: i64, hi: i64) -> Result<Value, String> {
    let t = text_of(hwnd, id);
    let n: i64 = t.parse().map_err(|_| format!("{} 에는 숫자를 넣어 주세요.", name))?;
    Ok(Value::Integer(n.clamp(lo, hi)))
}

/// 창의 값을 모아 설정 파일에 씁니다. 잘못된 값이 있으면 그 이유를 돌려줍니다.
unsafe fn save(a: &mut App, hwnd: HWND) -> Result<(), String> {
    let hk = text_of(hwnd, ID_HOTKEY);
    if !hk.is_empty() {
        hotkey::parse(&hk).map_err(|e| format!("팝업 단축키를 알아볼 수 없습니다: {}\n예: Alt+C, Ctrl+Shift+V", e))?;
    }
    let cap = text_of(hwnd, ID_CAPTURE);
    if !cap.is_empty() {
        hotkey::parse(&cap).map_err(|e| format!("캡처 단축키를 알아볼 수 없습니다: {}\n예: Ctrl+Alt+C", e))?;
    }
    if !hk.is_empty() && hk.eq_ignore_ascii_case(&cap) {
        return Err("팝업 단축키와 캡처 단축키가 같습니다.".into());
    }
    let mut v: Vec<(&str, Value)> = vec![
        ("hotkey", Value::String(hk)),
        ("capture_hotkey", Value::String(cap)),
        ("popup_position", Value::String(POSITIONS[combo_sel(hwnd, ID_POSITION).min(2)].2.into())),
        ("theme", Value::String(THEMES[combo_sel(hwnd, ID_THEME).min(1)].2.into())),
        ("popup_opacity", number(hwnd, ID_OPACITY, "투명도", 50, 100)?),
        ("popup_width", number(hwnd, ID_WIDTH, "팝업 가로", 240, 2000)?),
        ("popup_height", number(hwnd, ID_HEIGHT, "팝업 세로", 200, 2000)?),
        ("font_size", number(hwnd, ID_FONT_SIZE, "글자 크기", 6, 40)?),
        ("max_items", number(hwnd, ID_MAX_ITEMS, "보관 개수", 10, 100_000)?),
        ("popup_rows", number(hwnd, ID_POPUP_ROWS, "팝업에 보일 개수", 5, 2000)?),
        ("anthropic_api_key", Value::String(text_of(hwnd, ID_AI_KEY))),
        ("sync_folder", Value::String(text_of(hwnd, ID_SYNC))),
        ("paste_after_select", Value::Boolean(checked(hwnd, ID_PASTE_AFTER))),
        ("plain_text_by_default", Value::Boolean(checked(hwnd, ID_PLAIN))),
        ("move_to_top_on_paste", Value::Boolean(checked(hwnd, ID_TO_TOP))),
        ("show_preview", Value::Boolean(checked(hwnd, ID_PREVIEW))),
        ("capture_images", Value::Boolean(checked(hwnd, ID_IMAGES))),
    ];
    if let Some(sep) = SEPARATORS.get(combo_sel(hwnd, ID_SEPARATOR)) {
        v.push(("merge_separator", Value::String(sep.1.into())));
    }
    let double = match (checked(hwnd, ID_DOUBLE_COPY), a.config.double_copy_ms) {
        (false, _) => 0,
        (true, 0) => 400,
        (true, ms) => ms,
    };
    v.push(("double_copy_ms", Value::Integer(double as i64)));

    let path = a.data_dir.join(clipstash::storage::CONFIG_FILE);
    let old = std::fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG_TOML.to_string());
    let new = set_top_level_values(&old, &v);
    Config::from_toml(&new).map_err(|e| format!("설정을 만들지 못했습니다: {}", e))?;
    if new != old {
        std::fs::write(&path, new).map_err(|e| format!("설정 파일에 쓰지 못했습니다: {}", e))?;
    }

    let want_auto = checked(hwnd, ID_AUTOSTART);
    if want_auto != util::autostart_enabled() && !util::set_autostart(want_auto) {
        return Err("자동 실행 설정을 바꾸지 못했습니다(레지스트리에 쓸 수 없음).".into());
    }
    a.reload_config(false);
    Ok(())
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CTLCOLORSTATIC => {
            let dc = wp as HDC;
            SetBkColor(dc, GetSysColor(COLOR_WINDOW));
            SetTextColor(dc, GetSysColor(COLOR_WINDOWTEXT));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as i32;
            match id {
                IDOK => {
                    let a = app();
                    match save(a, hwnd) {
                        Ok(()) => {
                            DestroyWindow(hwnd);
                            a.balloon("설정을 저장했습니다", &format!("팝업 열기: {}", a.config.shortcut_label()), false);
                        }
                        Err(e) => {
                            util::message_box(hwnd, &e, MB_OK | MB_ICONWARNING);
                        }
                    }
                }
                IDCANCEL => {
                    DestroyWindow(hwnd);
                }
                ID_OPEN_FILE => {
                    DestroyWindow(hwnd);
                    app().open_config_file();
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            HWND_NOW.store(0, Ordering::Relaxed);
            let f = FONT.swap(0, Ordering::Relaxed);
            if f != 0 {
                DeleteObject(f as HGDIOBJ);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
