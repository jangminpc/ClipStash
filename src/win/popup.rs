//! 단축키(Ctrl+C 두 번 등)로 뜨는 기록 팝업 창.
//!
//! 단순하고 반투명한 디자인:
//! ┌──────────────────────────────────────┐
//! │  [ 검색…                ]  기록 고정 템플릿 │  ← 검색창 + 탭(Tab 키)
//! │  1  회의록 정리 …           메모장 · 방금 │
//! │ ❷  https://…              chrome · 3분 전 │  ← ❶❷ = 여러 개 고른 순서 표시
//! │ ❶  안녕하세요               카톡 · 5분 전 │
//! │  (F2 로 미리보기 켜기)                    │
//! │  2개 선택됨 · Enter 한 번에 붙여넣기      │  ← 한 줄 도움말
//! └──────────────────────────────────────┘

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use clipstash::config::{PopupPosition, Theme};
use clipstash::history::{relative_time, ClipContent, View};
use clipstash::search::Query;
use clipstash::transform::Transform;
use clipstash::{bmp, storage};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::{self, wide};
use super::{app, Action, App};

const POPUP_CLASS: &str = "ClipStash.Popup";
const PREVIEW_CLASS: &str = "ClipStash.Preview";
const ID_EDIT: usize = 10;
const ID_LIST: usize = 11;

const MAX_ROWS: usize = 2000;

// DWM 속성(Windows 11 에서 둥근 모서리/어두운 테두리). 이전 Windows 에서는 조용히 무시됩니다.
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWA_BORDER_COLOR: u32 = 34;
const DWMWCP_ROUND: u32 = 2;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// 테마 색상
#[derive(Clone, Copy)]
struct Palette {
    bg: COLORREF,
    surface: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    accent: COLORREF,
    on_accent: COLORREF,
    sel: COLORREF,
    pin: COLORREF,
    line: COLORREF,
    warn: COLORREF,
}

const DARK: Palette = Palette {
    bg: rgb(30, 31, 34),
    surface: rgb(45, 47, 52),
    text: rgb(234, 236, 239),
    muted: rgb(138, 143, 152),
    accent: rgb(92, 160, 255),
    on_accent: rgb(255, 255, 255),
    sel: rgb(50, 64, 90),
    pin: rgb(255, 190, 40),
    line: rgb(60, 62, 68),
    warn: rgb(255, 110, 100),
};

const LIGHT: Palette = Palette {
    bg: rgb(252, 252, 253),
    surface: rgb(239, 241, 244),
    text: rgb(28, 30, 34),
    muted: rgb(118, 124, 134),
    accent: rgb(38, 110, 220),
    on_accent: rgb(255, 255, 255),
    sel: rgb(222, 234, 252),
    pin: rgb(225, 145, 0),
    line: rgb(226, 228, 232),
    warn: rgb(210, 60, 50),
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    History,
    Pinned,
    Templates,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::History, Tab::Pinned, Tab::Templates];
    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap()
    }
    fn name(self) -> &'static str {
        match self {
            Tab::History => "기록",
            Tab::Pinned => "고정",
            Tab::Templates => "템플릿",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowRef {
    Clip(u64),
    Template(usize),
}

pub struct Row {
    pub r: RowRef,
    pub title: String,
    pub meta: String,
    pub pinned: bool,
    /// 이미지/파일처럼 텍스트가 아닌 항목(강조 색으로 표시)
    pub special: bool,
}

pub struct Popup {
    pub hwnd: HWND,
    edit: HWND,
    list: HWND,
    preview: HWND,
    font: HFONT,
    font_bold: HFONT,
    font_small: HFONT,
    bg_brush: HBRUSH,
    surface_brush: HBRUSH,
    pal: Palette,
    dpi: u32,
    font_h: i32,
    small_h: i32,
    header_h: i32,
    footer_h: i32,
    tab_rects: [RECT; 3],
    pub tab: Tab,
    pub rows: Vec<Row>,
    /// 여러 개 붙여넣기용으로 고른 항목(고른 순서)
    pub marks: Vec<u64>,
    pub target: HWND,
    show_preview: bool,
    image_cache: Option<(u64, Vec<u8>)>,
}

impl Default for Popup {
    fn default() -> Self {
        Popup {
            hwnd: null_mut(),
            edit: null_mut(),
            list: null_mut(),
            preview: null_mut(),
            font: null_mut(),
            font_bold: null_mut(),
            font_small: null_mut(),
            bg_brush: null_mut(),
            surface_brush: null_mut(),
            pal: DARK,
            dpi: 0,
            font_h: 16,
            small_h: 14,
            header_h: 40,
            footer_h: 24,
            tab_rects: [RECT { left: 0, top: 0, right: 0, bottom: 0 }; 3],
            tab: Tab::History,
            rows: Vec::new(),
            marks: Vec::new(),
            target: null_mut(),
            show_preview: false,
            image_cache: None,
        }
    }
}

impl Popup {
    pub fn visible(&self) -> bool {
        !self.hwnd.is_null() && unsafe { IsWindowVisible(self.hwnd) } != 0
    }

    fn scale(&self, v: i32) -> i32 {
        v * self.dpi as i32 / 96
    }

    fn selected_index(&self) -> Option<usize> {
        let i = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        (i >= 0 && (i as usize) < self.rows.len()).then_some(i as usize)
    }

    fn selected(&self) -> Option<RowRef> {
        self.selected_index().map(|i| self.rows[i].r)
    }

    fn search_text(&self) -> String {
        unsafe {
            let len = GetWindowTextLengthW(self.edit);
            let mut buf = vec![0u16; len as usize + 1];
            GetWindowTextW(self.edit, buf.as_mut_ptr(), buf.len() as i32);
            util::from_wide(&buf)
        }
    }

    fn mark_order(&self, id: u64) -> Option<usize> {
        self.marks.iter().position(|m| *m == id)
    }
}

pub fn register_classes(hinst: HINSTANCE) {
    unsafe {
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let cls = wide(POPUP_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(popup_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: cursor,
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
        let cls2 = wide(PREVIEW_CLASS);
        let wc2 = WNDCLASSEXW { lpfnWndProc: Some(preview_proc), lpszClassName: cls2.as_ptr(), style: 0, ..wc };
        RegisterClassExW(&wc2);
    }
}

static EDIT_PROC: AtomicIsize = AtomicIsize::new(0);
/// 처리한 키(Ctrl+Space 등)가 만드는 글자를 검색창에 넣지 않기 위한 표시
static SWALLOW_CHAR: AtomicBool = AtomicBool::new(false);

pub fn create(a: &mut App) {
    unsafe {
        let cls = wide(POPUP_CLASS);
        let title = wide("ClipStash");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            400,
            400,
            null_mut(),
            null_mut(),
            a.hinst,
            null(),
        );
        let edit_cls = wide("EDIT");
        let edit = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            0,
            0,
            10,
            10,
            hwnd,
            ID_EDIT as HMENU,
            a.hinst,
            null(),
        );
        let cue = wide("검색  (초성 OK: ㅎㄱ → 한국)");
        SendMessageW(edit, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
        let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        EDIT_PROC.store(old, Ordering::Relaxed);

        let lb_cls = wide("LISTBOX");
        let list = CreateWindowExW(
            0,
            lb_cls.as_ptr(),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_NODATA | LBS_NOINTEGRALHEIGHT) as u32,
            0,
            0,
            10,
            10,
            hwnd,
            ID_LIST as HMENU,
            a.hinst,
            null(),
        );
        let pv_cls = wide(PREVIEW_CLASS);
        let preview = CreateWindowExW(
            0,
            pv_cls.as_ptr(),
            null(),
            WS_CHILD,
            0,
            0,
            10,
            10,
            hwnd,
            null_mut(),
            a.hinst,
            null(),
        );
        let p = &mut a.popup;
        p.hwnd = hwnd;
        p.edit = edit;
        p.list = list;
        p.preview = preview;
        // Windows 11: 둥근 모서리
        let round = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const u32 as *const _, 4);
        apply_theme(a);
        set_dpi(a, 96);
    }
}

