//! 화면 영역 캡처: 화면 전체를 찍어 둔 뒤, 어둡게 덮은 화면 위에서 마우스로 영역을 고릅니다.
//! 드래그 = 그 영역, 그냥 클릭 = 마우스 아래 모니터 전체, Esc/오른쪽 클릭 = 취소.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::wide;
use super::{app, App};

const CLASS: &str = "ClipStash.Capture";

pub struct Capture {
    hwnd: HWND,
    /// 가상 화면(모든 모니터) 왼쪽 위 좌표와 크기
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    shot_dc: HDC,
    shot: HBITMAP,
    shot_old: HGDIOBJ,
    dim_dc: HDC,
    dim: HBITMAP,
    dim_old: HGDIOBJ,
    start: Option<POINT>,
    cur: POINT,
    /// 캡처 뒤 바로 글자 추출
    pub ocr: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Capture {
            hwnd: null_mut(),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            shot_dc: null_mut(),
            shot: null_mut(),
            shot_old: null_mut(),
            dim_dc: null_mut(),
            dim: null_mut(),
            dim_old: null_mut(),
            start: None,
            cur: POINT { x: 0, y: 0 },
            ocr: false,
        }
    }
}

impl Capture {
    pub fn active(&self) -> bool {
        !self.shot.is_null()
    }

    fn sel_rect(&self) -> Option<RECT> {
        let s = self.start?;
        Some(RECT {
            left: s.x.min(self.cur.x),
            top: s.y.min(self.cur.y),
            right: s.x.max(self.cur.x) + 1,
            bottom: s.y.max(self.cur.y) + 1,
        })
    }
}

