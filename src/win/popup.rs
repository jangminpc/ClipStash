//! 단축키(Ctrl+C 두 번 등)로 뜨는 기록 팝업 창.
//!
//! 단순하고 반투명한 디자인(검색창 없음):
//! ┌──────────────────────────────────────┐
//! │  기록  고정  템플릿                        │  ← 탭(Tab 키)
//! │  1  회의록 정리 …           메모장 · 방금 │  ← 숫자 키 1~9, 0 으로 바로 붙여넣기
//! │  2  [▧▧] 이미지 1920×1080   캡처 · 1분 전 │  ← 이미지는 작은 썸네일
//! │ ❷  https://…              chrome · 3분 전 │  ← ❶❷ = 여러 개 고른 순서 표시
//! │ ❶  안녕하세요               카톡 · 5분 전 │
//! │  2개 선택됨 · Enter 한 번에 붙여넣기      │  ← 한 줄 도움말
//! └──────────────────────────────────────┘

use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};

use clipstash::config::{app_display_name, PopupPosition, Theme};
use clipstash::history::{relative_time, ClipContent, View};
use clipstash::transform::Transform;
use clipstash::{bmp, storage, template};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::Ime::ImmAssociateContext;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::{self, wide};
use super::{app, Action, App};

const POPUP_CLASS: &str = "ClipStash.Popup";
const PREVIEW_CLASS: &str = "ClipStash.Preview";
const VIEWER_CLASS: &str = "ClipStash.Viewer";
const ID_LIST: usize = 11;
const ID_EDIT: usize = 12;

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
    /// 목록에 그릴 작은 이미지(항목 id → 비트맵, 가로, 세로)
    thumbs: HashMap<u64, (HBITMAP, i32, i32)>,
    /// 붙여넣기 전 편집 칸(F2)
    edit: HWND,
    /// 편집 중인 항목
    editing: Option<RowRef>,
    /// 이미지 크게 보기 창
    viewer: HWND,
    viewer_id: Option<u64>,
    /// 이 프로그램에서 복사한 것만 보기(None = 모든 프로그램)
    pub app_filter: Option<String>,
    app_chip: RECT,
    /// 열릴 때 서서히 나타나기(남은 단계)
    fade_step: u32,
}

impl Default for Popup {
    fn default() -> Self {
        Popup {
            hwnd: null_mut(),
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
            thumbs: HashMap::new(),
            edit: null_mut(),
            editing: None,
            viewer: null_mut(),
            viewer_id: None,
            app_filter: None,
            app_chip: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            fade_step: 0,
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

    /// 줄 왼쪽 배치: (체크박스 왼쪽, 체크박스 오른쪽, 번호 왼쪽, 내용 왼쪽)
    fn lead_x(&self) -> (i32, i32, i32, i32) {
        let check_l = self.scale(10);
        let check_r = check_l + self.scale(18);
        let num_l = check_r + self.scale(4);
        let content_l = num_l + self.scale(16) + self.scale(8);
        (check_l, check_r, num_l, content_l)
    }

    fn selected_index(&self) -> Option<usize> {
        let i = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        (i >= 0 && (i as usize) < self.rows.len()).then_some(i as usize)
    }

    fn selected(&self) -> Option<RowRef> {
        self.selected_index().map(|i| self.rows[i].r)
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
        let cls3 = wide(VIEWER_CLASS);
        let wc3 = WNDCLASSEXW { lpfnWndProc: Some(viewer_proc), lpszClassName: cls3.as_ptr(), ..wc };
        RegisterClassExW(&wc3);
    }
}

static LIST_PROC: AtomicIsize = AtomicIsize::new(0);
static EDIT_PROC: AtomicIsize = AtomicIsize::new(0);

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
        let lb_cls = wide("LISTBOX");
        let list = CreateWindowExW(
            0,
            lb_cls.as_ptr(),
            null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | (LBS_NOTIFY | LBS_OWNERDRAWVARIABLE | LBS_NOINTEGRALHEIGHT) as u32,
            0,
            0,
            10,
            10,
            hwnd,
            ID_LIST as HMENU,
            a.hinst,
            null(),
        );
        // 키 입력은 목록이 직접 받습니다(검색창 없음).
        let old = SetWindowLongPtrW(list, GWLP_WNDPROC, list_proc as *const () as isize);
        LIST_PROC.store(old, Ordering::Relaxed);
        // 한글 입력기가 켜져 있어도 Q·V·K 같은 글자 키가 그대로 오도록 목록에서는 입력기를 끕니다.
        ImmAssociateContext(list, null_mut());
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
        let ed_cls = wide("EDIT");
        let edit = CreateWindowExW(
            0,
            ed_cls.as_ptr(),
            null(),
            WS_CHILD | WS_VSCROLL | (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | ES_NOHIDESEL) as u32,
            0,
            0,
            10,
            10,
            hwnd,
            ID_EDIT as HMENU,
            a.hinst,
            null(),
        );
        let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        EDIT_PROC.store(old, Ordering::Relaxed);
        let p = &mut a.popup;
        p.hwnd = hwnd;
        p.list = list;
        p.preview = preview;
        p.edit = edit;
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
        SetWindowTheme(p.edit, theme.as_ptr(), null());
        ShowWindow(p.preview, if p.show_preview && p.editing.is_none() { SW_SHOWNA } else { SW_HIDE });
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
    p.font_h + p.scale(11)
}

/// 이미지 항목 줄 높이(썸네일이 들어갈 만큼)
fn image_row_height(p: &Popup) -> i32 {
    row_height(p).max(THUMB_H * p.dpi as i32 / 96 + p.scale(10))
}

const THUMB_W: i32 = 64;
const THUMB_H: i32 = 44;

/// DPI(또는 글꼴 설정)가 바뀌면 글꼴을 다시 만듭니다.
fn set_dpi(a: &mut App, dpi: u32) {
    let p = &mut a.popup;
    if p.dpi != dpi {
        clear_thumbs(p);
    }
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
    p.header_h = p.font_h + p.scale(20);
    p.footer_h = p.small_h + p.scale(14);
    unsafe {
        SendMessageW(p.edit, WM_SETFONT, p.font as WPARAM, 0);
        let m = p.scale(8) as usize;
        SendMessageW(p.edit, EM_SETMARGINS, (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM, (m | m << 16) as LPARAM);
    }
}

fn layout(p: &mut Popup) {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(p.hwnd, &mut rc);
        let (w, h) = (rc.right, rc.bottom);
        let list_y = p.header_h;
        let avail = h - list_y - p.footer_h;
        let preview_h = if p.show_preview { (avail * 32 / 100).max(p.font_h * 4) } else { 0 };
        let list_h = avail - preview_h;
        if p.editing.is_some() {
            let pad = p.scale(10);
            MoveWindow(p.edit, pad, list_y + p.scale(2), w - 2 * pad, avail - p.scale(6), 1);
            return;
        }
        MoveWindow(p.list, p.scale(6), list_y, w - p.scale(12), list_h, 1);
        if p.show_preview {
            let pad = p.scale(10);
            MoveWindow(p.preview, pad, list_y + list_h + p.scale(4), w - 2 * pad, preview_h - p.scale(8), 1);
        }
    }
}

/// 팝업을 엽니다. `target` 은 붙여넣을 창(없으면 클립보드에 복사만).
const TIMER_FADE: usize = 0x5A01;
const TIMER_SYNC_LATER: usize = 0x5A02;
const FADE_STEPS: u32 = 4;

fn target_alpha(a: &App) -> u8 {
    (a.config.popup_opacity as u32 * 255 / 100) as u8
}

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
        let wa = mi.rcWork;
        // 화면보다 크면 화면에 맞춥니다(가로 95%, 세로 90%).
        let w = p.scale(a.config.popup_width as i32).min((wa.right - wa.left) * 95 / 100);
        let h = p.scale(a.config.popup_height as i32).min((wa.bottom - wa.top) * 9 / 10);
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
        p.app_filter = None;
        end_edit(p);
        p.image_cache = None;
        p.show_preview = a.config.show_preview;
        ShowWindow(p.preview, if p.show_preview { SW_SHOWNA } else { SW_HIDE });
        layout(p);
        p.rows.clear();
        refresh(a);
        select(&a.popup, 0);
        let alpha = target_alpha(a);
        let p = &mut a.popup;
        // 지난번 화면이 잠깐 비치지 않도록, 투명한 상태로 띄워 새 내용을 다 그린 뒤 서서히 보여 줍니다.
        SetLayeredWindowAttributes(p.hwnd, 0, 0, LWA_ALPHA);
        ShowWindow(p.hwnd, SW_SHOW);
        SetForegroundWindow(p.hwnd);
        SetFocus(p.list);
        RedrawWindow(p.hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
        p.fade_step = 1;
        SetLayeredWindowAttributes(p.hwnd, 0, (alpha as u32 / FADE_STEPS) as u8, LWA_ALPHA);
        SetTimer(p.hwnd, TIMER_FADE, 15, None);
        // 다른 PC 기록 합치기는 팝업이 뜬 다음에(폴더가 느려도 여는 게 늦어지지 않게)
        SetTimer(p.hwnd, TIMER_SYNC_LATER, 120, None);
    }
}

