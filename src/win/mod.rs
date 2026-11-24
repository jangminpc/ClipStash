//! Windows 전용 부분: 숨은 메인 창(클립보드 감시, 전역 단축키, 트레이 아이콘)과 팝업.

mod clip;
mod paste;
mod popup;
mod util;

use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use clipstash::config::{Config, DEFAULT_CONFIG_TOML};
use clipstash::history::{ClipContent, History};
use clipstash::template::{self, Template};
use clipstash::transform::Transform;
use clipstash::hotkey::DoubleTap;
use clipstash::{bmp, hash, hotkey, storage};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardOwner, RemoveClipboardFormatListener,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use util::wide;

pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_SHOW: u32 = WM_APP + 2;
pub const WM_APP_DOUBLE_COPY: u32 = WM_APP + 3;
const HOTKEY_ID: i32 = 1;
const TIMER_SAVE: usize = 1;
const MAIN_CLASS: &str = "ClipStash.Main";
// NIN_SELECT / NIN_KEYSELECT (shellapi.h)
const TRAY_SELECT: u32 = WM_USER;
const TRAY_KEYSELECT: u32 = WM_USER + 1;

// 트레이 메뉴 명령
const CMD_OPEN: usize = 1;
const CMD_PAUSE: usize = 2;
const CMD_AUTOSTART: usize = 3;
const CMD_EDIT_CONFIG: usize = 4;
const CMD_RELOAD: usize = 5;
const CMD_DATA_DIR: usize = 6;
const CMD_CLEAR: usize = 7;
const CMD_ABOUT: usize = 8;
const CMD_EXIT: usize = 9;

/// 항목을 골랐을 때 할 일
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// 설정에 따라(서식 포함 또는 텍스트만) 붙여넣기
    Paste,
    /// 서식 없이 텍스트만 붙여넣기
    PastePlain,
    /// 텍스트를 바꿔서 붙여넣기
    Transform(Transform),
    /// 클립보드에 넣기만 하고 붙여넣지 않음
    CopyOnly,
}

pub struct App {
    pub hinst: HINSTANCE,
    pub main: HWND,
    pub data_dir: PathBuf,
    pub config: Config,
    config_mtime: Option<SystemTime>,
    pub history: History,
    pub paused: bool,
    pub formats: clip::Formats,
    icon_normal: HICON,
    icon_paused: HICON,
    hotkey_ok: bool,
    kb_hook: HHOOK,
    taskbar_created: u32,
    pub popup: popup::Popup,
}

static APP: AtomicPtr<App> = AtomicPtr::new(null_mut());

/// 이 프로그램은 UI 스레드 하나에서만 동작하므로 전역 상태를 이렇게 꺼내 씁니다.
pub fn app() -> &'static mut App {
    unsafe { &mut *APP.load(Ordering::Relaxed) }
}