pub fn register_class(hinst: HINSTANCE) {
    unsafe {
        let cls = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(capture_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: LoadCursorW(null_mut(), IDC_CROSS),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

pub fn start(a: &mut App, ocr: bool) {
    if a.capture.active() {
        return;
    }
    super::popup::hide(a);
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return;
        }
        let screen = GetDC(null_mut());
        let shot_dc = CreateCompatibleDC(screen);
        let shot = CreateCompatibleBitmap(screen, w, h);
        let dim_dc = CreateCompatibleDC(screen);
        let dim = CreateCompatibleBitmap(screen, w, h);
        if shot.is_null() || dim.is_null() {
            for b in [shot, dim] {
                if !b.is_null() {
                    DeleteObject(b as HGDIOBJ);
                }
            }
            DeleteDC(shot_dc);
            DeleteDC(dim_dc);
            ReleaseDC(null_mut(), screen);
            a.balloon("화면을 캡처하지 못했습니다", "메모리가 부족할 수 있습니다.", true);
            return;
        }
        let shot_old = SelectObject(shot_dc, shot as HGDIOBJ);
        BitBlt(shot_dc, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
        ReleaseDC(null_mut(), screen);

        // 어둡게 덮은 사본 + 안내 문구
        let dim_old = SelectObject(dim_dc, dim as HGDIOBJ);
        let full = RECT { left: 0, top: 0, right: w, bottom: h };
        FillRect(dim_dc, &full, GetStockObject(BLACK_BRUSH) as HBRUSH);
        let bf = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 110, AlphaFormat: 0 };
        AlphaBlend(dim_dc, 0, 0, w, h, shot_dc, 0, 0, w, h, bf);
        draw_hint(dim_dc, x, y);

        a.capture = Capture {
            hwnd: a.capture.hwnd,
            x,
            y,
            w,
            h,
            shot_dc,
            shot,
            shot_old,
            dim_dc,
            dim,
            dim_old,
            start: None,
            cur: POINT { x: 0, y: 0 },
            ocr,
        };
        if a.capture.hwnd.is_null() {
            let cls = wide(CLASS);
            a.capture.hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                cls.as_ptr(),
                null(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                null_mut(),
                null_mut(),
                a.hinst,
                null(),
            );
        }
        let hwnd = a.capture.hwnd;
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
        InvalidateRect(hwnd, null(), 0);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
}

/// 마우스가 있는 모니터 위쪽 가운데에 안내 문구
unsafe fn draw_hint(dc: HDC, vx: i32, vy: i32) {
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(mon, &mut mi);
    let m = mi.rcMonitor;
    let dpi = {
        let mut dx = 96u32;
        let mut dy = 96u32;
        if windows_sys::Win32::UI::HiDpi::GetDpiForMonitor(mon, windows_sys::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI, &mut dx, &mut dy) != 0 {
            dx = 96;
        }
        dx as i32
    };
    let s = |v: i32| v * dpi / 96;
    let face = wide("맑은 고딕");
    let font = CreateFontW(-s(15), 0, 0, 0, 600, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
    let old = SelectObject(dc, font as HGDIOBJ);
    let text = "드래그해서 영역 선택  ·  클릭 = 모니터 전체  ·  Esc 취소";
    let mut w: Vec<u16> = text.encode_utf16().collect();
    let mut calc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut calc, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    let (tw, th) = (calc.right + s(32), calc.bottom + s(16));
    let left = (m.left + m.right) / 2 - tw / 2 - vx;
    let top = m.top + s(28) - vy;
    let box_r = RECT { left, top, right: left + tw, bottom: top + th };
    let brush = CreateSolidBrush(0x00302B28);
    let pen = CreatePen(PS_SOLID, 1, 0x00504A45);
    let ob = SelectObject(dc, brush as HGDIOBJ);
    let op = SelectObject(dc, pen as HGDIOBJ);
    RoundRect(dc, box_r.left, box_r.top, box_r.right, box_r.bottom, s(14), s(14));
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(brush as HGDIOBJ);
    DeleteObject(pen as HGDIOBJ);
    SetTextColor(dc, 0x00F0EEEC);
    SetBkMode(dc, TRANSPARENT as _);
    let mut r = box_r;
    DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    DeleteObject(font as HGDIOBJ);
}

fn cleanup(c: &mut Capture) {
    // 먼저 상태를 비워서, 창을 숨길 때 오는 WM_ACTIVATE 가 다시 정리하지 않게 합니다.
    let hwnd = c.hwnd;
    let old = std::mem::replace(c, Capture { hwnd, ..Default::default() });
    unsafe {
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
        if !old.shot.is_null() {
            SelectObject(old.shot_dc, old.shot_old);
            SelectObject(old.dim_dc, old.dim_old);
            DeleteObject(old.shot as HGDIOBJ);
            DeleteObject(old.dim as HGDIOBJ);
            DeleteDC(old.shot_dc);
            DeleteDC(old.dim_dc);
        }
    }
}

/// 고른 영역을 24비트 DIB 로 잘라 냅니다.
unsafe fn crop(c: &Capture, r: RECT) -> Option<Vec<u8>> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut bits: *mut core::ffi::c_void = null_mut();
    let sec = CreateDIBSection(c.shot_dc, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if sec.is_null() || bits.is_null() {
        return None;
    }
    let mem = CreateCompatibleDC(c.shot_dc);
    let old = SelectObject(mem, sec as HGDIOBJ);
    BitBlt(mem, 0, 0, w, h, c.shot_dc, r.left, r.top, SRCCOPY);
    GdiFlush();
    let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
    SelectObject(mem, old);
    DeleteDC(mem);
    DeleteObject(sec as HGDIOBJ);
    clipstash::bmp::bgra_to_dib24(w as u32, h as u32, &px)
}

fn finish(a: &mut App, r: RECT) {
    let dib = unsafe { crop(&a.capture, r) };
    let ocr = a.capture.ocr;
    cleanup(&mut a.capture);
    match dib {
        Some(dib) => a.add_screen_capture(dib, ocr),
        None => a.balloon("캡처하지 못했습니다", "영역을 다시 골라 보세요.", true),
    }
}

pub fn cancel(a: &mut App) {
    if a.capture.active() {
        cleanup(&mut a.capture);
    }
}

fn pt(lp: LPARAM) -> POINT {
    POINT { x: (lp & 0xFFFF) as i16 as i32, y: ((lp >> 16) & 0xFFFF) as i16 as i32 }
}

/// 선택 영역(과 크기 글자)이 차지하는 부분
fn dirty_rect(c: &Capture) -> Option<RECT> {
    let r = c.sel_rect()?;
    let m = 120;
    Some(RECT { left: r.left - 4, top: r.top - 40, right: r.right + m, bottom: r.bottom + 40 })
}

unsafe fn invalidate(c: &Capture, before: Option<RECT>) {
    for r in [before, dirty_rect(c)].into_iter().flatten() {
        InvalidateRect(c.hwnd, &r, 0);
    }
}

unsafe fn paint(c: &Capture, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let p = ps.rcPaint;
    let (pw, ph) = (p.right - p.left, p.bottom - p.top);
    if pw > 0 && ph > 0 && c.active() {
        // 깜빡임 없게 다시 그릴 부분만 메모리에 그린 뒤 한 번에 옮깁니다.
        let buf_dc = CreateCompatibleDC(dc);
        let buf = CreateCompatibleBitmap(dc, pw, ph);
        let old = SelectObject(buf_dc, buf as HGDIOBJ);
        SetViewportOrgEx(buf_dc, -p.left, -p.top, null_mut());
        BitBlt(buf_dc, p.left, p.top, pw, ph, c.dim_dc, p.left, p.top, SRCCOPY);
        if let Some(r) = c.sel_rect() {
            BitBlt(buf_dc, r.left, r.top, r.right - r.left, r.bottom - r.top, c.shot_dc, r.left, r.top, SRCCOPY);
            let accent = CreateSolidBrush(0x00FFA05C);
            FrameRect(buf_dc, &RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, accent);
            FrameRect(buf_dc, &RECT { left: r.left - 2, top: r.top - 2, right: r.right + 2, bottom: r.bottom + 2 }, accent);
            DeleteObject(accent as HGDIOBJ);
            // 크기 표시
            let label = format!(" {} × {} ", r.right - r.left, r.bottom - r.top);
            let mut w: Vec<u16> = label.encode_utf16().collect();
            SetBkMode(buf_dc, OPAQUE as _);
            SetBkColor(buf_dc, 0x00302B28);
            SetTextColor(buf_dc, 0x00FFFFFF);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            let of = SelectObject(buf_dc, font);
            let y = if r.top > 24 { r.top - 22 } else { r.bottom + 4 };
            TextOutW(buf_dc, r.left, y, w.as_mut_ptr(), w.len() as i32);
            SelectObject(buf_dc, of);
        }
        SetViewportOrgEx(buf_dc, 0, 0, null_mut());
        BitBlt(dc, p.left, p.top, pw, ph, buf_dc, 0, 0, SRCCOPY);
        SelectObject(buf_dc, old);
        DeleteObject(buf as HGDIOBJ);
        DeleteDC(buf_dc);
    }
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn capture_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = app();
    match msg {
        WM_PAINT => {
            paint(&a.capture, hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_LBUTTONDOWN => {
            let c = &mut a.capture;
            let p = pt(lp);
            c.start = Some(p);
            c.cur = p;
            SetCapture(hwnd);
            invalidate(c, None);
            0
        }
        WM_MOUSEMOVE => {
            let c = &mut a.capture;
            if c.start.is_some() {
                let before = dirty_rect(c);
                c.cur = pt(lp);
                invalidate(c, before);
            }
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let c = &mut a.capture;
            c.cur = pt(lp);
            let Some(mut r) = c.sel_rect() else { return 0 };
            if r.right - r.left < 4 || r.bottom - r.top < 4 {
                // 그냥 클릭: 그 모니터 전체
                let sp = POINT { x: c.cur.x + c.x, y: c.cur.y + c.y };
                let mon = MonitorFromPoint(sp, MONITOR_DEFAULTTONEAREST);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(mon, &mut mi);
                let m = mi.rcMonitor;
                r = RECT { left: m.left - c.x, top: m.top - c.y, right: m.right - c.x, bottom: m.bottom - c.y };
            }
            r.left = r.left.max(0);
            r.top = r.top.max(0);
            r.right = r.right.min(c.w);
            r.bottom = r.bottom.min(c.h);
            finish(a, r);
            0
        }
        WM_RBUTTONUP => {
            cancel(a);
            0
        }
        WM_KEYDOWN => {
            if wp as u16 == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE {
                cancel(a);
            }
            0
        }
        WM_ACTIVATE => {
            if (wp & 0xFFFF) as u32 == WA_INACTIVE {
                cancel(a);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