/// 테마 색, 투명도, 스크롤바 모양을 설정에 맞춥니다.
fn apply_theme(a: &mut App) {
    let dark = a.config.theme == Theme::Dark;
    let p = &mut a.popup;
    p.pal = if dark { DARK } else { LIGHT };
    p.show_preview = a.config.show_preview;
    unsafe {
        for b in [p.bg_brush, p.surface_brush] {
            if !b.is_null() {
                DeleteObject(b as HGDIOBJ);
            }
        }
        p.bg_brush = CreateSolidBrush(p.pal.bg);
        p.surface_brush = CreateSolidBrush(p.pal.surface);
        let alpha = (a.config.popup_opacity as u32 * 255 / 100) as u8;
        SetLayeredWindowAttributes(p.hwnd, 0, alpha, LWA_ALPHA);
        let dark_flag: u32 = dark as u32;
        DwmSetWindowAttribute(p.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark_flag as *const u32 as *const _, 4);
        let border = p.pal.line;
        DwmSetWindowAttribute(p.hwnd, DWMWA_BORDER_COLOR, &border as *const u32 as *const _, 4);
        // 목록 스크롤바도 테마에 맞게(Windows 10 1809 이상)
        let theme = wide(if dark { "DarkMode_Explorer" } else { "Explorer" });
        SetWindowTheme(p.list, theme.as_ptr(), null());
        ShowWindow(p.preview, if p.show_preview { SW_SHOWNA } else { SW_HIDE });
    }
}

