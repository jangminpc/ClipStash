//! 설정 파일(config.toml). 처음 실행하면 주석이 달린 기본 파일을 만들어 줍니다.

use crate::template::Template;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PopupPosition {
    /// 글자 입력 커서(캐럿) 옆, 알 수 없으면 마우스 위치
    Caret,
    /// 마우스 커서 위치
    Mouse,
    /// 화면 가운데
    Center,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub double_copy_ms: u32,
    pub max_items: usize,
    pub capture_text: bool,
    pub capture_images: bool,
    pub capture_files: bool,
    pub max_text_kb: usize,
    pub max_image_mb: usize,
    pub keep_html: bool,
    pub paste_after_select: bool,
    pub plain_text_by_default: bool,
    pub move_to_top_on_paste: bool,
    pub ignore_whitespace_only: bool,
    pub popup_position: PopupPosition,
    pub popup_width: u32,
    pub popup_height: u32,
    pub font_name: String,
    pub font_size: u32,
    pub excluded_apps: Vec<String>,
    pub shift_insert_apps: Vec<String>,
    pub templates: Vec<Template>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: "Ctrl+Shift+V".into(),
            double_copy_ms: 400,
            max_items: 300,
            capture_text: true,
            capture_images: true,
            capture_files: true,
            max_text_kb: 1024,
            max_image_mb: 30,
            keep_html: true,
            paste_after_select: true,
            plain_text_by_default: false,
            move_to_top_on_paste: true,
            ignore_whitespace_only: true,
            popup_position: PopupPosition::Caret,
            popup_width: 480,
            popup_height: 540,
            font_name: "맑은 고딕".into(),
            font_size: 10,
            excluded_apps: DEFAULT_EXCLUDED.iter().map(|s| s.to_string()).collect(),
            shift_insert_apps: DEFAULT_SHIFT_INSERT.iter().map(|s| s.to_string()).collect(),
            templates: vec![
                Template {
                    name: "오늘 날짜".into(),
                    group: Some("기본".into()),
                    text: "{date:yyyy년 M월 d일 (E)}".into(),
                },
                Template {
                    name: "현재 시각".into(),
                    group: Some("기본".into()),
                    text: "{time}".into(),
                },
                Template {
                    name: "메일 맺음말".into(),
                    group: Some("업무".into()),
                    text: "{cursor}\n\n감사합니다.\n".into(),
                },
            ],
        }
    }
}

pub const DEFAULT_EXCLUDED: [&str; 7] = [
    "keepass.exe",
    "keepassxc.exe",
    "1password.exe",
    "bitwarden.exe",
    "enpass.exe",
    "lastpass.exe",
    "dashlane.exe",
];

pub const DEFAULT_SHIFT_INSERT: [&str; 3] = ["putty.exe", "mintty.exe", "kitty.exe"];

/// 프로그램 이름 목록에 `process_name` 이 있는지(대소문자/경로/.exe 무시).
pub fn app_in_list(list: &[String], process_name: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().to_lowercase();
        let s = s.rsplit(['\\', '/']).next().unwrap_or(&s).to_string();
        s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
    };
    let p = norm(process_name);
    !p.is_empty() && list.iter().any(|e| norm(e) == p)
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Config, String> {
        let mut c: Config = toml::from_str(s).map_err(|e| e.to_string())?;
        c.max_items = c.max_items.clamp(10, 100_000);
        c.popup_width = c.popup_width.clamp(240, 2000);
        c.popup_height = c.popup_height.clamp(200, 2000);
        c.font_size = c.font_size.clamp(6, 40);
        if c.double_copy_ms != 0 {
            c.double_copy_ms = c.double_copy_ms.clamp(150, 1500);
        }
        Ok(c)
    }

    /// 사람이 읽는 팝업 단축키 설명(트레이 메뉴/풍선 도움말용)
    pub fn shortcut_label(&self) -> String {
        let mut parts = Vec::new();
        if self.double_copy_ms > 0 {
            parts.push("Ctrl+C 두 번".to_string());
        }
        if !self.hotkey.trim().is_empty() {
            parts.push(self.hotkey.clone());
        }
        if parts.is_empty() {
            "트레이 아이콘 클릭".into()
        } else {
            parts.join(" 또는 ")
        }
    }

    /// 이 프로그램 이름(예: "KeePass.exe" 또는 "keepass")이 제외 목록에 있는지.
    pub fn is_excluded(&self, process_name: &str) -> bool {
        app_in_list(&self.excluded_apps, process_name)
    }

    /// Ctrl+V 대신 Shift+Insert 로 붙여넣어야 하는 프로그램인지(일부 터미널).
    pub fn uses_shift_insert(&self, process_name: &str) -> bool {
        app_in_list(&self.shift_insert_apps, process_name)
    }
}

/// 처음 실행할 때 만들어지는 설명이 달린 설정 파일.
pub const DEFAULT_CONFIG_TOML: &str = r#"# ClipStash 설정 파일
# 저장하면 다음에 팝업을 열 때 자동으로 다시 읽습니다.
# (트레이 메뉴 → "설정 다시 불러오기" 로 바로 적용할 수도 있습니다)

