//! 클립보드 매니저 창: 왼쪽 분류·즐겨찾기·태그, 가운데 검색·탭·목록, 오른쪽 미리보기·상세 정보,
//! 아래 도구 막대. 빠른 붙여넣기용 Alt+C 팝업과는 따로 동작합니다.

use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use clipstash::config::app_display_name;
use clipstash::history::{format_datetime, relative_time, Category, ClipContent, ClipItem};
use clipstash::search::Query;
use clipstash::{bmp, storage};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::{self, wide};
use super::{app, clip, paste, App};

const CLASS: &str = "ClipStashManager";
const PROMPT_CLASS: &str = "ClipStashPrompt";
const ID_LIST: i32 = 10;
const ID_SEARCH: i32 = 11;
/// 저장 공간 막대의 기준(표시용)
const STORAGE_CAP_MB: f64 = 1024.0;

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

const BG: COLORREF = rgb(11, 17, 32);
const PANEL: COLORREF = rgb(17, 25, 44);
const PANEL2: COLORREF = rgb(24, 35, 60);
const SEL: COLORREF = rgb(29, 52, 104);
const LINE: COLORREF = rgb(36, 48, 76);
const TEXT: COLORREF = rgb(230, 235, 245);
const MUTED: COLORREF = rgb(139, 151, 176);
const ACCENT: COLORREF = rgb(47, 107, 255);
const ACCENT_TEXT: COLORREF = rgb(120, 165, 255);
const DANGER: COLORREF = rgb(248, 113, 113);
const STAR: COLORREF = rgb(250, 190, 40);
const TAG_COLORS: [COLORREF; 8] = [
    rgb(52, 211, 153),
    rgb(190, 210, 60),
    rgb(167, 139, 250),
    rgb(244, 114, 182),
    rgb(96, 165, 250),
    rgb(251, 146, 60),
    rgb(45, 212, 191),
    rgb(250, 204, 21),
];

// 아이콘 글꼴(Segoe Fluent Icons / Segoe MDL2 Assets)의 글자
const IC_ALL: char = '\u{E8A9}';
const IC_IMAGE: char = '\u{E91B}';
const IC_TEXT: char = '\u{E8D2}';
const IC_FOLDER: char = '\u{E8B7}';
const IC_FILE: char = '\u{E8A5}';
const IC_LINK: char = '\u{E71B}';
const IC_HTML: char = '\u{E774}';
const IC_CODE: char = '\u{E943}';
const IC_STAR: char = '\u{E734}';
const IC_STAR_FILL: char = '\u{E735}';
const IC_TAG: char = '\u{E8EC}';
const IC_ADD: char = '\u{E710}';
const IC_SETTINGS: char = '\u{E713}';
const IC_SEARCH: char = '\u{E721}';
const IC_FILTER: char = '\u{E71C}';
const IC_LIST: char = '\u{E8FD}';
const IC_MORE: char = '\u{E712}';
const IC_COPY: char = '\u{E8C8}';
const IC_PASTE: char = '\u{E77F}';
const IC_DELETE: char = '\u{E74D}';
const IC_PIN: char = '\u{E718}';
const IC_HISTORY: char = '\u{E81C}';
const IC_TEMPLATE: char = '\u{E8F1}';
const IC_OPEN: char = '\u{E8A7}';
const IC_SAVE: char = '\u{E74E}';
const IC_INFO: char = '\u{E946}';
const IC_CLOCK: char = '\u{E823}';
const IC_SIZE: char = '\u{E740}';
const IC_APP: char = '\u{E71D}';
const IC_CHECK: char = '\u{E73E}';
const IC_DOWN: char = '\u{E70D}';
const IC_STORAGE: char = '\u{EDA2}';