fn make_font(name: &str, pt: u32, dpi: u32, bold: bool) -> HFONT {
    let face = wide(name);
    unsafe {
        CreateFontW(
            -((pt * dpi) as i32 / 72),
            0,
            0,
            0,
            if bold { FW_SEMIBOLD as i32 } else { FW_NORMAL as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

fn text_height(hwnd: HWND, font: HFONT) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        let old = SelectObject(dc, font as HGDIOBJ);
        let mut tm: TEXTMETRICW = std::mem::zeroed();
        GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
        tm.tmHeight
    }
}

fn row_height(p: &Popup) -> i32 {
    p.font_h + p.scale(16)
}

/// DPI(또는 글꼴 설정)가 바뀌면 글꼴을 다시 만듭니다.
fn set_dpi(a: &mut App, dpi: u32) {
    let p = &mut a.popup;
    p.dpi = dpi;
    unsafe {
        for f in [p.font, p.font_bold, p.font_small] {
            if !f.is_null() {
                DeleteObject(f as HGDIOBJ);
            }
        }
    }
    let size = a.config.font_size;
    p.font = make_font(&a.config.font_name, size, dpi, false);
    p.font_bold = make_font(&a.config.font_name, size, dpi, true);
    p.font_small = make_font(&a.config.font_name, size.saturating_sub(1).max(6), dpi, false);
    p.font_h = text_height(p.hwnd, p.font);
    p.small_h = text_height(p.hwnd, p.font_small);
    p.header_h = p.font_h + p.scale(28);
    p.footer_h = p.small_h + p.scale(14);
    unsafe {
        SendMessageW(p.edit, WM_SETFONT, p.font as WPARAM, 1);
        SendMessageW(p.list, LB_SETITEMHEIGHT, 0, row_height(p) as LPARAM);
    }
}

/// 탭 글자 영역의 너비(오른쪽 위)
fn tabs_width(p: &Popup) -> i32 {
    unsafe {
        let dc = GetDC(p.hwnd);
        let w: i32 = Tab::ALL.iter().map(|t| text_width(dc, t.name(), p.font_bold) + p.scale(14)).sum();
        ReleaseDC(p.hwnd, dc);
        w + p.scale(6)
    }
}

/// 검색창 배경 상자(둥근 사각형)
fn search_box(p: &Popup, client_w: i32) -> RECT {
    let pad = p.scale(10);
    let top = p.scale(10);
    RECT { left: pad, top, right: client_w - pad - tabs_width(p), bottom: p.header_h - p.scale(6) }
}

fn layout(p: &mut Popup) {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(p.hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let sb = search_box(p, w);
        let inset = p.scale(10);
        let edit_h = p.font_h + p.scale(2);
        let edit_y = sb.top + (sb.bottom - sb.top - edit_h) / 2;
        MoveWindow(p.edit, sb.left + inset, edit_y, sb.right - sb.left - 2 * inset, edit_h, 1);

        let list_y = p.header_h + p.scale(4);
        let avail = h - list_y - p.footer_h;
        let preview_h = if p.show_preview { (avail * 32 / 100).max(p.font_h * 4) } else { 0 };
        let list_h = avail - preview_h;
        MoveWindow(p.list, p.scale(6), list_y, w - p.scale(12), list_h, 1);
        if p.show_preview {
            let pad = p.scale(10);
            MoveWindow(p.preview, pad, list_y + list_h + p.scale(4), w - 2 * pad, preview_h - p.scale(8), 1);
        }
    }
}

/// 팝업을 엽니다. `target` 은 붙여넣을 창(없으면 클립보드에 복사만).
pub fn show(a: &mut App, target: HWND, at_mouse: bool) {
    a.reload_config(false);
    let target = if util::is_own_window(target) { null_mut() } else { target };
    a.popup.target = target;

    unsafe {
        // 위치: 입력 커서(캐럿) → 마우스 → 화면 가운데
        let mut anchor: Option<(POINT, i32)> = None; // (점, 캐럿 높이)
        if !at_mouse && a.config.popup_position == PopupPosition::Caret && !target.is_null() {
            anchor = caret_point(target);
        }
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        let (pt, caret_h) = anchor.unwrap_or((pt, 0));
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 || dx == 0 {
            dx = 96;
        }
        if dx != a.popup.dpi {
            set_dpi(a, dx);
        }
        let p = &mut a.popup;
        let w = p.scale(a.config.popup_width as i32);
        let h = p.scale(a.config.popup_height as i32);
        let wa = mi.rcWork;
        let (mut x, mut y);
        if a.config.popup_position == PopupPosition::Center && !at_mouse {
            x = wa.left + (wa.right - wa.left - w) / 2;
            y = wa.top + (wa.bottom - wa.top - h) / 2;
        } else {
            x = pt.x;
            y = pt.y + caret_h + p.scale(4);
            if x + w > wa.right {
                x = wa.right - w;
            }
            if y + h > wa.bottom {
                // 아래 공간이 모자라면 위로
                y = if caret_h > 0 { pt.y - h - p.scale(4) } else { wa.bottom - h };
            }
        }
        x = x.max(wa.left);
        y = y.max(wa.top);
        SetWindowPos(p.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);

        p.tab = Tab::History;
        p.marks.clear();
        p.image_cache = None;
        p.show_preview = a.config.show_preview;
        ShowWindow(p.preview, if p.show_preview { SW_SHOWNA } else { SW_HIDE });
        layout(p);
        let empty = wide("");
        SetWindowTextW(p.edit, empty.as_ptr()); // EN_CHANGE → refresh
        refresh(a);
        select(&a.popup, 0);
        let p = &mut a.popup;
        ShowWindow(p.hwnd, SW_SHOW);
        SetForegroundWindow(p.hwnd);
        SetFocus(p.edit);
    }
}

pub fn hide(a: &mut App) {
    let p = &mut a.popup;
    if p.visible() {
        unsafe { ShowWindow(p.hwnd, SW_HIDE) };
    }
    p.image_cache = None;
    p.marks.clear();
}

/// 입력 커서(캐럿)의 화면 좌표. 많은 최신 프로그램(크롬 등)은 알려주지 않아 None 이 됩니다.
fn caret_point(target: HWND) -> Option<(POINT, i32)> {
    unsafe {
        let tid = GetWindowThreadProcessId(target, null_mut());
        let mut gi: GUITHREADINFO = std::mem::zeroed();
        gi.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(tid, &mut gi) == 0 || gi.hwndCaret.is_null() {
            return None;
        }
        let r = gi.rcCaret;
        if r.right == 0 && r.bottom == 0 && r.left == 0 && r.top == 0 {
            return None;
        }
        let mut pt = POINT { x: r.left, y: r.top };
        ClientToScreen(gi.hwndCaret, &mut pt);
        Some((pt, (r.bottom - r.top).max(1)))
    }
}

/// 현재 탭/검색어로 목록을 다시 만듭니다. 선택은 가능하면 같은 항목에 유지합니다.
pub fn refresh(a: &mut App) {
    let query = a.popup.search_text();
    let now = util::now_unix();
    let rows: Vec<Row> = match a.popup.tab {
        Tab::History | Tab::Pinned => {
            let view = if a.popup.tab == Tab::Pinned { View::Pinned } else { View::All };
            a.history
                .filter(&query, view)
                .into_iter()
                .take(MAX_ROWS)
                .map(|i| {
                    let app = i.source_app.as_deref().map(|s| {
                        let lower = s.to_ascii_lowercase();
                        if lower.ends_with(".exe") { s[..s.len() - 4].to_string() } else { s.to_string() }
                    });
                    let when = relative_time(now, i.last_used);
                    Row {
                        r: RowRef::Clip(i.id),
                        title: i.preview(300),
                        meta: match app {
                            Some(app) => format!("{} · {}", app, when),
                            None => when,
                        },
                        pinned: i.pinned,
                        special: !matches!(i.content, ClipContent::Text { .. }),
                    }
                })
                .collect()
        }
        Tab::Templates => {
            let q = Query::new(&query);
            a.config
                .templates
                .iter()
                .enumerate()
                .filter(|(_, t)| q.matches(&format!("{} {} {}", t.name, t.group.as_deref().unwrap_or(""), t.text)))
                .map(|(idx, t)| Row {
                    r: RowRef::Template(idx),
                    title: t.name.clone(),
                    meta: t.group.clone().unwrap_or_default(),
                    pinned: false,
                    special: false,
                })
                .collect()
        }
    };
    // 지워진 항목은 선택 목록에서도 뺍니다.
    let history = &a.history;
    a.popup.marks.retain(|id| history.get(*id).is_some());
    let p = &mut a.popup;
    let prev = p.selected();
    p.rows = rows;
    unsafe {
        SendMessageW(p.list, WM_SETREDRAW, 0, 0);
        SendMessageW(p.list, LB_SETCOUNT, p.rows.len(), 0);
        if !p.rows.is_empty() {
            let keep = prev.and_then(|r| p.rows.iter().position(|row| row.r == r)).unwrap_or(0);
            SendMessageW(p.list, LB_SETCURSEL, keep, 0);
        }
        SendMessageW(p.list, WM_SETREDRAW, 1, 0);
        // 비어 있으면 목록을 숨기고 팝업이 안내 문구를 그립니다.
        ShowWindow(p.list, if p.rows.is_empty() { SW_HIDE } else { SW_SHOWNA });
        InvalidateRect(p.list, null(), 1);
        InvalidateRect(p.hwnd, null(), 0);
        InvalidateRect(p.preview, null(), 1);
    }
}

pub fn on_history_changed(a: &mut App) {
    if a.popup.visible() {
        refresh(a);
    }
}

pub fn on_config_changed(a: &mut App) {
    apply_theme(a);
    let dpi = a.popup.dpi;
    set_dpi(a, dpi);
    if a.popup.visible() {
        layout(&mut a.popup);
        refresh(a);
    }
}

fn select(p: &Popup, index: isize) {
    if p.rows.is_empty() {
        return;
    }
    let i = index.clamp(0, p.rows.len() as isize - 1);
    unsafe {
        SendMessageW(p.list, LB_SETCURSEL, i as usize, 0);
        InvalidateRect(p.preview, null(), 1);
    }
}

fn move_selection(p: &Popup, delta: isize) {
    let cur = p.selected_index().map(|i| i as isize).unwrap_or(-1);
    select(p, cur + delta);
}

fn page_rows(p: &Popup) -> isize {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(p.list, &mut rc);
        let row_h = SendMessageW(p.list, LB_GETITEMHEIGHT, 0, 0).max(1);
        ((rc.bottom as isize) / row_h).max(1)
    }
}

fn switch_tab(a: &mut App, tab: Tab) {
    a.popup.tab = tab;
    a.popup.rows.clear();
    unsafe { SendMessageW(a.popup.list, LB_SETCURSEL, usize::MAX, 0) };
    refresh(a);
    select(&a.popup, 0);
}

fn redraw(p: &Popup) {
    unsafe {
        InvalidateRect(p.list, null(), 0);
        InvalidateRect(p.hwnd, null(), 0);
    }
}

/// 선택한 항목을 "여러 개 붙여넣기" 목록에 넣거나 뺍니다.
fn toggle_mark(p: &mut Popup) {
    if let Some(RowRef::Clip(id)) = p.selected() {
        match p.mark_order(id) {
            Some(i) => {
                p.marks.remove(i);
            }
            None => p.marks.push(id),
        }
        redraw(p);
    }
}

fn ensure_marked(p: &mut Popup) {
    if let Some(RowRef::Clip(id)) = p.selected() {
        if p.mark_order(id).is_none() {
            p.marks.push(id);
        }
    }
}

/// Shift+↑/↓: 지나가는 항목을 차례로 골라 담습니다.
fn extend_marks(p: &mut Popup, delta: isize) {
    ensure_marked(p);
    move_selection(p, delta);
    ensure_marked(p);
    redraw(p);
}

fn activate(a: &mut App, row: Option<RowRef>, action: Action) {
    // 여러 개를 골라 두었으면 한 번에 붙여넣기
    if !a.popup.marks.is_empty() {
        let ids = a.popup.marks.clone();
        a.use_many(&ids, action);
        return;
    }
    match row {
        Some(RowRef::Clip(id)) => a.use_clip(id, action),
        Some(RowRef::Template(i)) => a.use_template(i, action),
        None => {}
    }
}

fn delete_selected(a: &mut App) {
    if let Some(RowRef::Clip(id)) = a.popup.selected() {
        let idx = a.popup.selected_index().unwrap_or(0);
        a.delete_clip(id);
        select(&a.popup, idx as isize);
    }
}

fn toggle_preview(a: &mut App) {
    let p = &mut a.popup;
    p.show_preview = !p.show_preview;
    unsafe { ShowWindow(p.preview, if p.show_preview { SW_SHOWNA } else { SW_HIDE }) };
    layout(p);
    unsafe { InvalidateRect(p.hwnd, null(), 0) };
}

// ── 오른쪽 클릭 메뉴 ──────────────────────────────────────────

const M_PASTE: usize = 100;
const M_PLAIN: usize = 101;
const M_COPY: usize = 102;
const M_MARK: usize = 103;
const M_TRANSFORM: usize = 110;
const M_PIN: usize = 130;
const M_TEMPLATE: usize = 131;
const M_DELETE: usize = 132;
const M_EDIT_CONFIG: usize = 133;

fn context_menu(a: &mut App, screen: POINT) {
    let Some(row) = a.popup.selected() else { return };
    unsafe {
        let m = CreatePopupMenu();
        let add = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            let t = wide(text);
            AppendMenuW(menu, flags, id, t.as_ptr());
        };
        let is_image = match row {
            RowRef::Clip(id) => matches!(a.history.get(id).map(|i| &i.content), Some(ClipContent::Image { .. })),
            RowRef::Template(_) => false,
        };
        let n = a.popup.marks.len();
        if n > 0 {
            add(m, MF_STRING, M_PASTE, &format!("고른 {}개 한 번에 붙여넣기\tEnter", n));
        } else {
            add(m, MF_STRING, M_PASTE, "붙여넣기\tEnter");
        }
        if !is_image {
            if n == 0 {
                add(m, MF_STRING, M_PLAIN, "텍스트만 붙여넣기\tShift+Enter");
            }
            let sub = CreatePopupMenu();
            for (i, t) in Transform::ALL.iter().enumerate() {
                add(sub, MF_STRING, M_TRANSFORM + i, t.label());
            }
            let t = wide("바꿔서 붙여넣기");
            AppendMenuW(m, MF_POPUP, sub as usize, t.as_ptr());
        }
        add(m, MF_STRING, M_COPY, "클립보드에 복사만\tCtrl+C");
        add(m, MF_SEPARATOR, 0, "");
        match row {
            RowRef::Clip(id) => {
                let marked = a.popup.mark_order(id).is_some();
                add(
                    m,
                    MF_STRING,
                    M_MARK,
                    if marked { "여러 개 선택에서 빼기\tCtrl+Space" } else { "여러 개 선택에 넣기\tCtrl+Space" },
                );
                let pinned = a.history.get(id).map(|i| i.pinned).unwrap_or(false);
                add(m, MF_STRING, M_PIN, if pinned { "고정 해제\tCtrl+P" } else { "고정\tCtrl+P" });
                if !is_image {
                    add(m, MF_STRING, M_TEMPLATE, "템플릿으로 저장\tCtrl+T");
                }
                add(m, MF_SEPARATOR, 0, "");
                add(m, MF_STRING, M_DELETE, "삭제\tDel");
            }
            RowRef::Template(_) => add(m, MF_STRING, M_EDIT_CONFIG, "설정 파일에서 템플릿 편집\tCtrl+E"),
        }
        let cmd = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            screen.x,
            screen.y,
            0,
            a.popup.hwnd,
            null(),
        ) as usize;
        DestroyMenu(m);
        match cmd {
            M_PASTE => activate(a, Some(row), Action::Paste),
            M_PLAIN => activate(a, Some(row), Action::PastePlain),
            M_COPY => activate(a, Some(row), Action::CopyOnly),
            M_MARK => toggle_mark(&mut a.popup),
            c if (M_TRANSFORM..M_TRANSFORM + Transform::ALL.len()).contains(&c) => {
                activate(a, Some(row), Action::Transform(Transform::ALL[c - M_TRANSFORM]))
            }
            M_PIN => {
                if let RowRef::Clip(id) = row {
                    a.toggle_pin(id)
                }
            }
            M_TEMPLATE => {
                if let RowRef::Clip(id) = row {
                    a.save_as_template(id)
                }
            }
            M_DELETE => delete_selected(a),
            M_EDIT_CONFIG => {
                hide(a);
                a.open_config_file();
            }
            _ => {}
        }
    }
}