pub fn run() {
    unsafe {
        // 이미 실행 중이면 그쪽 팝업만 띄우고 끝냅니다.
        let mutex_name = wide("Local\\ClipStash.SingleInstance");
        let _mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let cls = wide(MAIN_CLASS);
            let other = FindWindowW(cls.as_ptr(), null());
            if !other.is_null() {
                PostMessageW(other, WM_APP_SHOW, 0, 0);
            }
            return;
        }
        // 매니페스트에도 적혀 있지만 혹시 몰라 한 번 더
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinst = GetModuleHandleW(null());
        let data_dir = util::data_dir();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            util::message_box(
                null_mut(),
                &format!("데이터 폴더를 만들 수 없습니다:\n{}\n\n{}", data_dir.display(), e),
                MB_ICONERROR,
            );
            return;
        }

        // 설정 파일이 없으면 설명이 달린 기본 파일을 만듭니다.
        let config_path = data_dir.join(storage::CONFIG_FILE);
        if !config_path.exists() {
            let _ = std::fs::write(&config_path, DEFAULT_CONFIG_TOML);
        }
        let mut config_error = None;
        let config = match std::fs::read_to_string(&config_path) {
            Ok(s) => Config::from_toml(&s).unwrap_or_else(|e| {
                config_error = Some(e);
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        let config_mtime = std::fs::metadata(&config_path).and_then(|m| m.modified()).ok();

        let mut history = storage::load_history(&data_dir);
        let evicted = history.trim(config.max_items);
        storage::delete_item_files(&data_dir, &evicted);
        storage::remove_orphan_images(&data_dir, &history);

        let load_icon = |id: u16, size: i32| {
            LoadImageW(hinst, id as usize as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) as HICON
        };
        let small = GetSystemMetrics(SM_CXSMICON);
        let icon_normal = load_icon(1, small);
        let icon_paused = load_icon(2, small);

        let cls = wide(MAIN_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(main_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: load_icon(1, GetSystemMetrics(SM_CXICON)),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: icon_normal,
        };
        RegisterClassExW(&wc);
        popup::register_classes(hinst);

        let app_box = Box::new(App {
            hinst,
            main: null_mut(),
            data_dir,
            config,
            config_mtime,
            history,
            paused: false,
            formats: clip::Formats::register(),
            icon_normal,
            icon_paused,
            hotkey_ok: false,
            kb_hook: null_mut(),
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            popup: popup::Popup::default(),
        });
        APP.store(Box::into_raw(app_box), Ordering::Relaxed);

        // 숨은 최상위 창(TaskbarCreated 방송을 받으려면 메시지 전용 창이면 안 됨)
        let title = wide("ClipStash");
        let main = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            cls.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        let a = app();
        a.main = main;
        popup::create(a);
        AddClipboardFormatListener(main);
        a.tray(NIM_ADD);
        a.register_hotkey();
        a.update_keyboard_hook();
        if let Some(e) = config_error {
            a.balloon("설정 파일 오류 — 기본 설정으로 시작합니다", &e, true);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        app().shutdown();
    }
}

impl App {
    fn config_path(&self) -> PathBuf {
        self.data_dir.join(storage::CONFIG_FILE)
    }

    fn register_hotkey(&mut self) {
        unsafe {
            if self.hotkey_ok {
                UnregisterHotKey(self.main, HOTKEY_ID);
                self.hotkey_ok = false;
            }
            if self.config.hotkey.trim().is_empty() {
                self.tray(NIM_MODIFY);
                return;
            }
            match hotkey::parse(&self.config.hotkey) {
                Ok(hk) => {
                    let mods = hk.modifiers as HOT_KEY_MODIFIERS | MOD_NOREPEAT;
                    self.hotkey_ok = RegisterHotKey(self.main, HOTKEY_ID, mods, hk.vk) != 0;
                    if !self.hotkey_ok {
                        self.balloon(
                            "단축키를 등록하지 못했습니다",
                            &format!(
                                "{} 는 다른 프로그램이 이미 쓰고 있습니다. 트레이 메뉴 → 설정 파일 열기에서 hotkey 를 바꿔주세요.",
                                self.config.hotkey
                            ),
                            true,
                        );
                    }
                }
                Err(e) => self.balloon("단축키 설정 오류", &e, true),
            }
        }
        self.tray(NIM_MODIFY);
    }

    /// "Ctrl+C 두 번" 감지용 키보드 훅을 설정에 맞게 켜고 끕니다.
    /// 키를 가로채지는 않으므로 평소 복사는 그대로 동작합니다.
    fn update_keyboard_hook(&mut self) {
        DOUBLE_COPY_MS.store(self.config.double_copy_ms, Ordering::Relaxed);
        unsafe {
            if self.config.double_copy_ms > 0 && self.kb_hook.is_null() {
                self.kb_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), self.hinst, 0);
                if self.kb_hook.is_null() {
                    self.balloon(
                        "Ctrl+C 두 번 감지를 켜지 못했습니다",
                        &format!("{} 로 팝업을 열어 주세요.", self.config.hotkey),
                        true,
                    );
                }
            } else if self.config.double_copy_ms == 0 && !self.kb_hook.is_null() {
                UnhookWindowsHookEx(self.kb_hook);
                self.kb_hook = null_mut();
            }
        }
    }

    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.main;
        nid.uID = 1;
        nid
    }

    pub fn tray(&self, op: NOTIFY_ICON_MESSAGE) {
        let mut nid = self.tray_data();
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = if self.paused { self.icon_paused } else { self.icon_normal };
        let tip = if self.paused {
            "ClipStash — 기록 일시정지됨".to_string()
        } else {
            format!("ClipStash — 기록 {}개\n{} 로 열기", self.history.items.len(), self.config.shortcut_label())
        };
        util::copy_to_buf(&mut nid.szTip, &tip);
        unsafe {
            Shell_NotifyIconW(op, &nid);
            if op == NIM_ADD {
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &nid);
            }
        }
    }

    pub fn balloon(&self, title: &str, text: &str, warn: bool) {
        let mut nid = self.tray_data();
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = if warn { NIIF_WARNING } else { NIIF_INFO };
        util::copy_to_buf(&mut nid.szInfoTitle, title);
        util::copy_to_buf(&mut nid.szInfo, text);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    /// 기록이 바뀌었음 → 잠시 뒤에 한꺼번에 저장
    pub fn mark_dirty(&mut self) {
        unsafe { SetTimer(self.main, TIMER_SAVE, 1500, None) };
        self.tray(NIM_MODIFY);
    }

    fn save_now(&mut self) {
        unsafe { KillTimer(self.main, TIMER_SAVE) };
        if let Err(e) = storage::save_history(&self.data_dir, &self.history) {
            self.balloon("기록을 저장하지 못했습니다", &e.to_string(), true);
        }
    }

    fn shutdown(&mut self) {
        self.save_now();
        unsafe {
            RemoveClipboardFormatListener(self.main);
            if self.hotkey_ok {
                UnregisterHotKey(self.main, HOTKEY_ID);
            }
            if !self.kb_hook.is_null() {
                UnhookWindowsHookEx(self.kb_hook);
            }
            let nid = self.tray_data();
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    /// 설정 파일이 바뀌었으면(또는 force) 다시 읽습니다.
    pub fn reload_config(&mut self, force: bool) {
        let path = self.config_path();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if !force && mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                // 지워졌으면 기본 파일을 다시 만들어 둡니다.
                let _ = std::fs::write(&path, DEFAULT_CONFIG_TOML);
                DEFAULT_CONFIG_TOML.to_string()
            }
        };
        match Config::from_toml(&text) {
            Ok(c) => {
                let hotkey_changed = c.hotkey != self.config.hotkey;
                self.config = c;
                if hotkey_changed || !self.hotkey_ok {
                    self.register_hotkey();
                }
                self.update_keyboard_hook();
                let evicted = self.history.trim(self.config.max_items);
                if !evicted.is_empty() {
                    storage::delete_item_files(&self.data_dir, &evicted);
                    self.mark_dirty();
                }
                popup::on_config_changed(self);
                if force {
                    self.balloon("설정을 다시 불러왔습니다", &format!("팝업 열기: {}", self.config.shortcut_label()), false);
                }
            }
            Err(e) => self.balloon("설정 파일 오류 — 이전 설정을 유지합니다", &e, true),
        }
    }

    pub fn open_config_file(&self) {
        util::shell_open("notepad.exe", Some(&format!("\"{}\"", self.config_path().display())));
    }

    /// 텍스트 항목을 템플릿으로 설정 파일 끝에 추가합니다(주석은 그대로 둡니다).
    pub fn save_as_template(&mut self, id: u64) {
        let Some(text) = self.history.get(id).and_then(|i| i.content.as_plain_text()) else { return };
        let name: String = clipstash::history::one_line(&text, 24);
        let t = Template { name, group: Some("내 템플릿".into()), text };
        let Ok(body) = toml::to_string(&t) else { return };
        let path = self.config_path();
        let mut cur = std::fs::read_to_string(&path).unwrap_or_default();
        if !cur.ends_with('\n') {
            cur.push('\n');
        }
        cur.push_str("\n[[templates]]\n");
        cur.push_str(&body);
        match storage::write_atomic(&path, cur.as_bytes()) {
            Ok(()) => {
                self.reload_config(true);
                self.balloon(
                    "템플릿으로 저장했습니다",
                    &format!("\"{}\" — 팝업의 템플릿 탭에서 볼 수 있습니다.", t.name),
                    false,
                );
            }
            Err(e) => self.balloon("템플릿을 저장하지 못했습니다", &e.to_string(), true),
        }
    }

    fn on_clipboard_update(&mut self) {
        if self.paused {
            return;
        }
        // 어느 프로그램이 복사했는지(클립보드 소유 창 → 없으면 지금 앞에 있는 창)
        let owner = unsafe { GetClipboardOwner() };
        if util::is_own_window(owner) {
            return;
        }
        let source = util::process_name_of_window(owner)
            .or_else(|| util::process_name_of_window(unsafe { GetForegroundWindow() }));
        if let Some(s) = &source {
            if self.config.is_excluded(s) {
                return;
            }
        }
        let opt = clip::ReadOptions {
            text: self.config.capture_text,
            images: self.config.capture_images,
            files: self.config.capture_files,
            html: self.config.keep_html,
            max_text_bytes: self.config.max_text_kb * 1024,
            max_image_bytes: self.config.max_image_mb * 1024 * 1024,
        };
        let captured = match clip::read(self.main, &self.formats, &opt) {
            clip::ReadResult::Got(c) => c,
            _ => return,
        };
        let (content, h) = match captured {
            clip::Captured::Text { text, html } => {
                if text.is_empty() || (self.config.ignore_whitespace_only && text.trim().is_empty()) {
                    return;
                }
                let h = hash::content_hash("text", text.as_bytes());
                (ClipContent::Text { text, html }, h)
            }
            clip::Captured::Files { paths } => {
                let h = hash::content_hash("files", paths.join("\n").as_bytes());
                (ClipContent::Files { paths }, h)
            }
            clip::Captured::Image { dib } => {
                let h = hash::content_hash("image", &dib);
                let Some((width, height)) = bmp::dib_size(&dib) else { return };
                let file = format!("{}.bmp", h);
                let path = storage::image_path(&self.data_dir, &file);
                if !path.exists() {
                    let Some(data) = bmp::dib_to_bmp(&dib) else { return };
                    if storage::write_atomic(&path, &data).is_err() {
                        return;
                    }
                }
                (ClipContent::Image { file, width, height }, h)
            }
        };
        let out = self.history.add(content, h, source, util::now_unix(), self.config.max_items);
        storage::delete_item_files(&self.data_dir, &out.evicted);
        self.mark_dirty();
        popup::on_history_changed(self);
    }

    /// 기록 항목을 클립보드에 넣고(필요하면) 붙여넣기
    pub fn use_clip(&mut self, id: u64, action: Action) {
        let Some(item) = self.history.get(id).cloned() else { return };
        let plain = action == Action::PastePlain || self.config.plain_text_by_default;
        let ok = match (&item.content, action) {
            (ClipContent::Image { file, .. }, _) => {
                let data = std::fs::read(storage::image_path(&self.data_dir, file)).ok();
                match data.as_deref().and_then(bmp::bmp_to_dib) {
                    Some(dib) => clip::write(self.main, &self.formats, clip::Payload::Image { dib }),
                    None => false,
                }
            }
            (c, Action::Transform(t)) => match c.as_plain_text() {
                Some(text) => {
                    let s = t.apply(&text);
                    clip::write(self.main, &self.formats, clip::Payload::Text { text: &s, html: None })
                }
                None => false,
            },
            (ClipContent::Text { text, html }, _) => clip::write(
                self.main,
                &self.formats,
                clip::Payload::Text { text, html: if plain { None } else { html.as_deref() } },
            ),
            (ClipContent::Files { paths }, _) if plain => {
                let s = paths.join("\r\n");
                clip::write(self.main, &self.formats, clip::Payload::Text { text: &s, html: None })
            }
            (ClipContent::Files { paths }, _) => {
                clip::write(self.main, &self.formats, clip::Payload::Files { paths })
            }
        };
        if !ok {
            popup::hide(self);
            self.balloon(
                "클립보드에 넣지 못했습니다",
                "다른 프로그램이 클립보드를 쓰고 있을 수 있습니다. 잠시 뒤 다시 해 보세요.",
                true,
            );
            return;
        }
        self.history.mark_used(id, util::now_unix(), self.config.move_to_top_on_paste);
        self.mark_dirty();
        self.finish_paste(action == Action::CopyOnly, None);
    }

    /// 템플릿을 펼쳐서 붙여넣기
    pub fn use_template(&mut self, index: usize, action: Action) {
        let Some(t) = self.config.templates.get(index).cloned() else { return };
        let clipboard = if t.text.contains("{clipboard}") { clip::read_text(self.main) } else { None };
        let ctx = template::Context { now: util::local_time(), clipboard: clipboard.as_deref() };
        let mut ex = template::expand(&t.text, &ctx);
        if let Action::Transform(tr) = action {
            ex.text = tr.apply(&ex.text);
            ex.cursor_back = None;
        }
        if !clip::write(self.main, &self.formats, clip::Payload::Text { text: &ex.text, html: None }) {
            popup::hide(self);
            self.balloon("클립보드에 넣지 못했습니다", "잠시 뒤 다시 해 보세요.", true);
            return;
        }
        self.finish_paste(action == Action::CopyOnly, ex.cursor_back);
    }

    fn finish_paste(&mut self, copy_only: bool, cursor_back: Option<usize>) {
        let target = self.popup.target;
        popup::hide(self);
        if copy_only || !self.config.paste_after_select || target.is_null() {
            return;
        }
        if paste::focus(target) {
            let shift_insert = util::process_name_of_window(target)
                .map(|p| self.config.uses_shift_insert(&p))
                .unwrap_or(false);
            paste::send_paste(shift_insert, cursor_back);
        }
    }

    pub fn delete_clip(&mut self, id: u64) {
        if let Some(item) = self.history.remove(id) {
            storage::delete_item_files(&self.data_dir, &[item]);
            self.mark_dirty();
            popup::on_history_changed(self);
        }
    }

    pub fn toggle_pin(&mut self, id: u64) {
        if self.history.toggle_pin(id).is_some() {
            self.mark_dirty();
            popup::on_history_changed(self);
        }
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.tray(NIM_MODIFY);
        popup::on_history_changed(self);
    }

    fn show_tray_menu(&mut self) {
        unsafe {
            let m = CreatePopupMenu();
            let add = |flags: MENU_ITEM_FLAGS, id: usize, text: &str| {
                let t = wide(text);
                AppendMenuW(m, flags, id, t.as_ptr());
            };
            let checked = |b: bool| if b { MF_CHECKED } else { MF_UNCHECKED };
            add(MF_STRING, CMD_OPEN, &format!("기록 열기\t{}", self.config.shortcut_label()));
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING | checked(self.paused), CMD_PAUSE, "기록 일시정지");
            add(MF_STRING | checked(util::autostart_enabled()), CMD_AUTOSTART, "Windows 시작 시 자동 실행");
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING, CMD_EDIT_CONFIG, "설정 파일 열기 (템플릿·단축키·제외 프로그램)");
            add(MF_STRING, CMD_RELOAD, "설정 다시 불러오기");
            add(MF_STRING, CMD_DATA_DIR, "데이터 폴더 열기");
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING, CMD_CLEAR, "고정하지 않은 기록 모두 지우기…");
            add(MF_SEPARATOR, 0, "");
            add(MF_STRING, CMD_ABOUT, "ClipStash 정보");
            add(MF_STRING, CMD_EXIT, "종료");
            SetMenuDefaultItem(m, CMD_OPEN as u32, 0);

            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // 메뉴 밖을 누르면 닫히도록 하는 Windows 의 오래된 요령
            SetForegroundWindow(self.main);
            let cmd = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                pt.x,
                pt.y,
                0,
                self.main,
                null(),
            ) as usize;
            PostMessageW(self.main, WM_NULL, 0, 0);
            DestroyMenu(m);
            self.run_tray_command(cmd);
        }
    }

    fn run_tray_command(&mut self, cmd: usize) {
        match cmd {
            CMD_OPEN => popup::show(self, null_mut(), true),
            CMD_PAUSE => self.set_paused(!self.paused),
            CMD_AUTOSTART => {
                let want = !util::autostart_enabled();
                if !util::set_autostart(want) {
                    self.balloon("자동 실행 설정을 바꾸지 못했습니다", "레지스트리에 쓸 수 없습니다.", true);
                }
            }
            CMD_EDIT_CONFIG => self.open_config_file(),
            CMD_RELOAD => self.reload_config(true),
            CMD_DATA_DIR => util::shell_open(&self.data_dir.display().to_string(), None),
            CMD_CLEAR => {
                let n = self.history.items.iter().filter(|i| !i.pinned).count();
                let r = util::message_box(
                    self.main,
                    &format!("고정하지 않은 기록 {}개를 모두 지울까요?\n고정한 항목은 남습니다.", n),
                    MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                );
                if r == IDYES {
                    let gone = self.history.clear_unpinned();
                    storage::delete_item_files(&self.data_dir, &gone);
                    self.mark_dirty();
                    popup::on_history_changed(self);
                }
            }
            CMD_ABOUT => {
                util::message_box(
                    self.main,
                    &format!(
                        "ClipStash {}\n가벼운 클립보드 기록 관리자\n\n팝업 열기: {}\n데이터 폴더: {}",
                        env!("CARGO_PKG_VERSION"),
                        self.config.shortcut_label(),
                        self.data_dir.display()
                    ),
                    MB_ICONINFORMATION,
                );
            }
            CMD_EXIT => unsafe {
                DestroyWindow(self.main);
            },
            _ => {}
        }
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let a = APP.load(Ordering::Relaxed);
    if a.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let a = &mut *a;
    match msg {
        WM_CLIPBOARDUPDATE => {
            a.on_clipboard_update();
            0
        }
        WM_HOTKEY if wp as i32 == HOTKEY_ID => {
            if a.popup.visible() {
                popup::hide(a);
            } else {
                popup::show(a, GetForegroundWindow(), false);
            }
            0
        }
        WM_APP_DOUBLE_COPY => {
            if !a.popup.visible() {
                popup::show(a, GetForegroundWindow(), false);
            }
            0
        }
        WM_APP_SHOW => {
            popup::show(a, null_mut(), true);
            0
        }
        WM_APP_TRAY => {
            // NOTIFYICON_VERSION_4: LOWORD(lParam) 가 이벤트
            match (lp as u32) & 0xFFFF {
                // 왼쪽 클릭/키보드 선택 → 팝업, 오른쪽 클릭 → 메뉴
                TRAY_SELECT | TRAY_KEYSELECT => popup::show(a, null_mut(), true),
                WM_CONTEXTMENU => a.show_tray_menu(),
                _ => {}
            }
            0
        }
        WM_TIMER if wp == TIMER_SAVE => {
            a.save_now();
            0
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wp != 0 {
                a.save_now();
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ if msg == a.taskbar_created && msg != 0 => {
            // 탐색기가 다시 시작되면 트레이 아이콘을 다시 올립니다.
            a.tray(NIM_ADD);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ── Ctrl+C 두 번 감지 ─────────────────────────────────────────

static DOUBLE_COPY_MS: AtomicU32 = AtomicU32::new(0);
static C_HELD: AtomicBool = AtomicBool::new(false);
static DOUBLE_TAP: Mutex<DoubleTap> = Mutex::new(DoubleTap::new());

fn pressed(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

/// 저수준 키보드 훅. 키를 막지 않고 지켜보기만 하다가, Ctrl+C 가 짧은 간격으로
/// 두 번 눌리면 메인 창에 알립니다. 훅은 빨리 끝나야 하므로 여기서는 메시지만 보냅니다.
unsafe extern "system" fn keyboard_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp as *const KBDLLHOOKSTRUCT);
        let msg = wp as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let injected = k.flags & LLKHF_INJECTED != 0;
        if !injected {
            if k.vkCode == 'C' as u32 {
                if down {
                    // 꾹 누르고 있을 때 생기는 자동 반복은 무시
                    let repeat = C_HELD.swap(true, Ordering::Relaxed);
                    let only_ctrl = pressed(VK_CONTROL)
                        && !pressed(VK_SHIFT)
                        && !pressed(VK_MENU)
                        && !pressed(VK_LWIN)
                        && !pressed(VK_RWIN);
                    if !repeat && only_ctrl {
                        let window = DOUBLE_COPY_MS.load(Ordering::Relaxed);
                        let hit = DOUBLE_TAP.lock().map(|mut d| d.press(k.time, window)).unwrap_or(false);
                        if hit {
                            let main = APP.load(Ordering::Relaxed);
                            if !main.is_null() {
                                PostMessageW((*main).main, WM_APP_DOUBLE_COPY, 0, 0);
                            }
                        }
                    } else if !repeat {
                        if let Ok(mut d) = DOUBLE_TAP.lock() {
                            d.reset();
                        }
                    }
                } else {
                    C_HELD.store(false, Ordering::Relaxed);
                }
            } else if down && !matches!(k.vkCode, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C) {
                // 사이에 다른 키를 누르면 처음부터 다시 셉니다(Ctrl/Shift/Alt/Win 은 제외)
                if let Ok(mut d) = DOUBLE_TAP.lock() {
                    d.reset();
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}