#[derive(Clone, PartialEq, Debug, Default)]
pub enum Side {
    #[default]
    All,
    Cat(Category),
    Favorites,
    Tag(String),
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Tab {
    #[default]
    History,
    Pinned,
    Templates,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum RowRef {
    Clip(u64),
    Template(usize),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tool {
    Copy,
    Paste,
    Favorite,
    Tag,
    Delete,
    More,
}

#[derive(Clone, PartialEq, Debug)]
enum Hit {
    Side(Side),
    AddTag,
    Settings,
    Tab(Tab),
    AppFilter,
    Tool(Tool),
    HeaderFilter,
    HeaderSettings,
    HeaderMore,
    PvOpen,
    PvCopy,
    PvSave,
    DetailAddTag,
    DetailTag(String),
}

pub struct Manager {
    pub hwnd: HWND,
    list: HWND,
    search: HWND,
    dpi: i32,
    font: HFONT,
    font_bold: HFONT,
    font_small: HFONT,
    font_title: HFONT,
    icons: HFONT,
    icons_small: HFONT,
    side: Side,
    tab: Tab,
    app_filter: Option<String>,
    query: String,
    rows: Vec<RowRef>,
    /// 체크한 항목(체크한 순서)
    checks: Vec<u64>,
    hits: Vec<(RECT, Hit)>,
    thumbs: HashMap<u64, (HBITMAP, i32, i32)>,
    /// 미리보기용 이미지(항목 id, DIB)
    preview_dib: Option<(u64, Vec<u8>)>,
    /// 붙여넣을 창(매니저를 열기 전에 쓰던 창)
    target: HWND,
    storage_bytes: u64,
    panel_brush: HBRUSH,
}

impl Default for Manager {
    fn default() -> Self {
        Manager {
            hwnd: null_mut(),
            list: null_mut(),
            search: null_mut(),
            dpi: 96,
            font: null_mut(),
            font_bold: null_mut(),
            font_small: null_mut(),
            font_title: null_mut(),
            icons: null_mut(),
            icons_small: null_mut(),
            side: Side::All,
            tab: Tab::History,
            app_filter: None,
            query: String::new(),
            rows: Vec::new(),
            checks: Vec::new(),
            hits: Vec::new(),
            thumbs: HashMap::new(),
            preview_dib: None,
            target: null_mut(),
            storage_bytes: 0,
            panel_brush: null_mut(),
        }
    }
}

impl Manager {
    fn s(&self, v: i32) -> i32 {
        v * self.dpi / 96
    }

    pub fn visible(&self) -> bool {
        !self.hwnd.is_null() && unsafe { IsWindowVisible(self.hwnd) } != 0
    }

    fn selected_index(&self) -> Option<usize> {
        let r = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        (r >= 0 && (r as usize) < self.rows.len()).then_some(r as usize)
    }

    fn current(&self) -> Option<RowRef> {
        self.selected_index().map(|i| self.rows[i])
    }

    /// 도구 막대가 다룰 항목: 체크한 것들, 없으면 지금 줄
    fn targets(&self) -> Vec<u64> {
        if !self.checks.is_empty() {
            return self.checks.clone();
        }
        match self.current() {
            Some(RowRef::Clip(id)) => vec![id],
            _ => Vec::new(),
        }
    }
}

static LIST_PROC: AtomicIsize = AtomicIsize::new(0);
static SEARCH_PROC: AtomicIsize = AtomicIsize::new(0);

pub fn register(hinst: HINSTANCE) {
    unsafe {
        let cls = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(proc_),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            hIcon: LoadIconW(hinst, 1 as _),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let pc = wide(PROMPT_CLASS);
        let wc2 = WNDCLASSEXW {
            lpfnWndProc: Some(prompt_proc),
            lpszClassName: pc.as_ptr(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            style: 0,
            ..wc
        };
        RegisterClassExW(&wc2);
    }
}

/// 매니저 창을 열거나(이미 열려 있으면 앞으로) 숨깁니다.
pub fn toggle(a: &mut App) {
    if a.manager.visible() && unsafe { GetForegroundWindow() } == a.manager.hwnd {
        unsafe { ShowWindow(a.manager.hwnd, SW_HIDE) };
    } else {
        open(a);
    }
}

pub fn open(a: &mut App) {
    super::popup::hide(a);
    unsafe {
        let fg = GetForegroundWindow();
        if !util::is_own_window(fg) {
            a.manager.target = fg;
        }
        if a.manager.hwnd.is_null() {
            create(a);
        }
        let m = &mut a.manager;
        if m.hwnd.is_null() {
            return;
        }
        m.storage_bytes = storage_size(&a.data_dir);
        refresh(a);
        let m = &a.manager;
        if IsIconic(m.hwnd) != 0 {
            ShowWindow(m.hwnd, SW_RESTORE);
        } else {
            ShowWindow(m.hwnd, SW_SHOW);
        }
        SetForegroundWindow(m.hwnd);
        SetFocus(m.list);
    }
}

unsafe fn create(a: &mut App) {
    let cls = wide(CLASS);
    let title = wide("클립보드 매니저");
    let sys_dpi = windows_sys::Win32::UI::HiDpi::GetDpiForSystem().max(96) as i32;
    let (w, h) = (1140 * sys_dpi / 96, 760 * sys_dpi / 96);
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let (w, h) = (w.min(sw * 95 / 100), h.min(sh * 90 / 100));
    let hwnd = CreateWindowExW(
        0,
        cls.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        (sw - w) / 2,
        (sh - h) / 2,
        w,
        h,
        null_mut(),
        null_mut(),
        a.hinst,
        null(),
    );
    if hwnd.is_null() {
        return;
    }
    let m = &mut a.manager;
    m.hwnd = hwnd;
    m.dpi = (GetDpiForWindow(hwnd) as i32).max(96);
    // 어두운 제목 표시줄
    let dark: u32 = 1;
    DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as *const _, 4);
    let cap = BG;
    DwmSetWindowAttribute(hwnd, 35, &cap as *const u32 as *const _, 4);

    let lb = wide("LISTBOX");
    m.list = CreateWindowExW(
        0,
        lb.as_ptr(),
        null(),
        WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | WS_TABSTOP
            | (LBS_OWNERDRAWFIXED | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_WANTKEYBOARDINPUT) as u32,
        0,
        0,
        10,
        10,
        hwnd,
        ID_LIST as isize as HMENU,
        a.hinst,
        null(),
    );
    let old = SetWindowLongPtrW(m.list, GWLP_WNDPROC, list_proc as *const () as isize);
    LIST_PROC.store(old, Ordering::Relaxed);
    let ed = wide("EDIT");
    m.search = CreateWindowExW(
        0,
        ed.as_ptr(),
        null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        0,
        0,
        10,
        10,
        hwnd,
        ID_SEARCH as isize as HMENU,
        a.hinst,
        null(),
    );
    let old = SetWindowLongPtrW(m.search, GWLP_WNDPROC, search_proc as *const () as isize);
    SEARCH_PROC.store(old, Ordering::Relaxed);
    let cue = wide("클립보드 내역 검색 (키워드, 내용, 앱, 파일명, 태그 …)");
    SendMessageW(m.search, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
    let theme = wide("DarkMode_Explorer");
    SetWindowTheme(m.list, theme.as_ptr(), null());
    m.panel_brush = CreateSolidBrush(PANEL);
    make_fonts(m);
    layout(m);
}

unsafe fn make_fonts(m: &mut Manager) {
    for f in [m.font, m.font_bold, m.font_small, m.font_title, m.icons, m.icons_small] {
        if !f.is_null() {
            DeleteObject(f as HGDIOBJ);
        }
    }
    let mk = |pt: i32, weight: i32, face: &str| {
        let f = wide(face);
        CreateFontW(
            -(pt * m.dpi / 72),
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            f.as_ptr(),
        )
    };
    m.font = mk(10, FW_NORMAL as i32, "맑은 고딕");
    m.font_bold = mk(10, FW_SEMIBOLD as i32, "맑은 고딕");
    m.font_small = mk(9, FW_NORMAL as i32, "맑은 고딕");
    m.font_title = mk(13, FW_BOLD as i32, "맑은 고딕");
    let face = if font_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    m.icons = mk(11, FW_NORMAL as i32, face);
    m.icons_small = mk(9, FW_NORMAL as i32, face);
    SendMessageW(m.search, WM_SETFONT, m.font as usize, 0);
    SendMessageW(m.list, LB_SETITEMHEIGHT, 0, m.s(31) as LPARAM);
}

unsafe fn font_exists(face: &str) -> bool {
    let f = wide(face);
    let font = CreateFontW(12, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, 0, 0, f.as_ptr());
    let dc = GetDC(null_mut());
    let old = SelectObject(dc, font as HGDIOBJ);
    let mut buf = [0u16; 64];
    let n = GetTextFaceW(dc, buf.len() as i32, buf.as_mut_ptr());
    SelectObject(dc, old);
    ReleaseDC(null_mut(), dc);
    DeleteObject(font as HGDIOBJ);
    n > 0 && util::from_wide(&buf).eq_ignore_ascii_case(face)
}

// ── 배치 ───────────────────────────────────────────────────────

struct Geo {
    side_w: i32,
    top_h: i32,
    center: RECT,
    list_panel: RECT,
    right: RECT,
    bottom: RECT,
    search_box: RECT,
}

fn geo(m: &Manager) -> Geo {
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(m.hwnd, &mut rc) };
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    let side_w = s(212);
    let top_h = s(60);
    let pad = s(14);
    let right_w = s(310).min((rc.right - side_w) * 3 / 10);
    let bottom = RECT { left: side_w + pad, top: rc.bottom - s(62), right: rc.right - pad, bottom: rc.bottom - pad };
    let right = RECT { left: rc.right - pad - right_w, top: top_h, right: rc.right - pad, bottom: bottom.top - s(10) };
    let center = RECT { left: side_w + pad, top: top_h, right: right.left - s(12), bottom: bottom.top - s(10) };
    let list_panel = RECT { left: center.left, top: center.top + s(46), right: center.right, bottom: center.bottom };
    let search_box = RECT { left: side_w + pad, top: s(12), right: rc.right - pad - s(150), bottom: s(48) };
    Geo { side_w, top_h, center, list_panel, right, bottom, search_box }
}

unsafe fn layout(m: &mut Manager) {
    if m.hwnd.is_null() {
        return;
    }
    let g = geo(m);
    let lp = g.list_panel;
    MoveWindow(m.list, lp.left + m.s(4), lp.top + m.s(6), lp.right - lp.left - m.s(8), lp.bottom - lp.top - m.s(12), 1);
    let sb = g.search_box;
    let eh = m.s(22);
    MoveWindow(m.search, sb.left + m.s(40), (sb.top + sb.bottom - eh) / 2, sb.right - sb.left - m.s(120), eh, 1);
    InvalidateRect(m.hwnd, null(), 0);
}

// ── 목록 만들기 ─────────────────────────────────────────────────

pub fn on_history_changed(a: &mut App) {
    if a.manager.visible() {
        a.manager.storage_bytes = storage_size(&a.data_dir);
        refresh(a);
    }
}

fn storage_size(dir: &std::path::Path) -> u64 {
    let mut total = std::fs::metadata(dir.join(storage::HISTORY_FILE)).map(|m| m.len()).unwrap_or(0);
    if let Ok(rd) = std::fs::read_dir(dir.join(storage::IMAGE_DIR)) {
        total += rd.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum::<u64>();
    }
    total
}

fn matches_side(item: &ClipItem, side: &Side) -> bool {
    match side {
        Side::All => true,
        Side::Cat(c) => item.category() == *c,
        Side::Favorites => item.favorite,
        Side::Tag(t) => item.tags.iter().any(|x| x == t),
    }
}

pub fn refresh(a: &mut App) {
    let m = &mut a.manager;
    if m.hwnd.is_null() {
        return;
    }
    let history = &a.history;
    m.checks.retain(|id| history.get(*id).is_some());
    let gone: Vec<u64> = m.thumbs.keys().copied().filter(|id| history.get(*id).is_none()).collect();
    for id in gone {
        if let Some((b, _, _)) = m.thumbs.remove(&id) {
            unsafe { DeleteObject(b as HGDIOBJ) };
        }
    }
    let q = Query::new(&m.query);
    let rows: Vec<RowRef> = match m.tab {
        Tab::Templates => a
            .config
            .templates
            .iter()
            .enumerate()
            .filter(|(_, t)| q.is_empty() || q.matches(&format!("{} {}", t.name, t.text)))
            .map(|(i, _)| RowRef::Template(i))
            .collect(),
        tab => history
            .items
            .iter()
            .filter(|i| tab != Tab::Pinned || i.pinned)
            .filter(|i| matches_side(i, &m.side))
            .filter(|i| match (&m.app_filter, &i.source_app) {
                (None, _) => true,
                (Some(f), Some(s)) => f.eq_ignore_ascii_case(s),
                (Some(_), None) => false,
            })
            .filter(|i| q.is_empty() || q.matches(&i.search_text()))
            .map(|i| RowRef::Clip(i.id))
            .collect(),
    };
    let prev = m.current();
    m.rows = rows;
    unsafe {
        SendMessageW(m.list, WM_SETREDRAW, 0, 0);
        SendMessageW(m.list, LB_RESETCONTENT, 0, 0);
        SendMessageW(m.list, LB_INITSTORAGE, m.rows.len(), 0);
        for i in 0..m.rows.len() {
            SendMessageW(m.list, LB_ADDSTRING, 0, i as LPARAM);
        }
        if !m.rows.is_empty() {
            let keep = prev.and_then(|r| m.rows.iter().position(|x| *x == r)).unwrap_or(0);
            SendMessageW(m.list, LB_SETCURSEL, keep, 0);
        }
        SendMessageW(m.list, WM_SETREDRAW, 1, 0);
        InvalidateRect(m.list, null(), 1);
        InvalidateRect(m.hwnd, null(), 0);
    }
}

// ── 그리기 도우미 ───────────────────────────────────────────────

unsafe fn fill(dc: HDC, rc: &RECT, color: COLORREF) {
    let b = CreateSolidBrush(color);
    FillRect(dc, rc, b);
    DeleteObject(b as HGDIOBJ);
}

unsafe fn round(dc: HDC, rc: &RECT, radius: i32, fill_c: COLORREF, border: Option<COLORREF>) {
    let b = CreateSolidBrush(fill_c);
    let pen = CreatePen(PS_SOLID, 1, border.unwrap_or(fill_c));
    let ob = SelectObject(dc, b as HGDIOBJ);
    let op = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, rc.left, rc.top, rc.right, rc.bottom, radius, radius);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(b as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
}

unsafe fn text(dc: HDC, s: &str, rc: &RECT, color: COLORREF, font: HFONT, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = *rc;
    let old = SelectObject(dc, font as HGDIOBJ);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as _);
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn line_text(dc: HDC, s: &str, rc: &RECT, color: COLORREF, font: HFONT) {
    text(dc, s, rc, color, font, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
}

unsafe fn icon(dc: HDC, m: &Manager, c: char, rc: &RECT, color: COLORREF) {
    text(dc, &c.to_string(), rc, color, m.icons, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
}

unsafe fn text_w(dc: HDC, s: &str, font: HFONT) -> i32 {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let old = SelectObject(dc, font as HGDIOBJ);
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right
}

unsafe fn buffered(dc: HDC, rc: &RECT, f: impl FnOnce(HDC)) {
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    if w <= 0 || h <= 0 {
        return;
    }
    let mem = CreateCompatibleDC(dc);
    let bmp = CreateCompatibleBitmap(dc, w, h);
    let old = SelectObject(mem, bmp as HGDIOBJ);
    SetViewportOrgEx(mem, -rc.left, -rc.top, null_mut());
    f(mem);
    SetViewportOrgEx(mem, 0, 0, null_mut());
    BitBlt(dc, rc.left, rc.top, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old);
    DeleteObject(bmp as HGDIOBJ);
    DeleteDC(mem);
}

fn tag_color(tag: &str) -> COLORREF {
    let h = tag.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    TAG_COLORS[(h % TAG_COLORS.len() as u32) as usize]
}

fn app_color(app: &str) -> COLORREF {
    tag_color(app)
}

fn cat_icon(c: Category) -> char {
    match c {
        Category::Image => IC_IMAGE,
        Category::Text => IC_TEXT,
        Category::Files => IC_FOLDER,
        Category::Link => IC_LINK,
        Category::Html => IC_HTML,
        Category::Code => IC_CODE,
    }
}

fn item_icon(item: &ClipItem) -> char {
    match &item.content {
        ClipContent::Files { paths } if paths.len() == 1 => {
            if std::path::Path::new(&paths[0]).extension().is_some() {
                IC_FILE
            } else {
                IC_FOLDER
            }
        }
        _ => cat_icon(item.category()),
    }
}

/// 현지 시간 - UTC (초)
fn utc_offset() -> i64 {
    let t = util::local_time();
    let local = clipstash::history::days_from_civil(t.year as i64, t.month as u32, t.day as u32) * 86_400
        + t.hour as i64 * 3600
        + t.minute as i64 * 60
        + t.second as i64;
    ((local - util::now_unix()) as f64 / 900.0).round() as i64 * 900
}

unsafe fn thumbnail(a: &mut App, id: u64, w: i32, h: i32) -> Option<(HBITMAP, i32, i32)> {
    if let Some(t) = a.manager.thumbs.get(&id) {
        return Some(*t);
    }
    let ClipContent::Image { file, .. } = &a.history.get(id)?.content else { return None };
    let data = std::fs::read(storage::image_path(&a.data_dir, file)).ok()?;
    let dib = bmp::bmp_to_dib(&data)?;
    let t = scaled_bitmap(dib, w, h)?;
    a.manager.thumbs.insert(id, t);
    Some(t)
}

unsafe fn scaled_bitmap(dib: &[u8], max_w: i32, max_h: i32) -> Option<(HBITMAP, i32, i32)> {
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

unsafe fn blit(dc: HDC, bmp: HBITMAP, x: i32, y: i32, w: i32, h: i32) {
    let mem = CreateCompatibleDC(dc);
    let old = SelectObject(mem, bmp as HGDIOBJ);
    BitBlt(dc, x, y, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old);
    DeleteDC(mem);
}

// ── 창 그리기 ───────────────────────────────────────────────────

unsafe fn paint(a: &mut App, dc: HDC) {
    let m = &mut a.manager;
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(m.hwnd, &mut rc);
    fill(dc, &rc, BG);
    m.hits.clear();
    let g = geo(m);
    paint_sidebar(a, dc, &g, rc);
    paint_header(a, dc, &g, rc);
    paint_center(a, dc, &g);
    paint_right(a, dc, &g);
    paint_bottom(a, dc, &g);
}

unsafe fn paint_sidebar(a: &mut App, dc: HDC, g: &Geo, rc: RECT) {
    let m = &mut a.manager;
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    // 로고와 이름
    let logo = RECT { left: s(16), top: s(14), right: s(48), bottom: s(46) };
    round(dc, &logo, s(10), ACCENT, None);
    icon(dc, m, IC_PASTE, &logo, TEXT);
    let tr = RECT { left: s(56), top: s(10), right: g.side_w, bottom: s(50) };
    line_text(dc, "클립보드 매니저", &tr, TEXT, m.font_title);

    // 분류별 개수
    let mut counts: HashMap<Category, usize> = HashMap::new();
    let mut favs = 0;
    for i in &a.history.items {
        *counts.entry(i.category()).or_default() += 1;
        favs += i.favorite as usize;
    }
    let total = a.history.items.len();
    let mut y = g.top_h + s(4);
    let row_h = s(36);
    let x0 = s(8);
    let x1 = g.side_w - s(10);
    let mut entries: Vec<(Side, char, String, Option<usize>)> = vec![(Side::All, IC_ALL, "전체 내역".into(), Some(total))];
    for c in Category::ALL {
        entries.push((Side::Cat(c), cat_icon(c), c.label().into(), Some(*counts.get(&c).unwrap_or(&0))));
    }
    for (side, ic, label, count) in entries {
        side_row(m, dc, &mut y, row_h, x0, x1, &side, ic, &label, count, None);
    }
    y += s(6);
    fill(dc, &RECT { left: s(16), top: y, right: x1 - s(6), bottom: y + 1 }, LINE);
    y += s(8);
    side_row(m, dc, &mut y, row_h, x0, x1, &Side::Favorites, IC_STAR, "즐겨찾기", Some(favs), Some(STAR));

    // 태그
    let hr = RECT { left: x0, top: y, right: x1, bottom: y + row_h };
    icon(dc, m, IC_TAG, &RECT { left: hr.left + s(8), right: hr.left + s(32), ..hr }, MUTED);
    line_text(dc, "태그", &RECT { left: hr.left + s(40), ..hr }, TEXT, m.font);
    let plus = RECT { left: x1 - s(30), top: y + s(6), right: x1 - s(6), bottom: y + row_h - s(6) };
    round(dc, &plus, s(24), PANEL2, None);
    text(dc, &IC_ADD.to_string(), &plus, TEXT, m.icons_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    m.hits.push((plus, Hit::AddTag));
    y += row_h;
    let tags = a.history.tag_counts();
    let tag_bottom = rc.bottom - s(150);
    for (t, n) in tags.iter() {
        if y + s(30) > tag_bottom {
            break;
        }
        let side = Side::Tag(t.clone());
        let active = m.side == side;
        let w = text_w(dc, t, m.font_small) + s(64);
        let pill = RECT { left: x0 + s(8), top: y + s(3), right: (x0 + s(8) + w).min(x1), bottom: y + s(27) };
        round(dc, &pill, s(24), if active { SEL } else { PANEL2 }, if active { Some(ACCENT) } else { None });
        let dot = RECT { left: pill.left + s(10), top: pill.top + s(9), right: pill.left + s(17), bottom: pill.top + s(16) };
        round(dc, &dot, s(8), tag_color(t), None);
        line_text(dc, t, &RECT { left: pill.left + s(24), right: pill.right - s(26), ..pill }, TEXT, m.font_small);
        text(dc, &n.to_string(), &RECT { right: pill.right - s(10), ..pill }, MUTED, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
        m.hits.push((pill, Hit::Side(side)));
        y += s(30);
    }
    let add = RECT { left: x0 + s(8), top: y, right: x1, bottom: y + s(28) };
    line_text(dc, "+  태그 추가", &RECT { left: add.left + s(6), ..add }, MUTED, m.font_small);
    m.hits.push((add, Hit::AddTag));

    // 설정과 저장 공간
    let sy = rc.bottom - s(140);
    fill(dc, &RECT { left: s(16), top: sy, right: x1 - s(6), bottom: sy + 1 }, LINE);
    let set = RECT { left: x0, top: sy + s(8), right: x1, bottom: sy + s(8) + row_h };
    icon(dc, m, IC_SETTINGS, &RECT { left: set.left + s(8), right: set.left + s(32), ..set }, MUTED);
    line_text(dc, "설정", &RECT { left: set.left + s(40), ..set }, TEXT, m.font);
    m.hits.push((set, Hit::Settings));

    let by = rc.bottom - s(78);
    let mb = m.storage_bytes as f64 / 1_048_576.0;
    icon(dc, m, IC_STORAGE, &RECT { left: s(14), top: by, right: s(36), bottom: by + s(22) }, MUTED);
    line_text(dc, &format!("보관 내역 {}개", total), &RECT { left: s(42), top: by, right: x1, bottom: by + s(22) }, TEXT, m.font_small);
    let bar = RECT { left: s(16), top: by + s(30), right: x1 - s(6), bottom: by + s(36) };
    round(dc, &bar, s(6), PANEL2, None);
    let frac = (mb / STORAGE_CAP_MB).clamp(0.0, 1.0);
    let fw = ((bar.right - bar.left) as f64 * frac) as i32;
    if fw > 0 {
        round(dc, &RECT { right: bar.left + fw.max(s(6)), ..bar }, s(6), ACCENT, None);
    }
    let info = format!("{:.1} MB / 1 GB ({:.0}%)", mb, frac * 100.0);
    text(dc, &info, &RECT { left: s(16), top: by + s(40), right: x1 - s(6), bottom: by + s(60) }, MUTED, m.font_small, DT_SINGLELINE | DT_RIGHT | DT_VCENTER);
}

#[allow(clippy::too_many_arguments)]
unsafe fn side_row(
    m: &mut Manager,
    dc: HDC,
    y: &mut i32,
    row_h: i32,
    x0: i32,
    x1: i32,
    side: &Side,
    ic: char,
    label: &str,
    count: Option<usize>,
    ic_color: Option<COLORREF>,
) {
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    let r = RECT { left: x0, top: *y, right: x1, bottom: *y + row_h - s(2) };
    let active = m.side == *side;
    if active {
        round(dc, &r, s(10), SEL, Some(ACCENT));
    }
    icon(dc, m, ic, &RECT { left: r.left + s(8), right: r.left + s(32), ..r }, ic_color.unwrap_or(if active { TEXT } else { MUTED }));
    line_text(dc, label, &RECT { left: r.left + s(40), right: r.right - s(50), ..r }, TEXT, if active { m.font_bold } else { m.font });
    if let Some(n) = count {
        let t = n.to_string();
        let w = text_w(dc, &t, m.font_small) + s(16);
        let pill = RECT { left: r.right - s(8) - w, top: r.top + s(8), right: r.right - s(8), bottom: r.bottom - s(8) };
        round(dc, &pill, s(20), if active { ACCENT } else { PANEL2 }, None);
        text(dc, &t, &pill, TEXT, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }
    m.hits.push((r, Hit::Side(side.clone())));
    *y += row_h;
}

unsafe fn paint_header(a: &mut App, dc: HDC, g: &Geo, rc: RECT) {
    let m = &mut a.manager;
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    let sb = g.search_box;
    let focused = GetFocus() == m.search;
    round(dc, &sb, s(12), PANEL, Some(if focused { ACCENT } else { LINE }));
    icon(dc, m, IC_SEARCH, &RECT { left: sb.left + s(10), right: sb.left + s(36), ..sb }, MUTED);
    text(dc, "Ctrl + F", &RECT { right: sb.right - s(14), ..sb }, MUTED, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
    // 오른쪽 버튼들
    let mut x = sb.right + s(10);
    let bw = s(40);
    let buttons: [(char, Option<Hit>, bool); 4] = [
        (IC_FILTER, Some(Hit::HeaderFilter), m.app_filter.is_some()),
        (IC_LIST, None, true),
        (IC_SETTINGS, Some(Hit::HeaderSettings), false),
        (IC_MORE, Some(Hit::HeaderMore), false),
    ];
    let _ = rc;
    for (ic, hit, active) in buttons {
        let r = RECT { left: x, top: sb.top, right: x + bw - s(4), bottom: sb.bottom };
        if hit.is_none() {
            // 목록 보기(지금 보기) 표시
            round(dc, &r, s(10), ACCENT, None);
        } else {
            round(dc, &r, s(10), PANEL, Some(if active { ACCENT } else { LINE }));
        }
        icon(dc, m, ic, &r, if active { TEXT } else { MUTED });
        if let Some(h) = hit {
            m.hits.push((r, h));
        }
        x += bw;
    }
}

unsafe fn paint_center(a: &mut App, dc: HDC, g: &Geo) {
    let m = &mut a.manager;
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    let c = g.center;
    // 탭
    let mut x = c.left + s(4);
    let tab_h = s(40);
    for (tab, ic, label) in [(Tab::History, IC_HISTORY, "기록"), (Tab::Pinned, IC_PIN, "고정"), (Tab::Templates, IC_TEMPLATE, "템플릿")] {
        let w = text_w(dc, label, m.font_bold) + s(56);
        let r = RECT { left: x, top: c.top, right: x + w, bottom: c.top + tab_h };
        let active = m.tab == tab;
        let col = if active { TEXT } else { MUTED };
        icon(dc, m, ic, &RECT { left: r.left + s(8), right: r.left + s(30), ..r }, if active { ACCENT_TEXT } else { MUTED });
        line_text(dc, label, &RECT { left: r.left + s(34), ..r }, col, if active { m.font_bold } else { m.font });
        if active {
            round(dc, &RECT { left: r.left + s(6), top: r.bottom - s(3), right: r.right - s(6), bottom: r.bottom }, s(3), ACCENT, None);
        }
        m.hits.push((r, Hit::Tab(tab)));
        x += w + s(6);
    }
    // 프로그램 필터
    let label = match &m.app_filter {
        Some(f) => app_display_name(f).to_string(),
        None => "모든 프로그램".into(),
    };
    let w = text_w(dc, &label, m.font) + s(36);
    let r = RECT { left: c.right - w - s(4), top: c.top, right: c.right - s(4), bottom: c.top + tab_h };
    line_text(dc, &label, &RECT { right: r.right - s(22), ..r }, if m.app_filter.is_some() { ACCENT_TEXT } else { TEXT }, m.font);
    text(dc, &IC_DOWN.to_string(), &RECT { left: r.right - s(18), ..r }, MUTED, m.icons_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    m.hits.push((r, Hit::AppFilter));
    // 목록 바탕
    round(dc, &g.list_panel, s(14), PANEL, Some(LINE));
    if m.rows.is_empty() {
        let msg = if !m.query.is_empty() { "검색 결과가 없습니다" } else { "아직 항목이 없습니다" };
        let r = RECT { top: g.list_panel.top + s(60), ..g.list_panel };
        text(dc, msg, &r, MUTED, m.font, DT_CENTER | DT_SINGLELINE);
    }
}

unsafe fn paint_right(a: &mut App, dc: HDC, g: &Geo) {
    let dpi = a.manager.dpi;
    let s = move |v: i32| v * dpi / 96;
    let r = g.right;
    let split = r.top + (r.bottom - r.top) * 62 / 100;
    let pv = RECT { bottom: split - s(6), ..r };
    let dt = RECT { top: split + s(6), ..r };
    round(dc, &pv, s(14), PANEL, Some(LINE));
    round(dc, &dt, s(14), PANEL, Some(LINE));
    let cur = a.manager.current();
    let item = match cur {
        Some(RowRef::Clip(id)) => a.history.get(id).cloned(),
        _ => None,
    };
    // 미리보기 머리
    let head = RECT { left: pv.left + s(14), top: pv.top + s(8), right: pv.right - s(10), bottom: pv.top + s(44) };
    let (ic, title) = match (&item, cur) {
        (Some(i), _) => {
            let t = match &i.content {
                ClipContent::Image { width, height, .. } => format!("이미지 ({} × {})", width, height),
                ClipContent::Files { paths } => format!("파일/폴더 ({}개)", paths.len()),
                ClipContent::Text { text, .. } => format!("{} ({}자)", i.category().label(), text.chars().count()),
            };
            (item_icon(i), t)
        }
        (None, Some(RowRef::Template(_))) => (IC_TEMPLATE, "템플릿".to_string()),
        _ => (IC_INFO, "미리보기".to_string()),
    };
    {
        let m = &a.manager;
        icon(dc, m, ic, &RECT { right: head.left + s(24), ..head }, ACCENT_TEXT);
        line_text(dc, &title, &RECT { left: head.left + s(30), right: head.right - s(100), ..head }, TEXT, m.font_bold);
    }
    if item.is_some() {
        let mut x = head.right - s(96);
        for (ic, hit) in [(IC_OPEN, Hit::PvOpen), (IC_COPY, Hit::PvCopy), (IC_SAVE, Hit::PvSave)] {
            let b = RECT { left: x, top: head.top + s(4), right: x + s(28), bottom: head.bottom - s(4) };
            icon(dc, &a.manager, ic, &b, MUTED);
            a.manager.hits.push((b, hit));
            x += s(32);
        }
    }
    let body = RECT { left: pv.left + s(14), top: head.bottom + s(4), right: pv.right - s(14), bottom: pv.bottom - s(14) };
    match (&item, cur) {
        (Some(i), _) => match &i.content {
            ClipContent::Image { file, .. } => {
                let id = i.id;
                if a.manager.preview_dib.as_ref().map(|p| p.0) != Some(id) {
                    let data = std::fs::read(storage::image_path(&a.data_dir, file)).ok();
                    let dib = data.as_deref().and_then(bmp::bmp_to_dib).map(|d| d.to_vec());
                    a.manager.preview_dib = dib.map(|d| (id, d));
                }
                if let Some((_, dib)) = &a.manager.preview_dib {
                    if let Some((bw, bh)) = bmp::dib_size(dib) {
                        let (aw, ah) = ((body.right - body.left) as f64, (body.bottom - body.top) as f64);
                        let sc = (aw / bw as f64).min(ah / bh as f64).min(1.0);
                        let (w, h) = ((bw as f64 * sc) as i32, (bh as f64 * sc) as i32);
                        let x = body.left + ((aw as i32 - w) / 2);
                        let y = body.top + ((ah as i32 - h) / 2);
                        if let Some(off) = bmp::pixel_offset(dib) {
                            SetStretchBltMode(dc, HALFTONE);
                            SetBrushOrgEx(dc, 0, 0, null_mut());
                            StretchDIBits(
                                dc,
                                x,
                                y,
                                w,
                                h,
                                0,
                                0,
                                bw as i32,
                                bh as i32,
                                dib.as_ptr().add(off as usize) as *const _,
                                dib.as_ptr() as *const BITMAPINFO,
                                DIB_RGB_COLORS,
                                SRCCOPY,
                            );
                        }
                    }
                }
            }
            ClipContent::Files { paths } => {
                let t: String = paths.iter().take(200).map(|p| format!("• {}\n", p)).collect();
                text(dc, &t, &body, TEXT, a.manager.font_small, DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
            }
            ClipContent::Text { text: t, .. } => {
                let t: String = t.chars().take(4000).collect();
                text(dc, &t, &body, TEXT, a.manager.font_small, DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL | DT_END_ELLIPSIS);
            }
        },
        (None, Some(RowRef::Template(idx))) => {
            if let Some(t) = a.config.templates.get(idx) {
                text(dc, &t.text, &body, TEXT, a.manager.font_small, DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL);
            }
        }
        _ => {}
    }

    // 상세 정보
    let m = &mut a.manager;
    let head = RECT { left: dt.left + s(14), top: dt.top + s(8), right: dt.right - s(10), bottom: dt.top + s(42) };
    icon(dc, m, IC_INFO, &RECT { right: head.left + s(24), ..head }, ACCENT_TEXT);
    line_text(dc, "상세 정보", &RECT { left: head.left + s(30), ..head }, TEXT, m.font_bold);
    let Some(i) = item else { return };
    let offset = utc_offset();
    let now = util::now_unix();
    let size = match &i.content {
        ClipContent::Image { width, height, file } => {
            let kb = std::fs::metadata(storage::image_path(&a.data_dir, file)).map(|m| m.len() / 1024).unwrap_or(0);
            format!("{} × {} ({} KB)", width, height, kb)
        }
        ClipContent::Files { paths } => format!("{}개", paths.len()),
        ClipContent::Text { text, .. } => format!("{}자 · {}줄", text.chars().count(), text.lines().count().max(1)),
    };
    let rows: [(char, &str, String); 4] = [
        (IC_SIZE, "크기", size),
        (IC_APP, "복사한 프로그램", i.source_app.as_deref().map(app_display_name).unwrap_or("알 수 없음").to_string()),
        (IC_CLOCK, "복사한 시간", format!("{}\n({})", format_datetime(i.created, offset), relative_time(now, i.created))),
        (IC_PASTE, "사용 횟수", format!("{}번", i.use_count)),
    ];
    let mut y = head.bottom + s(2);
    let label_w = if dt.right - dt.left < s(300) { s(92) } else { s(104) };
    for (ic, label, value) in rows {
        // 값이 몇 줄로 그려질지 재서 줄 높이를 정합니다.
        let vw = (dt.right - s(12)) - (dt.left + s(14) + label_w + s(30));
        let mut calc = RECT { left: 0, top: 0, right: vw.max(s(40)), bottom: 0 };
        {
            let mut wv: Vec<u16> = value.encode_utf16().collect();
            let old = SelectObject(dc, m.font_small as HGDIOBJ);
            DrawTextW(dc, wv.as_mut_ptr(), wv.len() as i32, &mut calc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
            SelectObject(dc, old);
        }
        let h = (calc.bottom + s(10)).max(s(26));
        let r = RECT { left: dt.left + s(14), top: y, right: dt.right - s(12), bottom: y + h };
        icon(dc, m, ic, &RECT { right: r.left + s(20), bottom: r.top + s(26), ..r }, MUTED);
        line_text(dc, label, &RECT { left: r.left + s(28), right: r.left + label_w + s(28), bottom: r.top + s(26), ..r }, MUTED, m.font_small);
        let vr = RECT { left: r.left + label_w + s(30), top: r.top + s(4), ..r };
        text(dc, &value, &vr, TEXT, m.font_small, DT_WORDBREAK | DT_END_ELLIPSIS);
        y += h;
        if y > dt.bottom - s(40) {
            break;
        }
    }
    // 태그
    let r = RECT { left: dt.left + s(14), top: y + s(2), right: dt.right - s(12), bottom: y + s(30) };
    if r.bottom <= dt.bottom - s(4) {
        fill(dc, &RECT { top: r.top - s(2), bottom: r.top - s(1), ..r }, LINE);
        icon(dc, m, IC_TAG, &RECT { right: r.left + s(20), ..r }, MUTED);
        line_text(dc, "태그", &RECT { left: r.left + s(28), right: r.left + label_w, ..r }, MUTED, m.font_small);
        let mut x = r.left + label_w + s(30);
        for t in &i.tags {
            let w = text_w(dc, t, m.font_small) + s(30);
            if x + w > r.right {
                break;
            }
            let pill = RECT { left: x, top: r.top + s(3), right: x + w, bottom: r.bottom - s(3) };
            round(dc, &pill, s(22), PANEL2, None);
            round(dc, &RECT { left: pill.left + s(8), top: pill.top + s(8), right: pill.left + s(14), bottom: pill.top + s(14) }, s(6), tag_color(t), None);
            line_text(dc, t, &RECT { left: pill.left + s(18), ..pill }, TEXT, m.font_small);
            m.hits.push((pill, Hit::DetailTag(t.clone())));
            x += w + s(6);
        }
        let w = s(84);
        if x + w <= r.right {
            let add = RECT { left: x, top: r.top + s(3), right: x + w, bottom: r.bottom - s(3) };
            round(dc, &add, s(22), PANEL2, Some(LINE));
            text(dc, "+ 태그 추가", &add, TEXT, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            m.hits.push((add, Hit::DetailAddTag));
        }
    }
}

unsafe fn paint_bottom(a: &mut App, dc: HDC, g: &Geo) {
    let m = &mut a.manager;
    let dpi = m.dpi;
    let s = move |v: i32| v * dpi / 96;
    let b = g.bottom;
    round(dc, &b, s(14), PANEL, Some(LINE));
    let n = m.targets().len();
    let cb = RECT { left: b.left + s(16), top: (b.top + b.bottom) / 2 - s(9), right: b.left + s(34), bottom: (b.top + b.bottom) / 2 + s(9) };
    round(dc, &cb, s(6), if n > 0 { ACCENT } else { PANEL2 }, None);
    if n > 0 {
        text(dc, &IC_CHECK.to_string(), &cb, TEXT, m.icons_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    }
    line_text(dc, &format!("{}개 선택", n), &RECT { left: cb.right + s(10), right: cb.right + s(110), ..b }, TEXT, m.font);

    let buttons: [(Tool, char, &str, COLORREF, COLORREF); 6] = [
        (Tool::Copy, IC_COPY, "복사 (Ctrl+C)", PANEL2, TEXT),
        (Tool::Paste, IC_PASTE, "붙여넣기 (Enter)", ACCENT, TEXT),
        (Tool::Favorite, IC_STAR, "즐겨찾기", PANEL2, TEXT),
        (Tool::Tag, IC_TAG, "태그", PANEL2, TEXT),
        (Tool::Delete, IC_DELETE, "삭제", PANEL2, DANGER),
        (Tool::More, IC_MORE, "", PANEL2, TEXT),
    ];
    // 자리가 모자라면 단축키 표시를 빼고 짧게
    let avail = b.right - b.left - s(160);
    let full: i32 = buttons.iter().map(|(_, _, l, _, _)| if l.is_empty() { s(46) } else { text_w(dc, l, m.font) + s(56) }).sum::<i32>() + s(50);
    let short = full > avail;
    let buttons: Vec<(Tool, char, String, COLORREF, COLORREF)> = buttons
        .into_iter()
        .map(|(t, ic, l, bg, fg)| (t, ic, if short { l.split(" (").next().unwrap_or(l).to_string() } else { l.to_string() }, bg, fg))
        .collect();
    let widths: Vec<i32> = buttons
        .iter()
        .map(|(_, _, l, _, _)| if l.is_empty() { s(46) } else { text_w(dc, l, m.font) + s(if short { 48 } else { 56 }) })
        .collect();
    let total: i32 = widths.iter().sum::<i32>() + s(10) * (widths.len() as i32 - 1);
    let mut x = (b.left + s(150)).max((b.left + b.right - total) / 2);
    for ((tool, ic, label, bg, fg), w) in buttons.into_iter().zip(widths) {
        if x + w > b.right - s(8) {
            break;
        }
        let r = RECT { left: x, top: b.top + s(8), right: x + w, bottom: b.bottom - s(8) };
        round(dc, &r, s(10), bg, if bg == ACCENT { None } else { Some(LINE) });
        if label.is_empty() {
            icon(dc, m, ic, &r, fg);
        } else {
            icon(dc, m, ic, &RECT { left: r.left + s(10), right: r.left + s(36), ..r }, fg);
            line_text(dc, &label, &RECT { left: r.left + s(38), ..r }, fg, m.font);
        }
        m.hits.push((r, Hit::Tool(tool)));
        x += w + s(10);
    }
}

/// 목록 한 줄
unsafe fn draw_row(a: &mut App, dis: &DRAWITEMSTRUCT) {
    let dc = dis.hDC;
    let rc = dis.rcItem;
    fill(dc, &rc, PANEL);
    if dis.itemID == u32::MAX {
        return;
    }
    let idx = dis.itemID as usize;
    let Some(row) = a.manager.rows.get(idx).copied() else { return };
    let dpi = a.manager.dpi;
    let s = move |v: i32| v * dpi / 96;
    let selected = dis.itemState & ODS_SELECTED != 0;
    let checked = matches!(row, RowRef::Clip(id) if a.manager.checks.contains(&id));
    if selected {
        let r = RECT { left: rc.left + s(2), top: rc.top + s(1), right: rc.right - s(2), bottom: rc.bottom - s(1) };
        round(dc, &r, s(10), SEL, Some(ACCENT));
    }
    let cy = (rc.top + rc.bottom) / 2;
    // 체크 상자
    if let RowRef::Clip(id) = row {
        let cb = RECT { left: rc.left + s(12), top: cy - s(9), right: rc.left + s(30), bottom: cy + s(9) };
        if checked {
            round(dc, &cb, s(6), ACCENT, None);
            let order = a.manager.checks.iter().position(|x| *x == id).unwrap_or(0);
            if a.manager.checks.len() > 1 {
                text(dc, &(order + 1).to_string(), &cb, TEXT, a.manager.font_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            } else {
                text(dc, &IC_CHECK.to_string(), &cb, TEXT, a.manager.icons_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
            }
        } else {
            round(dc, &cb, s(6), if selected { SEL } else { PANEL }, Some(MUTED));
        }
    }
    // 번호
    let nr = RECT { left: rc.left + s(36), top: rc.top, right: rc.left + s(66), bottom: rc.bottom };
    text(dc, &(idx + 1).to_string(), &nr, MUTED, a.manager.font_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
    let w = rc.right - rc.left;
    let star_x = rc.right - s(34);
    let time_r = RECT { left: rc.right - s(110), top: rc.top, right: star_x - s(8), bottom: rc.bottom };
    let app_w = if w < s(640) { s(118) } else { s(150) };
    let app_r = RECT { left: time_r.left - s(8) - app_w, top: rc.top, right: time_r.left - s(8), bottom: rc.bottom };
    let narrow = w < s(430);
    let thumb_box = RECT { left: rc.left + s(72), top: rc.top + s(3), right: rc.left + s(132), bottom: rc.bottom - s(3) };
    let title_r = RECT { left: thumb_box.right + s(10), top: rc.top, right: if narrow { time_r.left - s(8) } else { app_r.left - s(10) }, bottom: rc.bottom };

    match row {
        RowRef::Template(i) => {
            let Some(t) = a.config.templates.get(i) else { return };
            icon(dc, &a.manager, IC_TEMPLATE, &thumb_box, ACCENT_TEXT);
            line_text(dc, &t.name, &title_r, TEXT, a.manager.font);
            if let Some(gp) = &t.group {
                line_text(dc, gp, &app_r, MUTED, a.manager.font_small);
            }
        }
        RowRef::Clip(id) => {
            let Some(item) = a.history.get(id).cloned() else { return };
            let is_image = matches!(item.content, ClipContent::Image { .. });
            let thumb = if is_image { thumbnail(a, id, thumb_box.right - thumb_box.left, thumb_box.bottom - thumb_box.top) } else { None };
            let m = &a.manager;
            if let Some((b, tw, th)) = thumb {
                let x = thumb_box.left + (thumb_box.right - thumb_box.left - tw) / 2;
                let y = thumb_box.top + (thumb_box.bottom - thumb_box.top - th) / 2;
                blit(dc, b, x, y, tw, th);
            } else {
                let col = match item.category() {
                    Category::Files => STAR,
                    Category::Link => ACCENT_TEXT,
                    Category::Html => rgb(251, 146, 60),
                    Category::Code => rgb(167, 139, 250),
                    _ => MUTED,
                };
                icon(dc, m, item_icon(&item), &thumb_box, col);
            }
            let title = match &item.content {
                ClipContent::Image { width, height, .. } => format!("[이미지] {}×{}", width, height),
                _ => item.preview(300),
            };
            let tcol = if is_image || title.starts_with("[파일") { ACCENT_TEXT } else { TEXT };
            line_text(dc, &title, &title_r, tcol, m.font);
            if !narrow {
                if let Some(app) = &item.source_app {
                    let name = app_display_name(app);
                    let badge = RECT { left: app_r.left, top: cy - s(9), right: app_r.left + s(18), bottom: cy + s(9) };
                    round(dc, &badge, s(6), app_color(app), None);
                    let first: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                    text(dc, &first, &badge, BG, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                    line_text(dc, name, &RECT { left: badge.right + s(8), ..app_r }, TEXT, m.font_small);
                }
            }
            text(dc, &relative_time(util::now_unix(), item.last_used), &time_r, MUTED, m.font_small, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);
            let sr = RECT { left: star_x, top: rc.top, right: rc.right - s(6), bottom: rc.bottom };
            icon(dc, m, if item.favorite { IC_STAR_FILL } else { IC_STAR }, &sr, if item.favorite { STAR } else { MUTED });
            if item.pinned {
                let pr = RECT { left: title_r.right - s(18), ..title_r };
                let _ = pr;
            }
        }
    }
}

// ── 동작 ────────────────────────────────────────────────────────

fn toggle_check(a: &mut App, id: u64) {
    let m = &mut a.manager;
    match m.checks.iter().position(|x| *x == id) {
        Some(i) => {
            m.checks.remove(i);
        }
        None => m.checks.push(id),
    }
    unsafe {
        InvalidateRect(m.list, null(), 0);
        InvalidateRect(m.hwnd, null(), 0);
    }
}

/// 클립보드에 넣기(여러 개면 체크한 순서대로 이어 붙임)
fn copy_targets(a: &mut App, ids: &[u64]) -> bool {
    let ok = if ids.len() == 1 {
        a.write_item(ids[0], false)
    } else {
        match a.history.merge_text(ids, &a.config.merge_separator) {
            Some(t) => clip::write(a.main, &a.formats, clip::Payload::Text { text: &t, html: None }),
            None => false,
        }
    };
    if ok {
        let now = util::now_unix();
        for id in ids {
            a.history.mark_used(*id, now, false);
        }
        a.mark_dirty();
    } else {
        a.balloon("클립보드에 넣지 못했습니다", "이미지는 여러 개를 한 번에 넣을 수 없습니다. 하나씩 해 보세요.", true);
    }
    ok
}

fn do_copy(a: &mut App) {
    if let Some(RowRef::Template(i)) = a.manager.current() {
        if a.manager.checks.is_empty() {
            a.use_template(i, super::Action::CopyOnly);
            return;
        }
    }
    let ids = a.manager.targets();
    if !ids.is_empty() && copy_targets(a, &ids) {
        let n = ids.len();
        a.balloon("복사했습니다", &format!("{}개 항목을 클립보드에 넣었습니다.", n), false);
    }
}

fn do_paste(a: &mut App) {
    let target = a.manager.target;
    if let (Some(RowRef::Template(i)), true) = (a.manager.current(), a.manager.checks.is_empty()) {
        a.popup.target = target;
        a.use_template(i, super::Action::Paste);
        return;
    }
    let ids = a.manager.targets();
    if ids.is_empty() || !copy_targets(a, &ids) {
        return;
    }
    if target.is_null() || unsafe { IsWindow(target) } == 0 {
        a.balloon("클립보드에 넣었습니다", "붙여넣을 창을 찾지 못했습니다. 원하는 곳에서 Ctrl+V 를 누르세요.", false);
        return;
    }
    if paste::focus(target) {
        let shift_insert = util::process_name_of_window(target).map(|p| a.config.uses_shift_insert(&p)).unwrap_or(false);
        paste::send_paste(shift_insert, None);
    }
}

fn do_favorite(a: &mut App) {
    let ids = a.manager.targets();
    if ids.is_empty() {
        return;
    }
    let all_fav = ids.iter().all(|id| a.history.get(*id).is_some_and(|i| i.favorite));
    for id in &ids {
        if a.history.get(*id).is_some_and(|i| i.favorite == all_fav) {
            a.history.toggle_favorite(*id);
        }
    }
    a.mark_dirty();
    refresh(a);
}

fn do_delete(a: &mut App) {
    let ids = a.manager.targets();
    if ids.is_empty() {
        return;
    }
    if ids.len() > 1 {
        let r = util::message_box(a.manager.hwnd, &format!("{}개 항목을 지울까요?", ids.len()), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2);
        if r != IDYES {
            return;
        }
    }
    a.manager.checks.clear();
    for id in ids {
        a.delete_clip(id);
    }
    refresh(a);
}

unsafe fn menu_at(a: &App, items: &[(usize, String, bool, bool)], pt: Option<POINT>) -> usize {
    let menu = CreatePopupMenu();
    for (id, label, checked, enabled) in items {
        if *id == 0 {
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
            continue;
        }
        let w = wide(label);
        let mut f = MF_STRING;
        if *checked {
            f |= MF_CHECKED;
        }
        if !*enabled {
            f |= MF_GRAYED;
        }
        AppendMenuW(menu, f, *id, w.as_ptr());
    }
    let pt = pt.unwrap_or_else(|| {
        let mut p = POINT { x: 0, y: 0 };
        GetCursorPos(&mut p);
        p
    });
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, 0, a.manager.hwnd, null()) as usize;
    DestroyMenu(menu);
    cmd
}

/// 태그 메뉴: 붙어 있는 태그는 체크 표시, 고르면 붙이기/떼기, "새 태그…"
fn tag_menu(a: &mut App) {
    let ids = a.manager.targets();
    if ids.is_empty() {
        return;
    }
    let tags = a.history.tag_counts();
    let mut items: Vec<(usize, String, bool, bool)> = Vec::new();
    for (i, (t, _)) in tags.iter().enumerate() {
        let all = ids.iter().all(|id| a.history.get(*id).is_some_and(|it| it.tags.contains(t)));
        items.push((100 + i, t.clone(), all, true));
    }
    if !items.is_empty() {
        items.push((0, String::new(), false, true));
    }
    items.push((1, "새 태그…".into(), false, true));
    let cmd = unsafe { menu_at(a, &items, None) };
    if cmd == 1 {
        new_tag_for(a, &ids);
    } else if cmd >= 100 {
        if let Some((t, _)) = tags.get(cmd - 100) {
            let all = ids.iter().all(|id| a.history.get(*id).is_some_and(|it| it.tags.contains(t)));
            for id in &ids {
                let has = a.history.get(*id).is_some_and(|it| it.tags.contains(t));
                if has == all {
                    a.history.toggle_tag(*id, t);
                }
            }
            a.mark_dirty();
            refresh(a);
        }
    }
}

fn new_tag_for(a: &mut App, ids: &[u64]) {
    let Some(name) = prompt(a.manager.hwnd, "새 태그", "태그 이름 (예: 업무, 서버, 교회)") else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    for id in ids {
        let has = a.history.get(*id).is_some_and(|it| it.tags.contains(&name));
        if !has {
            a.history.toggle_tag(*id, &name);
        }
    }
    a.mark_dirty();
    refresh(a);
}

fn app_filter_menu(a: &mut App) {
    let mut apps: Vec<String> = Vec::new();
    for i in &a.history.items {
        if let Some(s) = &i.source_app {
            if !apps.iter().any(|x| x.eq_ignore_ascii_case(s)) {
                apps.push(s.clone());
            }
        }
    }
    apps.sort_by_key(|s| s.to_lowercase());
    let mut items = vec![(1usize, "모든 프로그램".to_string(), a.manager.app_filter.is_none(), true), (0, String::new(), false, true)];
    for (i, s) in apps.iter().enumerate().take(60) {
        let on = a.manager.app_filter.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(s));
        items.push((100 + i, app_display_name(s).to_string(), on, true));
    }
    let cmd = unsafe { menu_at(a, &items, None) };
    if cmd == 1 {
        a.manager.app_filter = None;
    } else if cmd >= 100 {
        a.manager.app_filter = apps.get(cmd - 100).cloned();
    } else {
        return;
    }
    refresh(a);
}

fn more_menu(a: &mut App) {
    let cur = a.manager.current();
    let item = match cur {
        Some(RowRef::Clip(id)) => a.history.get(id).cloned(),
        _ => None,
    };
    let is_image = matches!(item.as_ref().map(|i| &i.content), Some(ClipContent::Image { .. }));
    let pinned = item.as_ref().is_some_and(|i| i.pinned);
    let n = a.manager.checks.len();
    let items = vec![
        (1, if pinned { "고정 해제".to_string() } else { "고정".to_string() }, false, item.is_some()),
        (2, "서식 없이 붙여넣기".into(), false, item.is_some()),
        (3, "이미지에서 글자 추출 (OCR)".into(), false, is_image),
        (4, "템플릿으로 저장".into(), false, item.as_ref().is_some_and(|i| i.content.as_plain_text().is_some())),
        (0, String::new(), false, true),
        (5, "보이는 항목 모두 체크".into(), false, !a.manager.rows.is_empty()),
        (6, format!("체크 해제 ({}개)", n), false, n > 0),
        (0, String::new(), false, true),
        (7, "설정…".into(), false, true),
        (8, "데이터 폴더 열기".into(), false, true),
    ];
    let cmd = unsafe { menu_at(a, &items, None) };
    let id = item.as_ref().map(|i| i.id);
    match (cmd, id) {
        (1, Some(id)) => {
            a.toggle_pin(id);
            refresh(a);
        }
        (2, Some(_)) => {
            let ids = a.manager.targets();
            if ids.len() == 1 && a.write_item(ids[0], true) && !a.manager.target.is_null() && paste::focus(a.manager.target) {
                paste::send_paste(false, None);
            }
        }
        (3, Some(id)) => a.ocr_item(id, false),
        (4, Some(id)) => a.save_as_template(id),
        (5, _) => {
            a.manager.checks = a.manager.rows.iter().filter_map(|r| if let RowRef::Clip(id) = r { Some(*id) } else { None }).collect();
            refresh(a);
        }
        (6, _) => {
            a.manager.checks.clear();
            refresh(a);
        }
        (7, _) => super::settings::open(a),
        (8, _) => util::shell_open(&a.data_dir.display().to_string(), None),
        _ => {}
    }
}

fn header_more_menu(a: &mut App) {
    let items = vec![
        (1, "설정 파일 직접 열기".to_string(), false, true),
        (2, "데이터 폴더 열기".into(), false, true),
        (0, String::new(), false, true),
        (3, "기록 백업 파일로 내보내기…".into(), false, true),
        (4, "백업 파일에서 가져오기…".into(), false, true),
        (0, String::new(), false, true),
        (5, "빠른 팝업 열기 (Alt+C)".into(), false, true),
    ];
    match unsafe { menu_at(a, &items, None) } {
        1 => a.open_config_file(),
        2 => util::shell_open(&a.data_dir.display().to_string(), None),
        3 => a.export_backup(),
        4 => a.import_backup(),
        5 => super::popup::show(a, a.manager.target, false),
        _ => {}
    }
}

fn preview_open(a: &mut App) {
    let Some(RowRef::Clip(id)) = a.manager.current() else { return };
    let Some(item) = a.history.get(id).cloned() else { return };
    match &item.content {
        ClipContent::Image { file, .. } => util::shell_open(&storage::image_path(&a.data_dir, file).display().to_string(), None),
        ClipContent::Files { paths } => {
            if let Some(p) = paths.first() {
                util::shell_open("explorer.exe", Some(&format!("/select,\"{}\"", p)));
            }
        }
        ClipContent::Text { text, .. } => {
            let t = text.trim();
            if clipstash::history::is_link(t) {
                let url = if t.to_ascii_lowercase().starts_with("www.") { format!("https://{}", t) } else { t.to_string() };
                util::shell_open(&url, None);
            } else {
                a.balloon("열 수 있는 링크가 아닙니다", "텍스트는 복사하거나 붙여넣어 쓰세요.", false);
            }
        }
    }
}

fn preview_save(a: &mut App) {
    let Some(RowRef::Clip(id)) = a.manager.current() else { return };
    let Some(item) = a.history.get(id).cloned() else { return };
    let (filter, ext, name, data): (&str, &str, String, Option<Vec<u8>>) = match &item.content {
        ClipContent::Image { file, .. } => (
            "비트맵 이미지 (*.bmp)\0*.bmp\0\0",
            "bmp",
            format!("클립보드-이미지-{}.bmp", id),
            std::fs::read(storage::image_path(&a.data_dir, file)).ok(),
        ),
        _ => (
            "텍스트 파일 (*.txt)\0*.txt\0\0",
            "txt",
            format!("클립보드-{}.txt", id),
            item.content.as_plain_text().map(|t| {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend(t.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes());
                v
            }),
        ),
    };
    let Some(data) = data else { return };
    let mut buf = [0u16; 1024];
    util::copy_to_buf(&mut buf, &name);
    let f: Vec<u16> = filter.encode_utf16().collect();
    let e = wide(ext);
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = a.manager.hwnd;
    ofn.lpstrFilter = f.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrDefExt = e.as_ptr();
    ofn.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT;
    if unsafe { GetSaveFileNameW(&mut ofn) } == 0 {
        return;
    }
    let path = util::from_wide(&buf);
    match std::fs::write(&path, data) {
        Ok(()) => a.balloon("저장했습니다", &path, false),
        Err(e) => a.balloon("저장하지 못했습니다", &e.to_string(), true),
    }
}

fn on_hit(a: &mut App, hit: Hit) {
    match hit {
        Hit::Side(side) => {
            a.manager.side = if a.manager.side == side && side != Side::All { Side::All } else { side };
            if a.manager.tab == Tab::Templates {
                a.manager.tab = Tab::History;
            }
            refresh(a);
        }
        Hit::AddTag | Hit::DetailAddTag => {
            let ids = a.manager.targets();
            if ids.is_empty() {
                a.balloon("태그를 붙일 항목을 먼저 고르세요", "목록에서 항목을 누르거나 체크한 뒤 태그를 추가하세요.", false);
            } else {
                new_tag_for(a, &ids);
            }
        }
        Hit::DetailTag(t) => {
            if let Some(RowRef::Clip(id)) = a.manager.current() {
                let r = util::message_box(a.manager.hwnd, &format!("이 항목에서 '{}' 태그를 뗄까요?", t), MB_YESNO | MB_ICONQUESTION);
                if r == IDYES {
                    a.history.toggle_tag(id, &t);
                    a.mark_dirty();
                    refresh(a);
                }
            }
        }
        Hit::Settings | Hit::HeaderSettings => super::settings::open(a),
        Hit::Tab(t) => {
            a.manager.tab = t;
            refresh(a);
        }
        Hit::AppFilter | Hit::HeaderFilter => app_filter_menu(a),
        Hit::HeaderMore => header_more_menu(a),
        Hit::Tool(t) => match t {
            Tool::Copy => do_copy(a),
            Tool::Paste => do_paste(a),
            Tool::Favorite => do_favorite(a),
            Tool::Tag => tag_menu(a),
            Tool::Delete => do_delete(a),
            Tool::More => more_menu(a),
        },
        Hit::PvOpen => preview_open(a),
        Hit::PvCopy => {
            if let Some(RowRef::Clip(id)) = a.manager.current() {
                if copy_targets(a, &[id]) {
                    a.balloon("복사했습니다", "클립보드에 넣었습니다.", false);
                }
            }
        }
        Hit::PvSave => preview_save(a),
    }
    unsafe { InvalidateRect(a.manager.hwnd, null(), 0) };
}

fn hit_at(m: &Manager, pt: POINT) -> Option<Hit> {
    m.hits.iter().rev().find(|(r, _)| unsafe { PtInRect(r, pt) } != 0).map(|(_, h)| h.clone())
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// 목록/창의 단축키. 처리했으면 true.
fn handle_key(a: &mut App, vk: VIRTUAL_KEY) -> bool {
    let ctrl = key_down(VK_CONTROL);
    match vk {
        VK_RETURN => do_paste(a),
        VK_DELETE => do_delete(a),
        VK_SPACE => {
            if let Some(RowRef::Clip(id)) = a.manager.current() {
                toggle_check(a, id);
            }
        }
        VK_ESCAPE => {
            if !a.manager.checks.is_empty() {
                a.manager.checks.clear();
                refresh(a);
            } else if !a.manager.query.is_empty() {
                unsafe { SetWindowTextW(a.manager.search, wide("").as_ptr()) };
            } else {
                unsafe { ShowWindow(a.manager.hwnd, SW_HIDE) };
            }
        }
        _ if ctrl && vk == b'F' as u16 => unsafe {
            SetFocus(a.manager.search);
            SendMessageW(a.manager.search, EM_SETSEL, 0, -1);
        },
        _ if ctrl && vk == b'C' as u16 => do_copy(a),
        _ if ctrl && vk == b'D' as u16 => do_favorite(a),
        _ if ctrl && vk == b'T' as u16 => tag_menu(a),
        _ if ctrl && vk == b'A' as u16 => {
            a.manager.checks = a.manager.rows.iter().filter_map(|r| if let RowRef::Clip(id) = r { Some(*id) } else { None }).collect();
            refresh(a);
        }
        _ => return false,
    }
    true
}

// ── 창 프로시저 ─────────────────────────────────────────────────

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            buffered(dc, &rc, |m| paint(a, m));
            EndPaint(hwnd, &ps);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            layout(&mut a.manager);
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp as *mut MINMAXINFO);
            let d = (GetDpiForWindow(hwnd) as i32).max(96);
            mmi.ptMinTrackSize = POINT { x: 880 * d / 96, y: 560 * d / 96 };
            0
        }
        WM_DPICHANGED => {
            a.manager.dpi = ((wp & 0xFFFF) as i32).max(96);
            make_fonts(&mut a.manager);
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            clear_thumbs(&mut a.manager);
            layout(&mut a.manager);
            0
        }
        WM_MEASUREITEM => {
            let mis = &mut *(lp as *mut MEASUREITEMSTRUCT);
            mis.itemHeight = a.manager.s(31) as u32;
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
        WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            SetTextColor(dc, TEXT);
            SetBkColor(dc, PANEL);
            a.manager.panel_brush as LRESULT
        }
        WM_CTLCOLORLISTBOX => a.manager.panel_brush as LRESULT,
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as i32;
            let code = ((wp >> 16) & 0xFFFF) as u32;
            match (id, code) {
                (ID_LIST, LBN_SELCHANGE) => {
                    a.manager.preview_dib = None;
                    InvalidateRect(hwnd, null(), 0);
                }
                (ID_LIST, LBN_DBLCLK) => do_paste(a),
                (ID_SEARCH, EN_CHANGE) => {
                    let n = GetWindowTextLengthW(a.manager.search).max(0) as usize;
                    let mut buf = vec![0u16; n + 1];
                    GetWindowTextW(a.manager.search, buf.as_mut_ptr(), buf.len() as i32);
                    a.manager.query = util::from_wide(&buf);
                    refresh(a);
                }
                (ID_SEARCH, EN_SETFOCUS) | (ID_SEARCH, EN_KILLFOCUS) => {
                    InvalidateRect(hwnd, null(), 0);
                }
                _ => {}
            }
            0
        }
        WM_LBUTTONDOWN => {
            let pt = POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 };
            if let Some(h) = hit_at(&a.manager, pt) {
                on_hit(a, h);
            }
            0
        }
        WM_SETCURSOR => {
            if (lp & 0xFFFF) as u32 == HTCLIENT && wp as HWND == hwnd {
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                ScreenToClient(hwnd, &mut pt);
                if hit_at(&a.manager, pt).is_some() {
                    SetCursor(LoadCursorW(null_mut(), IDC_HAND));
                    return 1;
                }
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 != WA_INACTIVE {
                SetFocus(a.manager.list);
            }
            0
        }
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_TIMER if wp == TIMER_SHOT => {
            KillTimer(hwnd, TIMER_SHOT);
            take_shot(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn clear_thumbs(m: &mut Manager) {
    for (_, (b, _, _)) in m.thumbs.drain() {
        unsafe { DeleteObject(b as HGDIOBJ) };
    }
}

unsafe extern "system" fn list_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            if handle_key(app(), wp as VIRTUAL_KEY) {
                return 0;
            }
        }
        WM_CHAR => return 0,
        WM_GETDLGCODE => return DLGC_WANTALLKEYS as LRESULT,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let a = app();
            let r = SendMessageW(hwnd, LB_ITEMFROMPOINT, 0, lp);
            if (r >> 16) & 0xFFFF == 0 {
                let idx = (r & 0xFFFF) as usize;
                let x = (lp & 0xFFFF) as i16 as i32;
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                if let Some(RowRef::Clip(id)) = a.manager.rows.get(idx).copied() {
                    let dpi = a.manager.dpi;
    let s = move |v: i32| v * dpi / 96;
                    if x < s(34) {
                        SendMessageW(hwnd, LB_SETCURSEL, idx, 0);
                        toggle_check(a, id);
                        return 0;
                    }
                    if x > rc.right - s(38) {
                        SendMessageW(hwnd, LB_SETCURSEL, idx, 0);
                        a.history.toggle_favorite(id);
                        a.mark_dirty();
                        refresh(a);
                        return 0;
                    }
                }
            }
        }
        WM_ERASEBKGND => {
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
                fill(dc, &rc, PANEL);
            }
            return 1;
        }
        _ => {}
    }
    let old: WNDPROC = std::mem::transmute(LIST_PROC.load(Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

unsafe extern "system" fn search_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            let a = app();
            match wp as VIRTUAL_KEY {
                VK_DOWN | VK_RETURN => {
                    SetFocus(a.manager.list);
                    return 0;
                }
                VK_ESCAPE => {
                    if a.manager.query.is_empty() {
                        SetFocus(a.manager.list);
                    } else {
                        SetWindowTextW(hwnd, wide("").as_ptr());
                    }
                    return 0;
                }
                _ => {}
            }
        }
        WM_CHAR if wp == 0x0D || wp == 0x1B => return 0,
        _ => {}
    }
    let old: WNDPROC = std::mem::transmute(SEARCH_PROC.load(Ordering::Relaxed));
    CallWindowProcW(old, hwnd, msg, wp, lp)
}

// ── 글 입력 상자(새 태그 이름) ──────────────────────────────────

static PROMPT_DONE: AtomicBool = AtomicBool::new(false);
static PROMPT_OK: AtomicBool = AtomicBool::new(false);
static PROMPT_EDIT: AtomicIsize = AtomicIsize::new(0);

/// 작은 입력 창을 띄우고 입력한 글을 돌려줍니다(취소하면 None).
fn prompt(owner: HWND, title: &str, label: &str) -> Option<String> {
    unsafe {
        let a = app();
        let dpi = (GetDpiForWindow(owner) as i32).max(96);
        let s = |v: i32| v * dpi / 96;
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let mut r = RECT { left: 0, top: 0, right: s(360), bottom: s(130) };
        AdjustWindowRectEx(&mut r, style, 0, WS_EX_DLGMODALFRAME);
        let mut orc: RECT = std::mem::zeroed();
        GetWindowRect(owner, &mut orc);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = orc.left + (orc.right - orc.left - w) / 2;
        let y = orc.top + (orc.bottom - orc.top - h) / 2;
        let cls = wide(PROMPT_CLASS);
        let t = wide(title);
        let hwnd = CreateWindowExW(WS_EX_DLGMODALFRAME, cls.as_ptr(), t.as_ptr(), style, x, y, w, h, owner, null_mut(), a.hinst, null());
        if hwnd.is_null() {
            return None;
        }
        let font = a.manager.font;
        let mk = |class: &str, text: &str, st: u32, x: i32, y: i32, w: i32, h: i32, id: i32| {
            let c = wide(class);
            let t = wide(text);
            let ctl = CreateWindowExW(0, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | st, x, y, w, h, hwnd, id as isize as HMENU, null_mut(), null());
            SendMessageW(ctl, WM_SETFONT, font as usize, 0);
            ctl
        };
        mk("STATIC", label, 0, s(14), s(12), s(332), s(22), 0);
        let edit = mk("EDIT", "", WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32, s(14), s(38), s(332), s(26), 100);
        mk("BUTTON", "확인", WS_TABSTOP | BS_DEFPUSHBUTTON as u32, s(164), s(84), s(88), s(30), IDOK);
        mk("BUTTON", "취소", WS_TABSTOP | BS_PUSHBUTTON as u32, s(258), s(84), s(88), s(30), IDCANCEL);
        PROMPT_EDIT.store(edit as isize, Ordering::Relaxed);
        PROMPT_DONE.store(false, Ordering::Relaxed);
        PROMPT_OK.store(false, Ordering::Relaxed);
        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(edit);
        let mut result = None;
        let mut msg: MSG = std::mem::zeroed();
        while !PROMPT_DONE.load(Ordering::Relaxed) && GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if PROMPT_OK.load(Ordering::Relaxed) {
            let n = GetWindowTextLengthW(edit).max(0) as usize;
            let mut buf = vec![0u16; n + 1];
            GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
            result = Some(util::from_wide(&buf));
        }
        EnableWindow(owner, 1);
        DestroyWindow(hwnd);
        SetForegroundWindow(owner);
        if msg.message == WM_QUIT {
            PostQuitMessage(msg.wParam as i32);
        }
        result
    }
}

unsafe extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wp & 0xFFFF) as i32 {
                IDOK => {
                    PROMPT_OK.store(true, Ordering::Relaxed);
                    PROMPT_DONE.store(true, Ordering::Relaxed);
                }
                IDCANCEL => PROMPT_DONE.store(true, Ordering::Relaxed),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            PROMPT_DONE.store(true, Ordering::Relaxed);
            0
        }
        WM_CTLCOLORSTATIC => {
            let dc = wp as HDC;
            SetBkColor(dc, GetSysColor(COLOR_WINDOW));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ── CI 화면 확인용: 예시 기록을 넣고 매니저 창을 그림 파일로 저장 ─────────────

const TIMER_SHOT: usize = 0x5B01;
static SHOT_PATH: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// 환경 변수 CLIPSTASH_MANAGER_SHOT=결과.bmp 이면 예시 기록으로 창을 띄우고 찍은 뒤 끝냅니다.
pub fn screenshot_test(a: &mut App) -> bool {
    let Ok(path) = std::env::var("CLIPSTASH_MANAGER_SHOT") else { return false };
    if let Ok(mut p) = SHOT_PATH.lock() {
        *p = path;
    }
    // 악보처럼 보이는 예시 이미지
    let (w, h) = (388u32, 599u32);
    let mut px = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let staff = (y % 110) > 50 && (y % 110) < 90 && (y % 8) == 0;
            let note = (y % 110) > 55 && (y % 110) < 85 && (x % 47) < 6 && ((y + x) % 9) < 5;
            let words = (y % 110) > 92 && (y % 110) < 104 && (x % 13) < 9 && x > 20 && x < 370;
            if staff || note || words {
                px[i] = 30;
                px[i + 1] = 30;
                px[i + 2] = 30;
            }
        }
    }
    let samples: Vec<(ClipContent, &str)> = vec![
        (ClipContent::text("fn main() {\n    println!(\"안녕\");\n}"), "Code.exe"),
        (ClipContent::text("<div class=\"card\">회의 자료</div>"), "chrome.exe"),
        (ClipContent::text("http://192.168.0.25:26000/"), "Antigravity.exe"),
        (ClipContent::Files { paths: vec!["C:\\Users\\pc\\Downloads\\LinkIPFinder-v9.1-win64.zip".into()] }, "explorer.exe"),
        (ClipContent::Files { paths: vec!["C:\\Users\\pc\\Downloads\\BaseHymnV1.0".into()] }, "explorer.exe"),
        (ClipContent::text("오늘 회의는 3시에 2층 회의실에서 합니다."), "KakaoTalk.exe"),
        (ClipContent::Files { paths: vec!["C:\\Users\\pc\\Downloads\\찬송가.html".into()] }, "RustJump.exe"),
    ];
    for (i, (c, app)) in samples.into_iter().enumerate() {
        let h = format!("sample-{}", i);
        a.add_entry(c, h, Some(app.into()), None);
    }
    if let Some(dib) = bmp::bgra_to_dib24(w, h, &px) {
        if let Some((content, hsh)) = a.store_image(&dib) {
            a.add_entry(content, hsh, Some("SnippingTool.exe".into()), Some(&dib));
        }
    }
    let ids: Vec<u64> = a.history.items.iter().map(|i| i.id).collect();
    if ids.len() >= 4 {
        a.history.toggle_favorite(ids[1]);
        for (id, t) in [(ids[0], "교회"), (ids[2], "서버"), (ids[3], "네트워크"), (ids[2], "업무"), (ids[4], "업무")] {
            a.history.toggle_tag(id, t);
        }
    }
    open(a);
    if let Some(first) = ids.first() {
        a.manager.checks.push(*first);
    }
    unsafe {
        SetWindowPos(a.manager.hwnd, HWND_TOP, 20, 20, a.manager.s(1140), a.manager.s(760), 0);
        InvalidateRect(a.manager.hwnd, null(), 1);
        SetTimer(a.manager.hwnd, TIMER_SHOT, 2000, None);
    }
    true
}

unsafe fn take_shot(hwnd: HWND) {
    let mut wr: RECT = std::mem::zeroed();
    GetWindowRect(hwnd, &mut wr);
    let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
    let screen = GetDC(null_mut());
    let mem = CreateCompatibleDC(screen);
    let bmp_h = CreateCompatibleBitmap(screen, w, h);
    let old = SelectObject(mem, bmp_h as HGDIOBJ);
    // 화면보다 커도 창 전체를 그려 받기(PW_RENDERFULLCONTENT)
    windows_sys::Win32::Storage::Xps::PrintWindow(hwnd, mem, 2);
    SelectObject(mem, old);
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut px = vec![0u8; (w * h * 4) as usize];
    GetDIBits(mem, bmp_h, 0, h as u32, px.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);
    DeleteObject(bmp_h as HGDIOBJ);
    DeleteDC(mem);
    ReleaseDC(null_mut(), screen);
    let path = SHOT_PATH.lock().map(|p| p.clone()).unwrap_or_default();
    if let Some(file) = bmp::bgra_to_dib24(w as u32, h as u32, &px).and_then(|d| bmp::dib_to_bmp(&d)) {
        let _ = std::fs::write(&path, file);
    }
}