fn context_menu_at_selection(a: &mut App) {
    let Some(i) = a.popup.selected_index() else { return };
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        SendMessageW(a.popup.list, LB_GETITEMRECT, i, &mut rc as *mut RECT as LPARAM);
        let mut pt = POINT { x: rc.left + a.popup.scale(40), y: rc.bottom };
        ClientToScreen(a.popup.list, &mut pt);
        context_menu(a, pt);
    }
}

// ── 창 프로시저들 ─────────────────────────────────────────────

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

fn clear_search(edit: HWND) {
    let e = wide("");
    unsafe { SetWindowTextW(edit, e.as_ptr()) };
}

/// 검색창의 키 입력을 가로채서 목록 조작에 씁니다. 처리했으면 true.
fn handle_key(a: &mut App, edit: HWND, vk: VIRTUAL_KEY) -> bool {
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(VK_SHIFT);
    let paste = if shift { Action::PastePlain } else { Action::Paste };
    match vk {
        VK_UP if shift => extend_marks(&mut a.popup, -1),
        VK_DOWN if shift => extend_marks(&mut a.popup, 1),
        VK_UP => move_selection(&a.popup, -1),
        VK_DOWN => move_selection(&a.popup, 1),
        VK_PRIOR => move_selection(&a.popup, -page_rows(&a.popup)),
        VK_NEXT => move_selection(&a.popup, page_rows(&a.popup)),
        VK_HOME if ctrl => select(&a.popup, 0),
        VK_END if ctrl => select(&a.popup, isize::MAX),
        VK_SPACE if ctrl => {
            SWALLOW_CHAR.store(true, Ordering::Relaxed);
            toggle_mark(&mut a.popup);
            move_selection(&a.popup, 1);
        }
        VK_INSERT => {
            toggle_mark(&mut a.popup);
            move_selection(&a.popup, 1);
        }
        VK_RETURN => {
            let row = a.popup.selected();
            activate(a, row, paste);
        }
        VK_ESCAPE => {
            if !a.popup.marks.is_empty() {
                a.popup.marks.clear();
                redraw(&a.popup);
            } else if a.popup.search_text().is_empty() {
                hide(a);
            } else {
                clear_search(edit);
            }
        }
        VK_TAB => {
            let i = a.popup.tab.index() as isize + if shift { -1 } else { 1 };
            switch_tab(a, Tab::ALL[i.rem_euclid(3) as usize]);
        }
        VK_F2 => toggle_preview(a),
        VK_DELETE if shift || a.popup.search_text().is_empty() => delete_selected(a),
        VK_APPS => context_menu_at_selection(a),
        VK_F10 if shift => context_menu_at_selection(a),
        VK_BACK if ctrl => clear_search(edit),
        _ if ctrl => match vk as u8 {
            b'P' => {
                if let Some(RowRef::Clip(id)) = a.popup.selected() {
                    a.toggle_pin(id);
                }
            }
            b'T' => {
                if let Some(RowRef::Clip(id)) = a.popup.selected() {
                    a.save_as_template(id);
                }
            }
            b'E' => {
                hide(a);
                a.open_config_file();
            }
            b'A' => unsafe {
                SendMessageW(edit, EM_SETSEL, 0, -1);
            },
            b'C' => {
                let (mut s, mut e) = (0u32, 0u32);
                unsafe {
                    SendMessageW(edit, EM_GETSEL, &mut s as *mut u32 as WPARAM, &mut e as *mut u32 as LPARAM)
                };
                if s != e {
                    return false; // 검색어를 선택했으면 평범한 복사
                }
                let row = a.popup.selected();
                activate(a, row, Action::CopyOnly);
            }
            d @ b'0'..=b'9' => {
                // Ctrl+1 … Ctrl+9, Ctrl+0 = 10번째
                let idx = if d == b'0' { 9 } else { (d - b'1') as usize };
                let row = a.popup.rows.get(idx).map(|r| r.r);
                if let Some(row) = row {
                    a.popup.marks.clear();
                    activate(a, Some(row), paste);
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            if handle_key(app(), hwnd, wp as VIRTUAL_KEY) {
                return 0;
            }
        }
        WM_CHAR => {
            if SWALLOW_CHAR.swap(false, Ordering::Relaxed) {
                return 0;
            }
            // 위에서 처리한 키(Enter, Esc, Tab, Ctrl+문자)가 삑 소리나 이상한 글자를 내지 않도록.
            // Ctrl+V(0x16)/X(0x18)/Z(0x1A) 는 검색창 편집에 쓰므로 통과.
            let c = wp as u32;
            if (c < 0x20 && !matches!(c, 0x08 | 0x16 | 0x18 | 0x1A)) || c == 0x7F {
                return 0;
            }
        }
        _ => {}
    }
    let old: WNDPROC = std::mem::transmute(EDIT_PROC.load(Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

fn point_from_lparam(lp: LPARAM) -> POINT {
    POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    match msg {
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 == WA_INACTIVE {
                hide(a);
            }
            0
        }
        WM_COMMAND => {
            let id = wp & 0xFFFF;
            let code = ((wp >> 16) & 0xFFFF) as u32;
            match (id, code) {
                (ID_EDIT, EN_CHANGE) => {
                    refresh(a);
                    select(&a.popup, 0);
                }
                (ID_LIST, LBN_SELCHANGE) => {
                    // Ctrl+클릭 = 여러 개 선택에 넣기/빼기
                    if key_down(VK_CONTROL) {
                        toggle_mark(&mut a.popup);
                    }
                    InvalidateRect(a.popup.preview, null(), 1);
                    SetFocus(a.popup.edit);
                }
                (ID_LIST, LBN_DBLCLK) => {
                    let row = a.popup.selected();
                    activate(a, row, Action::Paste);
                }
                _ => {}
            }
            0
        }
        WM_CONTEXTMENU => {
            if wp as HWND == a.popup.list {
                if lp == -1 {
                    context_menu_at_selection(a);
                } else {
                    let pt = point_from_lparam(lp);
                    let mut cpt = pt;
                    ScreenToClient(a.popup.list, &mut cpt);
                    let packed = ((cpt.y as u32 & 0xFFFF) << 16 | (cpt.x as u32 & 0xFFFF)) as LPARAM;
                    let r = SendMessageW(a.popup.list, LB_ITEMFROMPOINT, 0, packed);
                    if (r >> 16) & 0xFFFF == 0 {
                        select(&a.popup, r & 0xFFFF);
                        context_menu(a, pt);
                    }
                    SetFocus(a.popup.edit);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let pt = point_from_lparam(lp);
            if let Some(i) = a.popup.tab_rects.iter().position(|r| PtInRect(r, pt) != 0) {
                switch_tab(a, Tab::ALL[i]);
            }
            SetFocus(a.popup.edit);
            0
        }
        WM_MEASUREITEM => {
            let mis = &mut *(lp as *mut MEASUREITEMSTRUCT);
            mis.itemHeight = row_height(&a.popup) as u32;
            1
        }
        WM_DRAWITEM => {
            draw_row(a, &*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_PAINT => {
            paint_chrome(a, hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_DPICHANGED => {
            set_dpi(a, (wp & 0xFFFF) as u32);
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            layout(&mut a.popup);
            0
        }
        WM_SIZE => {
            layout(&mut a.popup);
            0
        }
        WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            SetBkColor(dc, a.popup.pal.surface);
            SetTextColor(dc, a.popup.pal.text);
            a.popup.surface_brush as LRESULT
        }
        WM_CTLCOLORLISTBOX => a.popup.bg_brush as LRESULT,
        WM_CLOSE => {
            hide(a);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ── 그리기 ─────────────────────────────────────────────────────

unsafe fn fill(dc: HDC, rc: &RECT, color: COLORREF) {
    let b = CreateSolidBrush(color);
    FillRect(dc, rc, b);
    DeleteObject(b as HGDIOBJ);
}

/// 테두리 없는 둥근 사각형 채우기
unsafe fn fill_round(dc: HDC, rc: &RECT, radius: i32, color: COLORREF) {
    let b = CreateSolidBrush(color);
    let pen = CreatePen(PS_SOLID, 1, color);
    let ob = SelectObject(dc, b as HGDIOBJ);
    let op = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, rc.left, rc.top, rc.right, rc.bottom, radius, radius);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(b as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn draw_text(dc: HDC, s: &str, rc: &mut RECT, color: COLORREF, font: HFONT, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let old = SelectObject(dc, font as HGDIOBJ);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as _);
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, rc, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn text_width(dc: HDC, s: &str, font: HFONT) -> i32 {
    let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    draw_text(dc, s, &mut rc, 0, font, DT_CALCRECT | DT_SINGLELINE);
    rc.right
}

/// 검색창 상자, 탭, (비어 있을 때) 안내 문구, 아래쪽 한 줄 도움말을 그립니다.
unsafe fn paint_chrome(a: &mut App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let p = &mut a.popup;
    let pal = p.pal;
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    fill(dc, &rc, pal.bg);

    // 검색창 상자
    let sb = search_box(p, rc.right);
    fill_round(dc, &sb, p.scale(12), pal.surface);

    // 탭(오른쪽 위, 글자만)
    let mut x = sb.right + p.scale(6);
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let sel = *tab == p.tab;
        let font = if sel { p.font_bold } else { p.font };
        let w = text_width(dc, tab.name(), p.font_bold) + p.scale(14);
        let mut r = RECT { left: x, top: sb.top, right: x + w, bottom: sb.bottom };
        p.tab_rects[i] = r;
        draw_text(dc, tab.name(), &mut r, if sel { pal.accent } else { pal.muted }, font, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        if sel {
            let cx = (r.left + r.right) / 2;
            let dot = RECT { left: cx - p.scale(8), top: r.bottom - p.scale(2), right: cx + p.scale(8), bottom: r.bottom };
            fill_round(dc, &dot, p.scale(2), pal.accent);
        }
        x += w;
    }

    // 비어 있을 때 안내
    if p.rows.is_empty() {
        let msg = match p.tab {
            Tab::Pinned => "고정한 항목이 없어요\n기록에서 Ctrl+P 로 고정하세요",
            Tab::Templates => "템플릿이 없어요\nCtrl+E 로 설정 파일에서 추가하세요",
            Tab::History if !p.search_text().is_empty() => "검색 결과가 없어요",
            Tab::History => "아직 기록이 없어요\n아무 글이나 복사(Ctrl+C)해 보세요",
        };
        let mut r = RECT { left: 0, top: p.header_h + p.scale(40), right: rc.right, bottom: rc.bottom - p.footer_h };
        draw_text(dc, msg, &mut r, pal.muted, p.font, DT_CENTER | DT_WORDBREAK);
    }

    // 아래쪽 한 줄 도움말
    let n = p.marks.len();
    let (hint, color) = if n > 0 {
        (format!("{}개 선택됨  ·  Enter 한 번에 붙여넣기  ·  Esc 선택 해제", n), pal.accent)
    } else {
        let h = match p.tab {
            Tab::Templates => "Enter 붙여넣기  ·  Tab 전환  ·  Ctrl+E 편집",
            _ => "Enter 붙여넣기  ·  Ctrl+Space 여러 개  ·  Tab 전환  ·  F2 미리보기",
        };
        (h.to_string(), pal.muted)
    };
    let mut r = RECT {
        left: p.scale(14),
        top: rc.bottom - p.footer_h,
        right: rc.right - p.scale(14),
        bottom: rc.bottom - p.scale(2),
    };
    if a.paused {
        let mut pr = r;
        draw_text(dc, "● 일시정지", &mut pr, pal.warn, p.font_small, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
        r.right -= text_width(dc, "● 일시정지", p.font_small) + p.scale(8);
    }
    draw_text(dc, &hint, &mut r, color, p.font_small, DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    EndPaint(hwnd, &ps);
}

unsafe fn draw_row(a: &App, dis: &DRAWITEMSTRUCT) {
    let p = &a.popup;
    let pal = p.pal;
    let dc = dis.hDC;
    let rc = dis.rcItem;
    fill(dc, &rc, pal.bg);
    if dis.itemID == u32::MAX {
        return;
    }
    let Some(row) = p.rows.get(dis.itemID as usize) else { return };
    let selected = dis.itemState & ODS_SELECTED != 0;
    let mark = match row.r {
        RowRef::Clip(id) => p.mark_order(id),
        RowRef::Template(_) => None,
    };
    if selected {
        let r = RECT { left: rc.left + p.scale(4), top: rc.top + p.scale(1), right: rc.right - p.scale(4), bottom: rc.bottom - p.scale(1) };
        fill_round(dc, &r, p.scale(10), pal.sel);
    }
    let pad = p.scale(12);
    let mut x = rc.left + pad;

    // 왼쪽: 여러 개 선택 순서(동그라미) 또는 번호(Ctrl+1~9)
    let badge = p.scale(20);
    let cy = (rc.top + rc.bottom) / 2;
    if let Some(order) = mark {
        let br = RECT { left: x, top: cy - badge / 2, right: x + badge, bottom: cy + badge / 2 };
        fill_round(dc, &br, badge, pal.accent);
        let mut tr = br;
        draw_text(dc, &(order + 1).to_string(), &mut tr, pal.on_accent, p.font_small, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    } else if dis.itemID < 9 {
        let mut r = RECT { left: x, top: rc.top, right: x + badge, bottom: rc.bottom };
        draw_text(dc, &(dis.itemID + 1).to_string(), &mut r, pal.muted, p.font_small, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    x += badge + p.scale(10);

    // 오른쪽 정보(프로그램 · 시간)
    let meta_w = text_width(dc, &row.meta, p.font_small).min((rc.right - rc.left) / 3);
    let mut mr = RECT { left: rc.right - pad - meta_w, top: rc.top, right: rc.right - pad, bottom: rc.bottom };
    draw_text(dc, &row.meta, &mut mr, pal.muted, p.font_small, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);

    // 고정 표시
    if row.pinned {
        let w = text_width(dc, "★", p.font_small) + p.scale(6);
        let mut r = RECT { left: x, top: rc.top, right: x + w, bottom: rc.bottom };
        draw_text(dc, "★", &mut r, pal.pin, p.font_small, DT_VCENTER | DT_SINGLELINE);
        x += w;
    }

    // 본문
    let mut tr = RECT { left: x, top: rc.top, right: mr.left - p.scale(10), bottom: rc.bottom };
    let color = if row.special || mark.is_some() { pal.accent } else { pal.text };
    draw_text(dc, &row.title, &mut tr, color, p.font, DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
}

/// 미리보기 칸(F2): 텍스트, 파일 목록, 또는 이미지
unsafe extern "system" fn preview_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let a = app();
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            fill(dc, &rc, a.popup.pal.bg);
            fill_round(dc, &rc, a.popup.scale(12), a.popup.pal.surface);
            paint_preview(a, dc, rc);
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn paint_preview(a: &mut App, dc: HDC, rc: RECT) {
    let p = &a.popup;
    let pal = p.pal;
    let (font, small_h) = (p.font_small, p.small_h);
    let pad = p.scale(10);
    let gap = p.scale(4);
    let mut inner = RECT { left: rc.left + pad, top: rc.top + pad, right: rc.right - pad, bottom: rc.bottom - pad };
    let info_h = small_h + gap;
    let mut info_r = RECT { left: inner.left, top: inner.bottom - small_h, right: inner.right, bottom: inner.bottom };
    let text_flags = DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL | DT_END_ELLIPSIS;

    // 여러 개를 골랐으면 합쳐질 결과를 보여 줍니다.
    if p.marks.len() > 1 {
        let merged = a.history.merge_text(&p.marks, &a.config.merge_separator).unwrap_or_default();
        inner.bottom -= info_h;
        draw_text(dc, &merged.chars().take(4000).collect::<String>(), &mut inner, pal.text, font, text_flags);
        let info = format!("{}개를 복사한 순서대로 이어 붙입니다", p.marks.len());
        draw_text(dc, &info, &mut info_r, pal.accent, font, DT_SINGLELINE | DT_END_ELLIPSIS);
        return;
    }

    let Some(row) = p.selected() else { return };
    inner.bottom -= info_h;

    let info = match row {
        RowRef::Template(i) => {
            let Some(t) = a.config.templates.get(i) else { return };
            draw_text(dc, &t.text.replace("\r\n", "\n"), &mut inner, pal.text, font, text_flags);
            format!("템플릿 · {}", t.group.as_deref().unwrap_or("그룹 없음"))
        }
        RowRef::Clip(id) => {
            let Some(item) = a.history.get(id).cloned() else { return };
            let mut info = match &item.content {
                ClipContent::Text { text, html } => {
                    let shown: String = text.chars().take(4000).collect();
                    draw_text(dc, &shown.replace("\r\n", "\n"), &mut inner, pal.text, font, text_flags);
                    let mut s = format!("{}자 · {}줄", text.chars().count(), text.lines().count().max(1));
                    if html.is_some() {
                        s.push_str(" · 서식 포함");
                    }
                    s
                }
                ClipContent::Files { paths } => {
                    draw_text(dc, &paths.join("\n"), &mut inner, pal.text, font, text_flags);
                    format!("파일/폴더 {}개", paths.len())
                }
                ClipContent::Image { file, width, height } => {
                    paint_image(a, dc, id, file, inner);
                    format!("이미지 {}×{}", width, height)
                }
            };
            if let Some(app) = &item.source_app {
                info.push_str(&format!(" · {}", app));
            }
            if item.use_count > 0 {
                info.push_str(&format!(" · {}번 사용", item.use_count));
            }
            info
        }
    };
    draw_text(dc, &info, &mut info_r, pal.muted, font, DT_SINGLELINE | DT_END_ELLIPSIS);
}

unsafe fn paint_image(a: &mut App, dc: HDC, id: u64, file: &str, area: RECT) {
    let cached = matches!(&a.popup.image_cache, Some((cid, _)) if *cid == id);
    if !cached {
        let data = std::fs::read(storage::image_path(&a.data_dir, file)).unwrap_or_default();
        a.popup.image_cache = Some((id, data));
    }
    let Some((_, data)) = &a.popup.image_cache else { return };
    let Some(dib) = bmp::bmp_to_dib(data) else { return };
    let (Some((w, h)), Some(off)) = (bmp::dib_size(dib), bmp::pixel_offset(dib)) else { return };
    if w == 0 || h == 0 || off as usize >= dib.len() {
        return;
    }
    let aw = (area.right - area.left).max(1) as f64;
    let ah = (area.bottom - area.top).max(1) as f64;
    let s = (aw / w as f64).min(ah / h as f64).min(1.0);
    let dw = (w as f64 * s).max(1.0) as i32;
    let dh = (h as f64 * s).max(1.0) as i32;
    let x = area.left;
    let y = area.top + ((ah as i32 - dh) / 2).max(0);
    SetStretchBltMode(dc, HALFTONE);
    StretchDIBits(
        dc,
        x,
        y,
        dw,
        dh,
        0,
        0,
        w as i32,
        h as i32,
        dib.as_ptr().add(off as usize) as *const _,
        dib.as_ptr() as *const BITMAPINFO,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
}