# Ctrl+C 를 빠르게 두 번 누르면 기록 팝업 열기(한 번 누르면 평소처럼 복사만).
# 두 번 누르는 사이 허용 시간(밀리초). 0 이면 끔.
double_copy_ms = 400

# 기록 팝업을 여는 추가 단축키. 예: "Ctrl+Shift+V", "Alt+C", "Ctrl+`", "F9", 끄려면 ""
hotkey = "Ctrl+Shift+V"

# 보관할 기록 개수(고정한 항목은 개수에 포함되지 않고 지워지지도 않습니다)
max_items = 300

# 어떤 종류를 기록할지
capture_text = true
capture_images = true
capture_files = true

# 이보다 큰 텍스트(KB)/이미지(MB)는 기록하지 않습니다
max_text_kb = 1024
max_image_mb = 30

# 웹페이지 등에서 복사한 HTML 서식도 함께 저장해서, 서식 그대로 붙여넣기
keep_html = true

# 항목을 고르면 원래 창에 바로 붙여넣기(false 면 클립보드에만 복사)
paste_after_select = true

# 기본 붙여넣기를 항상 서식 없는 텍스트로
plain_text_by_default = false

# 붙여넣은 항목을 목록 맨 위로 올리기
move_to_top_on_paste = true

# 공백만 있는 텍스트는 기록하지 않기
ignore_whitespace_only = true

# 팝업 위치: "caret"(입력 커서 옆), "mouse"(마우스 위치), "center"(화면 가운데)
popup_position = "caret"
popup_width = 480
popup_height = 540
font_name = "맑은 고딕"
font_size = 10

# 이 프로그램에서 복사한 내용은 기록하지 않습니다(비밀번호 관리자 등).
# 프로그램 실행 파일 이름을 적으세요. 대소문자 무시, .exe 는 생략 가능.
# 참고: 비밀번호 관리자가 "기록하지 말라"는 표시를 붙인 복사는 목록과 상관없이 항상 무시합니다.
excluded_apps = [
    "keepass.exe",
    "keepassxc.exe",
    "1password.exe",
    "bitwarden.exe",
    "enpass.exe",
    "lastpass.exe",
    "dashlane.exe",
]

# Ctrl+V 대신 Shift+Insert 로 붙여넣을 프로그램(일부 터미널 프로그램)
shift_insert_apps = ["putty.exe", "mintty.exe", "kitty.exe"]

# ── 템플릿(상용구) ─────────────────────────────────────────────
# 팝업에서 Tab 키로 "템플릿" 탭으로 가서 고를 수 있습니다.
# 쓸 수 있는 자리표시자:
#   {date} {time} {datetime} {weekday}
#   {date:yyyy.MM.dd} {time:HH시 mm분}   (yyyy yy MM M dd d HH H mm ss E=요일)
#   {clipboard}  지금 클립보드의 텍스트
#   {cursor}     붙여넣은 뒤 커서가 갈 자리
#   {{ }}        중괄호 문자 그대로
# 여러 줄은 '''...''' 로 쓰면 됩니다.

[[templates]]
name = "오늘 날짜"
group = "기본"
text = "{date:yyyy년 M월 d일 (E)}"

[[templates]]
name = "현재 시각"
group = "기본"
text = "{time}"

[[templates]]
name = "메일 맺음말"
group = "업무"
text = '''
{cursor}

감사합니다.
'''
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_file_parses_and_matches_defaults() {
        let c = Config::from_toml(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(c, Config::default());
        assert!(crate::hotkey::parse(&c.hotkey).is_ok());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let c = Config::from_toml("max_items = 5\npopup_position = \"mouse\"").unwrap();
        assert_eq!(c.max_items, 10); // 최소값으로 보정
        assert_eq!(c.popup_position, PopupPosition::Mouse);
        assert_eq!(c.hotkey, "Ctrl+Shift+V");
    }

    #[test]
    fn shortcut_labels() {
        let mut c = Config::default();
        assert_eq!(c.shortcut_label(), "Ctrl+C 두 번 또는 Ctrl+Shift+V");
        c.hotkey = String::new();
        assert_eq!(c.shortcut_label(), "Ctrl+C 두 번");
        c.double_copy_ms = 0;
        assert_eq!(c.shortcut_label(), "트레이 아이콘 클릭");
        let c = Config::from_toml("double_copy_ms = 50").unwrap();
        assert_eq!(c.double_copy_ms, 150);
    }

    #[test]
    fn bad_toml_is_error() {
        assert!(Config::from_toml("max_items = \"many\"").is_err());
    }

    #[test]
    fn exclusion_matching() {
        let c = Config::default();
        assert!(c.is_excluded("KeePass.exe"));
        assert!(c.is_excluded("C:\\Program Files\\KeePassXC\\KeePassXC.exe"));
        assert!(c.is_excluded("bitwarden"));
        assert!(!c.is_excluded("notepad.exe"));
        assert!(!c.is_excluded(""));
        assert!(c.uses_shift_insert("PuTTY.exe"));
    }
}