unsafe fn on_popup_timer(a: &mut App, hwnd: HWND, id: usize) {
    match id {
        TIMER_FADE => {
            let alpha = target_alpha(a) as u32;
            let p = &mut a.popup;
            p.fade_step += 1;
            if p.fade_step >= FADE_STEPS || !p.visible() {
                KillTimer(hwnd, TIMER_FADE);
                p.fade_step = 0;
                SetLayeredWindowAttributes(hwnd, 0, alpha as u8, LWA_ALPHA);
            } else {
                SetLayeredWindowAttributes(hwnd, 0, (alpha * p.fade_step / FADE_STEPS) as u8, LWA_ALPHA);
            }
        }
        TIMER_SYNC_LATER => {
            KillTimer(hwnd, TIMER_SYNC_LATER);
            if a.popup.visible() && a.popup.editing.is_none() {
                a.sync_pull();
            }
        }
        _ => {}
    }
}

pub fn hide(a: &mut App) {
    let alpha = target_alpha(a);
    let p = &mut a.popup;
    close_viewer(p, false);
    if p.visible() {
        unsafe {
            KillTimer(p.hwnd, TIMER_FADE);
            KillTimer(p.hwnd, TIMER_SYNC_LATER);
            ShowWindow(p.hwnd, SW_HIDE);
            SetLayeredWindowAttributes(p.hwnd, 0, alpha, LWA_ALPHA);
        }
        p.fade_step = 0;
    }
    end_edit(p);
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
    let now = util::now_unix();
    let rows: Vec<Row> = match a.popup.tab {
        Tab::History | Tab::Pinned => {
            let view = if a.popup.tab == Tab::Pinned { View::Pinned } else { View::All };
            let filter = a.popup.app_filter.clone();
            a.history
                .filter("", view)
                .into_iter()
                .filter(|i| match (&filter, &i.source_app) {
                    (None, _) => true,
                    (Some(f), Some(s)) => f.eq_ignore_ascii_case(s),
                    (Some(_), None) => false,
                })
                .take(MAX_ROWS)
                .map(|i| {
                    let app = i.source_app.as_deref().map(|s| app_display_name(s).to_string());
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
            a.config
                .templates
                .iter()
                .enumerate()
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
    // 지워진 이미지의 썸네일도 정리
    let gone: Vec<u64> = a.popup.thumbs.keys().copied().filter(|id| history.get(*id).is_none()).collect();
    for id in gone {
        if let Some((bmp, _, _)) = a.popup.thumbs.remove(&id) {
            unsafe { DeleteObject(bmp as HGDIOBJ) };
        }
    }
    let p = &mut a.popup;
    let prev = p.selected();
    p.rows = rows;
    unsafe {
        SendMessageW(p.list, WM_SETREDRAW, 0, 0);
        SendMessageW(p.list, LB_RESETCONTENT, 0, 0);
        SendMessageW(p.list, LB_INITSTORAGE, p.rows.len(), 0);
        for i in 0..p.rows.len() {
            // 그리기는 직접 하므로 항목 데이터로 번호만 넣습니다(WM_MEASUREITEM 이 줄 높이를 정함)
            SendMessageW(p.list, LB_ADDSTRING, 0, i as LPARAM);
        }
        if !p.rows.is_empty() {
            let keep = prev.and_then(|r| p.rows.iter().position(|row| row.r == r)).unwrap_or(0);
            SendMessageW(p.list, LB_SETCURSEL, keep, 0);
        }
        SendMessageW(p.list, WM_SETREDRAW, 1, 0);
        // 비어 있으면 목록을 숨기고 팝업이 안내 문구를 그립니다.
        if p.editing.is_none() {
            ShowWindow(p.list, if p.rows.is_empty() { SW_HIDE } else { SW_SHOWNA });
        }
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

fn switch_tab(a: &mut App, tab: Tab) {
    a.popup.tab = tab;
    a.popup.marks.clear();
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

// ── 붙여넣기 전 편집(F2) ──────────────────────────────────────

fn start_edit(a: &mut App) {
    let Some(row) = a.popup.selected() else { return };
    let (text, caret) = match row {
        RowRef::Clip(id) => match a.history.get(id).and_then(|i| i.content.as_plain_text()) {
            Some(t) => (t, None),
            None => return,
        },
        RowRef::Template(i) => {
            let Some(t) = a.config.templates.get(i).cloned() else { return };
            let clipboard = if t.text.contains("{clipboard}") { super::clip::read_text(a.main) } else { None };
            let ctx = template::Context { now: util::local_time(), clipboard: clipboard.as_deref() };
            let ex = template::expand(&t.text, &ctx);
            let caret = ex.cursor_back.map(|n| ex.text.chars().count().saturating_sub(n));
            (ex.text, caret)
        }
    };
    let p = &mut a.popup;
    p.marks.clear();
    p.editing = Some(row);
    let crlf = super::clip::crlf(&text);
    let w = wide(&crlf);
    unsafe {
        SetWindowTextW(p.edit, w.as_ptr());
        ShowWindow(p.list, SW_HIDE);
        ShowWindow(p.preview, SW_HIDE);
        ShowWindow(p.edit, SW_SHOW);
        layout(p);
        SetFocus(p.edit);
        // 커서 위치: 템플릿의 {cursor} 자리, 아니면 맨 끝. (UTF-16 + \r\n 기준으로 셉니다)
        let pos = match caret {
            Some(chars) => super::clip::crlf(&text.chars().take(chars).collect::<String>()).encode_utf16().count(),
            None => w.len() - 1,
        };
        SendMessageW(p.edit, EM_SETSEL, pos, pos as LPARAM);
        SendMessageW(p.edit, EM_SCROLLCARET, 0, 0);
        InvalidateRect(p.hwnd, null(), 0);
    }
}

fn end_edit(p: &mut Popup) {
    if p.editing.take().is_none() {
        return;
    }
    unsafe {
        ShowWindow(p.edit, SW_HIDE);
        ShowWindow(p.list, if p.rows.is_empty() { SW_HIDE } else { SW_SHOWNA });
        ShowWindow(p.preview, if p.show_preview { SW_SHOWNA } else { SW_HIDE });
        layout(p);
        SetFocus(p.list);
        InvalidateRect(p.hwnd, null(), 0);
    }
}

fn finish_edit(a: &mut App) {
    let text = unsafe {
        let len = GetWindowTextLengthW(a.popup.edit).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(a.popup.edit, buf.as_mut_ptr(), buf.len() as i32);
        util::from_wide(&buf).replace("\r\n", "\n")
    };
    end_edit(&mut a.popup);
    a.paste_edited(text);
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ctrl = key_down(VK_CONTROL);
    match msg {
        WM_KEYDOWN => match wp as VIRTUAL_KEY {
            VK_RETURN if ctrl => {
                finish_edit(app());
                return 0;
            }
            VK_ESCAPE => {
                end_edit(&mut app().popup);
                return 0;
            }
            k if ctrl && k == b'A' as u16 => {
                SendMessageW(hwnd, EM_SETSEL, 0, -1);
                return 0;
            }
            _ => {}
        },
        // Ctrl+Enter(\n), Ctrl+A, Esc 가 글자로 들어가지 않게
        WM_CHAR if (ctrl && (wp == 0x0A || wp == 0x01)) || wp == 0x1B => return 0,
        WM_GETDLGCODE => return DLGC_WANTALLKEYS as LRESULT,
        _ => {}
    }
    let old: WNDPROC = std::mem::transmute(EDIT_PROC.load(Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

// ── 이미지 크게 보기 ──────────────────────────────────────────

fn selected_image(a: &App) -> Option<u64> {
    match a.popup.selected()? {
        RowRef::Clip(id) if matches!(a.history.get(id)?.content, ClipContent::Image { .. }) => Some(id),
        _ => None,
    }
}

fn open_viewer(a: &mut App, id: u64) {
    let Some(ClipContent::Image { width, height, .. }) = a.history.get(id).map(|i| i.content.clone()) else { return };
    let p = &mut a.popup;
    unsafe {
        if p.viewer.is_null() {
            let cls = wide(VIEWER_CLASS);
            p.viewer = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                cls.as_ptr(),
                null(),
                WS_POPUP,
                0,
                0,
                10,
                10,
                p.hwnd,
                null_mut(),
                a.hinst,
                null(),
            );
            let round = DWMWCP_ROUND;
            DwmSetWindowAttribute(p.viewer, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const u32 as *const _, 4);
        }
        let mon = MonitorFromWindow(p.hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let pad = p.scale(10);
        let max_w = ((wa.right - wa.left) * 9 / 10 - 2 * pad).max(50) as f64;
        let max_h = ((wa.bottom - wa.top) * 9 / 10 - 2 * pad).max(50) as f64;
        let s = (max_w / width.max(1) as f64).min(max_h / height.max(1) as f64).min(p.dpi as f64 / 96.0);
        let w = ((width as f64 * s) as i32).max(40) + 2 * pad;
        let h = ((height as f64 * s) as i32).max(40) + 2 * pad;
        let x = wa.left + (wa.right - wa.left - w) / 2;
        let y = wa.top + (wa.bottom - wa.top - h) / 2;
        p.viewer_id = Some(id);
        let dark: u32 = (p.pal.bg == DARK.bg) as u32;
        DwmSetWindowAttribute(p.viewer, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const u32 as *const _, 4);
        SetWindowPos(p.viewer, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
        InvalidateRect(p.viewer, null(), 0);
        SetForegroundWindow(p.viewer);
    }
}

fn close_viewer(p: &mut Popup, refocus: bool) {
    if p.viewer.is_null() || unsafe { IsWindowVisible(p.viewer) } == 0 {
        return;
    }
    p.viewer_id = None;
    unsafe {
        ShowWindow(p.viewer, SW_HIDE);
        if refocus && p.visible() {
            SetForegroundWindow(p.hwnd);
            SetFocus(p.list);
        }
    }
}

unsafe extern "system" fn viewer_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            fill(dc, &rc, a.popup.pal.bg);
            if let Some(id) = a.popup.viewer_id {
                if let Some(ClipContent::Image { file, .. }) = a.history.get(id).map(|i| i.content.clone()) {
                    let pad = a.popup.scale(10);
                    let area = RECT { left: rc.left + pad, top: rc.top + pad, right: rc.right - pad, bottom: rc.bottom - pad };
                    paint_image(a, dc, id, &file, area, 100.0);
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONUP | WM_RBUTTONUP | WM_KEYDOWN | WM_SYSKEYDOWN => {
            close_viewer(&mut a.popup, true);
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 == WA_INACTIVE {
                if lp as HWND == a.popup.hwnd {
                    close_viewer(&mut a.popup, false);
                } else {
                    hide(a);
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ── 프로그램별 보기 ───────────────────────────────────────────

const M_APP_BASE: usize = 300;

/// 기록에 나오는 프로그램 목록을 메뉴로 보여 주고, 고른 프로그램 것만 보이게 합니다.
fn app_menu(a: &mut App, pt: POINT) {
    let mut apps: Vec<(String, usize)> = Vec::new();
    for i in &a.history.items {
        if let Some(s) = &i.source_app {
            match apps.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(s)) {
                Some(e) => e.1 += 1,
                None => apps.push((s.clone(), 1)),
            }
        }
    }
    apps.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.to_lowercase().cmp(&y.0.to_lowercase())));
    apps.truncate(30);
    unsafe {
        let m = CreatePopupMenu();
        let add = |flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
            let t = wide(text);
            AppendMenuW(m, flags, id, t.as_ptr());
        };
        let cur = a.popup.app_filter.clone();
        add(MF_STRING | if cur.is_none() { MF_CHECKED } else { MF_UNCHECKED }, M_APP_BASE, "모든 프로그램");
        if !apps.is_empty() {
            add(MF_SEPARATOR, 0, "");
        }
        for (i, (name, n)) in apps.iter().enumerate() {
            let on = cur.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(name));
            add(
                MF_STRING | if on { MF_CHECKED } else { MF_UNCHECKED },
                M_APP_BASE + 1 + i,
                &format!("{}\t{}", app_display_name(name), n),
            );
        }
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, a.popup.hwnd, null()) as usize;
        DestroyMenu(m);
        if cmd == M_APP_BASE {
            set_app_filter(a, None);
        } else if cmd > M_APP_BASE && cmd <= M_APP_BASE + apps.len() {
            set_app_filter(a, Some(apps[cmd - M_APP_BASE - 1].0.clone()));
        }
    }
}

fn set_app_filter(a: &mut App, f: Option<String>) {
    a.popup.app_filter = f;
    a.popup.marks.clear();
    if a.popup.tab == Tab::Templates {
        a.popup.tab = Tab::History;
    }
    a.popup.rows.clear();
    refresh(a);
    select(&a.popup, 0);
}

fn app_menu_at_chip(a: &mut App) {
    let r = a.popup.app_chip;
    let mut pt = POINT { x: r.left, y: r.bottom };
    unsafe { ClientToScreen(a.popup.hwnd, &mut pt) };
    app_menu(a, pt);
}

/// 고른 항목들(없으면 지금 줄)의 기록 id
fn chosen_ids(p: &Popup) -> Vec<u64> {
    if !p.marks.is_empty() {
        return p.marks.clone();
    }
    match p.selected() {
        Some(RowRef::Clip(id)) => vec![id],
        _ => Vec::new(),
    }
}

fn queue_key(a: &mut App) {
    if a.popup.marks.is_empty() {
        a.toggle_queue();
    } else {
        let ids = a.popup.marks.clone();
        a.start_queue_with(&ids);
    }
}

fn kakao_key(a: &mut App) {
    let ids = chosen_ids(&a.popup);
    if !ids.is_empty() {
        a.send_to_kakao(&ids);
    }
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
const M_EDIT: usize = 140;
const M_VIEW: usize = 141;
const M_OCR: usize = 142;
const M_OCR_WIN: usize = 147;
const M_KAKAO: usize = 143;
const M_QUEUE: usize = 144;
const M_APP_ONLY: usize = 145;
const M_APP_ALL: usize = 146;

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
        if n > 0 {
            add(m, MF_STRING, M_QUEUE, &format!("고른 {}개 순차 붙여넣기 (Ctrl+V 마다 하나씩)\tQ", n));
        }
        if is_image {
            add(m, MF_SEPARATOR, 0, "");
            add(m, MF_STRING, M_VIEW, "크게 보기\tV");
            let ai = !a.config.anthropic_api_key.trim().is_empty() || std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty());
            add(m, MF_STRING, M_OCR, if ai { "AI 로 글자 추출\tO" } else { "글자 추출 (OCR)\tO" });
            if ai {
                add(m, MF_STRING, M_OCR_WIN, "Windows OCR 로 글자 추출(오프라인)\tShift+O");
            }
        } else if n == 0 {
            add(m, MF_STRING, M_EDIT, "고쳐서 붙여넣기\tF2");
        }
        if !is_image && matches!(row, RowRef::Clip(_)) {
            add(m, MF_STRING, M_KAKAO, "카카오톡 나에게 보내기\tK");
        }
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
                if let Some(app) = a.history.get(id).and_then(|i| i.source_app.clone()) {
                    if a.popup.app_filter.is_none() {
                        add(m, MF_STRING, M_APP_ONLY, &format!("{} 에서 복사한 것만 보기\tA", app_display_name(&app)));
                    }
                }
                if a.popup.app_filter.is_some() {
                    add(m, MF_STRING, M_APP_ALL, "모든 프로그램 보기");
                }
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
            M_EDIT => start_edit(a),
            M_VIEW => {
                if let RowRef::Clip(id) = row {
                    open_viewer(a, id)
                }
            }
            M_OCR => {
                if let RowRef::Clip(id) = row {
                    a.ocr_item(id, false)
                }
            }
            M_OCR_WIN => {
                if let RowRef::Clip(id) = row {
                    a.ocr_item(id, true)
                }
            }
            M_KAKAO => kakao_key(a),
            M_QUEUE => queue_key(a),
            M_APP_ONLY => {
                if let RowRef::Clip(id) = row {
                    let app = a.history.get(id).and_then(|i| i.source_app.clone());
                    set_app_filter(a, app);
                }
            }
            M_APP_ALL => set_app_filter(a, None),
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

/// T: 지금 줄을 바꿔서(대문자, 한 줄로, 공백 제거 등) 붙여넣기 메뉴
fn transform_menu_at_selection(a: &mut App) {
    let Some(i) = a.popup.selected_index() else { return };
    let Some(row) = a.popup.selected() else { return };
    if let RowRef::Clip(id) = row {
        if matches!(a.history.get(id).map(|i| &i.content), Some(ClipContent::Image { .. })) {
            return;
        }
    }
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        SendMessageW(a.popup.list, LB_GETITEMRECT, i, &mut rc as *mut RECT as LPARAM);
        let mut pt = POINT { x: rc.left + a.popup.scale(40), y: rc.bottom };
        ClientToScreen(a.popup.list, &mut pt);
        let m = CreatePopupMenu();
        for (k, t) in Transform::ALL.iter().enumerate() {
            let label = wide(&format!("&{}  {}", k + 1, t.label()));
            AppendMenuW(m, MF_STRING, M_TRANSFORM + k, label.as_ptr());
        }
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, 0, a.popup.hwnd, null()) as usize;
        DestroyMenu(m);
        if (M_TRANSFORM..M_TRANSFORM + Transform::ALL.len()).contains(&cmd) {
            activate(a, Some(row), Action::Transform(Transform::ALL[cmd - M_TRANSFORM]));
        }
    }
}

// ── 창 프로시저들 ─────────────────────────────────────────────

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// 목록의 키 입력. 처리했으면 true(처리하지 않은 ↑↓ PgUp/PgDn Home/End 는 목록이 알아서 움직임).
fn handle_key(a: &mut App, vk: VIRTUAL_KEY) -> bool {
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(VK_SHIFT);
    let paste = if shift { Action::PastePlain } else { Action::Paste };
    match vk {
        VK_UP if shift => extend_marks(&mut a.popup, -1),
        // Shift+O: 인터넷 없이 Windows 내장 OCR 로
        k if shift && !ctrl && k == b'O' as u16 => {
            if let Some(id) = selected_image(a) {
                a.ocr_item(id, true);
            }
        }
        VK_DOWN if shift => extend_marks(&mut a.popup, 1),
        VK_SPACE | VK_INSERT => {
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
            } else {
                hide(a);
            }
        }
        VK_TAB => {
            let i = a.popup.tab.index() as isize + if shift { -1 } else { 1 };
            switch_tab(a, Tab::ALL[i.rem_euclid(3) as usize]);
        }
        VK_F2 => start_edit(a),
        VK_F3 => toggle_preview(a),
        VK_F4 => app_menu_at_chip(a),
        VK_DELETE => delete_selected(a),
        VK_APPS => context_menu_at_selection(a),
        VK_F10 if shift => context_menu_at_selection(a),
        // 숫자 키 1~9, 0(=10번째): 바로 붙여넣기 (Ctrl 과 함께 눌러도 됨)
        k @ (0x30..=0x39 | VK_NUMPAD0..=VK_NUMPAD9) => {
            let d = if k >= VK_NUMPAD0 { k - VK_NUMPAD0 } else { k - 0x30 } as usize;
            let idx = if d == 0 { 9 } else { d - 1 };
            if let Some(row) = a.popup.rows.get(idx).map(|r| r.r) {
                a.popup.marks.clear();
                activate(a, Some(row), paste);
            }
        }
        _ if !ctrl && !shift && !key_down(VK_MENU) => match vk as u8 {
            b'E' => start_edit(a),
            b'V' => {
                if let Some(id) = selected_image(a) {
                    open_viewer(a, id);
                }
            }
            b'O' => {
                if let Some(id) = selected_image(a) {
                    a.ocr_item(id, false);
                }
            }
            b'K' => kakao_key(a),
            b'T' => transform_menu_at_selection(a),
            b'Q' => queue_key(a),
            b'A' => app_menu_at_chip(a),
            _ => return false,
        },
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
            b'E' => super::settings::open(a),
            b'C' => {
                let row = a.popup.selected();
                activate(a, row, Action::CopyOnly);
            }
            _ => return false,
        },
        _ => return false,
    }
    true
}

unsafe extern "system" fn list_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            if handle_key(app(), wp as VIRTUAL_KEY) {
                return 0;
            }
        }
        // 체크박스를 누르면 체크/해제(체크한 순서대로 번호가 붙음), 나머지 곳은 한 번 클릭 = 붙여넣기
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if !key_down(VK_CONTROL) && !key_down(VK_SHIFT) => {
            let a = app();
            let r = SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp);
            if (r >> 16) & 0xFFFF == 0 {
                let idx = (r & 0xFFFF) as usize;
                let x = (lp & 0xFFFF) as i16 as i32;
                let (_, check_r, _, _) = a.popup.lead_x();
                if let Some(RowRef::Clip(_)) = a.popup.rows.get(idx).map(|row| row.r) {
                    if x < check_r + a.popup.scale(3) {
                        select(&a.popup, idx as isize);
                        SetFocus(hwnd);
                        toggle_mark(&mut a.popup);
                        return 0;
                    }
                }
                // 그 밖의 곳을 한 번 누르면 바로 붙여넣기.
                // 이미 체크한 항목이 있으면 붙여넣지 않고 체크에 넣거나 뺍니다(Enter로 한 번에).
                if let Some(row) = a.popup.rows.get(idx).map(|row| row.r) {
                    select(&a.popup, idx as isize);
                    SetFocus(hwnd);
                    if !a.popup.marks.is_empty() {
                        if let RowRef::Clip(_) = row {
                            toggle_mark(&mut a.popup);
                        }
                        return 0;
                    }
                    if msg == WM_LBUTTONDOWN {
                        activate(a, Some(row), Action::Paste);
                    }
                    return 0;
                }
            }
        }
        // 배경은 마지막 줄 아래 빈 곳만 칠합니다(줄은 직접 그리므로 지웠다 그리는 깜빡임 방지)
        WM_ERASEBKGND => {
            let a = app();
            let dc = wp as HDC;
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let n = SendMessageW(hwnd, LB_GETCOUNT, 0, 0);
            if n > 0 {
                let mut last: RECT = std::mem::zeroed();
                if SendMessageW(hwnd, LB_GETITEMRECT, (n - 1) as usize, &mut last as *mut RECT as LPARAM) != LB_ERR as isize {
                    rc.top = rc.top.max(last.bottom);
                }
            }
            if rc.bottom > rc.top {
                fill(dc, &rc, a.popup.pal.bg);
            }
            return 1;
        }
        // 글자 입력은 쓰지 않습니다(삑 소리 방지)
        WM_CHAR => return 0,
        // Tab/Enter/Esc 등 모든 키를 목록이 직접 받도록
        WM_GETDLGCODE => return DLGC_WANTALLKEYS as LRESULT,
        _ => {}
    }
    let old: WNDPROC = std::mem::transmute(LIST_PROC.load(Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

fn point_from_lparam(lp: LPARAM) -> POINT {
    POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    match msg {
        WM_ACTIVATE => {
            // 크게 보기 창으로 넘어간 것은 닫지 않음
            if (wp & 0xFFFF) as u32 == WA_INACTIVE && (lp as HWND != a.popup.viewer || a.popup.viewer.is_null()) {
                hide(a);
            }
            0
        }
        WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            SetTextColor(dc, a.popup.pal.text);
            SetBkColor(dc, a.popup.pal.surface);
            a.popup.surface_brush as LRESULT
        }
        WM_COMMAND => {
            let id = wp & 0xFFFF;
            let code = ((wp >> 16) & 0xFFFF) as u32;
            match (id, code) {
                (ID_LIST, LBN_SELCHANGE) => {
                    // Ctrl+클릭 = 여러 개 선택에 넣기/빼기
                    if key_down(VK_CONTROL) && key_down(VK_LBUTTON) {
                        toggle_mark(&mut a.popup);
                    }
                    InvalidateRect(a.popup.preview, null(), 1);
                    InvalidateRect(a.popup.hwnd, null(), 0);
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
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let pt = point_from_lparam(lp);
            if a.popup.editing.is_some() {
                SetFocus(a.popup.edit);
                return 0;
            }
            if let Some(i) = a.popup.tab_rects.iter().position(|r| PtInRect(r, pt) != 0) {
                switch_tab(a, Tab::ALL[i]);
            } else if a.popup.tab != Tab::Templates && PtInRect(&a.popup.app_chip, pt) != 0 {
                app_menu_at_chip(a);
            }
            SetFocus(a.popup.list);
            0
        }
        WM_MEASUREITEM => {
            let mis = &mut *(lp as *mut MEASUREITEMSTRUCT);
            let p = &a.popup;
            let is_image = match p.rows.get(mis.itemID as usize).map(|r| r.r) {
                Some(RowRef::Clip(id)) => matches!(a.history.get(id).map(|i| &i.content), Some(ClipContent::Image { .. })),
                _ => false,
            };
            mis.itemHeight = if is_image { image_row_height(p) } else { row_height(p) } as u32;
            1
        }
        WM_DRAWITEM => {
            let dis = *(lp as *const DRAWITEMSTRUCT);
            buffered(dis.hDC, &dis.rcItem, |m| {
                let mut d = dis;
                d.hDC = m;
                draw_row(a, &d);
            });
            1
        }
        WM_PAINT => {
            paint_chrome(a, hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_TIMER => {
            on_popup_timer(a, hwnd, wp);
            0
        }
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

/// 탭, (비어 있을 때) 안내 문구, 아래쪽 한 줄 도움말을 그립니다.
unsafe fn paint_chrome(a: &mut App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    buffered(dc, &rc, |m| paint_chrome_into(a, hwnd, m));
    EndPaint(hwnd, &ps);
}

/// 화면 밖 비트맵에 먼저 그린 뒤 한 번에 옮깁니다(깜빡임 없음).
/// `f` 안에서는 원래 DC 와 같은 좌표를 그대로 씁니다.
unsafe fn buffered(dc: HDC, rc: &RECT, f: impl FnOnce(HDC)) {
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    if w <= 0 || h <= 0 {
        return f(dc);
    }
    let mem = CreateCompatibleDC(dc);
    let bmp = CreateCompatibleBitmap(dc, w, h);
    if mem.is_null() || bmp.is_null() {
        if !bmp.is_null() {
            DeleteObject(bmp as HGDIOBJ);
        }
        if !mem.is_null() {
            DeleteDC(mem);
        }
        return f(dc);
    }
    let old = SelectObject(mem, bmp as HGDIOBJ);
    SetViewportOrgEx(mem, -rc.left, -rc.top, null_mut());
    f(mem);
    SetViewportOrgEx(mem, 0, 0, null_mut());
    BitBlt(dc, rc.left, rc.top, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old);
    DeleteObject(bmp as HGDIOBJ);
    DeleteDC(mem);
}

unsafe fn paint_chrome_into(a: &mut App, hwnd: HWND, dc: HDC) {
    let p = &mut a.popup;
    let pal = p.pal;
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);
    fill(dc, &rc, pal.bg);

    // 탭(위쪽, 글자만)
    let sb = RECT { left: 0, top: p.scale(4), right: 0, bottom: p.header_h - p.scale(4) };
    let mut x = p.scale(12);
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

    // 오른쪽 위: 프로그램 고르기(A)
    if p.tab != Tab::Templates {
        let label = match &p.app_filter {
            Some(f) => format!("{}  ▾", app_display_name(f)),
            None => "모든 프로그램  ▾".to_string(),
        };
        let w = text_width(dc, &label, p.font_small).min(rc.right / 2) + p.scale(18);
        let r = RECT { left: rc.right - p.scale(10) - w, top: sb.top + p.scale(3), right: rc.right - p.scale(10), bottom: sb.bottom - p.scale(3) };
        p.app_chip = r;
        if p.app_filter.is_some() {
            fill_round(dc, &r, p.scale(12), pal.sel);
        }
        let mut tr = r;
        let color = if p.app_filter.is_some() { pal.accent } else { pal.muted };
        draw_text(dc, &label, &mut tr, color, p.font_small, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    } else {
        p.app_chip = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    }

    // 비어 있을 때 안내
    if p.rows.is_empty() && p.editing.is_none() {
        let msg = match p.tab {
            _ if p.app_filter.is_some() && p.tab != Tab::Templates => "이 프로그램에서 복사한 기록이 없어요\nA 로 다른 프로그램을 고르세요",
            Tab::Pinned => "고정한 항목이 없어요\n기록에서 Ctrl+P 로 고정하세요",
            Tab::Templates => "템플릿이 없어요\nCtrl+E 로 설정 파일에서 추가하세요",
            Tab::History => "아직 기록이 없어요\n아무 글이나 복사(Ctrl+C)해 보세요",
        };
        let mut r = RECT { left: 0, top: p.header_h + p.scale(40), right: rc.right, bottom: rc.bottom - p.footer_h };
        draw_text(dc, msg, &mut r, pal.muted, p.font, DT_CENTER | DT_WORDBREAK);
    }

    // 아래쪽 한 줄 도움말
    let n = p.marks.len();
    let selected_is_image = matches!(
        p.selected().and_then(|r| match r { RowRef::Clip(id) => a.history.get(id), _ => None }).map(|i| &i.content),
        Some(ClipContent::Image { .. })
    );
    let queue_on = a.queue_on || !a.queue.is_empty();
    let (hint, color) = if p.editing.is_some() {
        ("고치는 중  ·  Ctrl+Enter 붙여넣기  ·  Esc 취소".to_string(), pal.accent)
    } else if n > 0 {
        (format!("{}개 선택됨  ·  Enter 한 번에  ·  Q 순차 붙여넣기  ·  Esc 해제", n), pal.accent)
    } else if queue_on && p.tab != Tab::Templates {
        (format!("순차 붙여넣기 켜짐 · {}개 남음 · Ctrl+V 마다 하나씩 · Q 끄기", a.queue.len()), pal.accent)
    } else {
        let h = match p.tab {
            Tab::Templates => "1~9 · Enter 붙여넣기  ·  F2 고쳐서 붙여넣기  ·  Ctrl+E 편집",
            _ if selected_is_image => "Enter 붙여넣기  ·  V 크게 보기  ·  O 글자 추출  ·  우클릭 메뉴",
            _ => "1~9 붙여넣기  ·  체크(Space) 순서대로 합치기  ·  T 바꿔서",
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
}

fn clear_thumbs(p: &mut Popup) {
    for (_, (bmp, _, _)) in p.thumbs.drain() {
        unsafe { DeleteObject(bmp as HGDIOBJ) };
    }
}

/// DIB 를 작은 비트맵으로 줄여 만듭니다.
unsafe fn make_thumb(dib: &[u8], max_w: i32, max_h: i32) -> Option<(HBITMAP, i32, i32)> {
    let (w, h) = bmp::dib_size(dib)?;
    let off = bmp::pixel_offset(dib)? as usize;
    if w == 0 || h == 0 || off >= dib.len() {
        return None;
    }
    let s = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
    let tw = ((w as f64 * s) as i32).max(1);
    let th = ((h as f64 * s) as i32).max(1);
    let screen = GetDC(null_mut());
    let mem = CreateCompatibleDC(screen);
    let bmp = CreateCompatibleBitmap(screen, tw, th);
    let old = SelectObject(mem, bmp as HGDIOBJ);
    SetStretchBltMode(mem, HALFTONE);
    SetBrushOrgEx(mem, 0, 0, null_mut());
    StretchDIBits(
        mem,
        0,
        0,
        tw,
        th,
        0,
        0,
        w as i32,
        h as i32,
        dib.as_ptr().add(off) as *const _,
        dib.as_ptr() as *const BITMAPINFO,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
    SelectObject(mem, old);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);
    Some((bmp, tw, th))
}

/// 새로 복사한 이미지의 썸네일을 미리 만들어 둡니다(팝업을 열 때 파일을 읽지 않도록).
pub fn cache_thumbnail(a: &mut App, id: u64, dib: &[u8]) {
    let p = &mut a.popup;
    if p.thumbs.contains_key(&id) || p.dpi == 0 {
        return;
    }
    let (mw, mh) = (THUMB_W * p.dpi as i32 / 96, THUMB_H * p.dpi as i32 / 96);
    if let Some(t) = unsafe { make_thumb(dib, mw, mh) } {
        p.thumbs.insert(id, t);
    }
}

/// 이미지 항목의 썸네일(없으면 파일에서 읽어 만들고 기억해 둠). 이미지가 아니면 None.
unsafe fn thumbnail(a: &mut App, id: u64) -> Option<(HBITMAP, i32, i32)> {
    let ClipContent::Image { file, .. } = &a.history.get(id)?.content else { return None };
    if let Some(t) = a.popup.thumbs.get(&id) {
        return Some(*t);
    }
    let data = std::fs::read(storage::image_path(&a.data_dir, file)).ok()?;
    let dib = bmp::bmp_to_dib(&data)?;
    let dpi = a.popup.dpi as i32;
    let t = make_thumb(dib, THUMB_W * dpi / 96, THUMB_H * dpi / 96)?;
    a.popup.thumbs.insert(id, t);
    Some(t)
}

unsafe fn draw_row(a: &mut App, dis: &DRAWITEMSTRUCT) {
    let thumb = match a.popup.rows.get(dis.itemID as usize).map(|r| r.r) {
        Some(RowRef::Clip(id)) => thumbnail(a, id),
        _ => None,
    };
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
    let (check_l, check_r, num_l, content_l) = p.lead_x();
    let cy = (rc.top + rc.bottom) / 2;

    // 맨 왼쪽: 체크박스(체크하면 체크한 순서 번호)
    if let RowRef::Clip(_) = row.r {
        let half = (check_r - check_l) / 2;
        let br = RECT { left: rc.left + check_l, top: cy - half, right: rc.left + check_r, bottom: cy + half };
        let radius = p.scale(6);
        if let Some(order) = mark {
            fill_round(dc, &br, radius, pal.accent);
            let mut tr = br;
            draw_text(dc, &(order + 1).to_string(), &mut tr, pal.on_accent, p.font_small, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            fill_round(dc, &br, radius, pal.muted);
            let t = p.scale(2).max(1);
            let ir = RECT { left: br.left + t, top: br.top + t, right: br.right - t, bottom: br.bottom - t };
            fill_round(dc, &ir, (radius - t).max(1), if selected { pal.sel } else { pal.bg });
        }
    }
    // 그 옆: 번호(1~9 바로 붙여넣기)
    if dis.itemID < 9 {
        let mut r = RECT { left: rc.left + num_l, top: rc.top, right: rc.left + num_l + p.scale(16), bottom: rc.bottom };
        draw_text(dc, &(dis.itemID + 1).to_string(), &mut r, pal.muted, p.font_small, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let mut x = rc.left + content_l;
    let pad = p.scale(12);

    // 이미지면 작은 썸네일
    {
        if let Some((bmp, tw, th)) = thumb {
            let box_w = THUMB_W * p.dpi as i32 / 96;
            let ty = cy - th / 2;
            let mem = CreateCompatibleDC(dc);
            let old = SelectObject(mem, bmp as HGDIOBJ);
            BitBlt(dc, x, ty, tw, th, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            DeleteDC(mem);
            let frame = RECT { left: x - 1, top: ty - 1, right: x + tw + 1, bottom: ty + th + 1 };
            let b = CreateSolidBrush(pal.line);
            FrameRect(dc, &frame, b);
            DeleteObject(b as HGDIOBJ);
            x += box_w + p.scale(10);
        }
    }

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
            buffered(dc, &rc, |m| {
                fill(m, &rc, a.popup.pal.bg);
                fill_round(m, &rc, a.popup.scale(12), a.popup.pal.surface);
                paint_preview(a, m, rc);
            });
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
        let info = format!("{}개를 체크한 순서대로 이어 붙입니다", p.marks.len());
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
                    paint_image(a, dc, id, file, inner, 1.0);
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

unsafe fn paint_image(a: &mut App, dc: HDC, id: u64, file: &str, area: RECT, max_scale: f64) {
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
    let s = (aw / w as f64).min(ah / h as f64).min(max_scale);
    let dw = (w as f64 * s).max(1.0) as i32;
    let dh = (h as f64 * s).max(1.0) as i32;
    let x = if max_scale > 1.0 { area.left + ((aw as i32 - dw) / 2).max(0) } else { area.left };
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
