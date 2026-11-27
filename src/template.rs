//! 템플릿(상용구) 치환.
//!
//! 지원하는 자리표시자:
//! - `{date}` → 2026-11-24, `{time}` → 15:35, `{datetime}` → 2026-11-24 15:35
//! - `{date:yyyy.MM.dd}`, `{time:HH시 mm분}` 처럼 형식 지정 가능
//!   (yyyy, yy, MM, M, dd, d, HH, H, mm, ss, E=요일)
//! - `{weekday}` → 화
//! - `{clipboard}` → 지금 클립보드에 있는 텍스트
//! - `{cursor}` → 붙여넣은 뒤 커서를 이 위치로 옮김
//! - `{{`, `}}` → 중괄호 문자 그대로

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Template {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// 0 = 일요일
    pub weekday: u8,
}

pub struct Context<'a> {
    pub now: LocalTime,
    pub clipboard: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    /// `{cursor}` 가 있으면, 붙여넣은 뒤 왼쪽 화살표를 몇 번 눌러야 하는지(문자 수)
    pub cursor_back: Option<usize>,
}

const WEEKDAYS: [&str; 7] = ["일", "월", "화", "수", "목", "금", "토"];

pub fn format_time(t: &LocalTime, fmt: &str) -> String {
    let tokens: [(&str, String); 11] = [
        ("yyyy", format!("{:04}", t.year)),
        ("yy", format!("{:02}", t.year % 100)),
        ("MM", format!("{:02}", t.month)),
        ("M", t.month.to_string()),
        ("dd", format!("{:02}", t.day)),
        ("d", t.day.to_string()),
        ("HH", format!("{:02}", t.hour)),
        ("H", t.hour.to_string()),
        ("mm", format!("{:02}", t.minute)),
        ("ss", format!("{:02}", t.second)),
        ("E", WEEKDAYS[(t.weekday % 7) as usize].to_string()),
    ];
    let mut out = String::new();
    let mut rest = fmt;
    'outer: while !rest.is_empty() {
        for (tok, val) in &tokens {
            if let Some(r) = rest.strip_prefix(tok) {
                out.push_str(val);
                rest = r;
                continue 'outer;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

pub fn expand(template: &str, ctx: &Context) -> Expanded {
    let mut out = String::new();
    let mut cursor_at: Option<usize> = None; // out 안의 바이트 위치
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("{{") {
            out.push('{');
            rest = &rest[2..];
            continue;
        }
        if rest.starts_with("}}") {
            out.push('}');
            rest = &rest[2..];
            continue;
        }
        if c == '{' {
            if let Some(end) = rest.find('}') {
                let inner = &rest[1..end];
                let (name, arg) = match inner.split_once(':') {
                    Some((n, a)) => (n, Some(a)),
                    None => (inner, None),
                };
                let replacement = match (name, arg) {
                    ("date", None) => Some(format_time(&ctx.now, "yyyy-MM-dd")),
                    ("time", None) => Some(format_time(&ctx.now, "HH:mm")),
                    ("datetime", None) => Some(format_time(&ctx.now, "yyyy-MM-dd HH:mm")),
                    ("date" | "time" | "datetime", Some(f)) => Some(format_time(&ctx.now, f)),
                    ("weekday", None) => Some(format_time(&ctx.now, "E")),
                    ("clipboard", None) => Some(ctx.clipboard.unwrap_or("").to_string()),
                    ("cursor", None) => {
                        if cursor_at.is_none() {
                            cursor_at = Some(out.len());
                        }
                        Some(String::new())
                    }
                    _ => None,
                };
                if let Some(r) = replacement {
                    out.push_str(&r);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    // 붙여넣을 때 줄바꿈은 왼쪽 화살표 한 번에 넘어가므로 한 글자로 셉니다.
    let cursor_back = cursor_at.map(|pos| out[pos..].replace("\r\n", "\n").chars().count());
    Expanded { text: out, cursor_back }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context<'static> {
        Context {
            now: LocalTime { year: 2026, month: 11, day: 4, hour: 9, minute: 5, second: 7, weekday: 2 },
            clipboard: Some("CLIP"),
        }
    }

    #[test]
    fn basic_placeholders() {
        let e = expand("{date} {time} {weekday} [{clipboard}]", &ctx());
        assert_eq!(e.text, "2026-11-04 09:05 화 [CLIP]");
        assert_eq!(e.cursor_back, None);
    }

    #[test]
    fn custom_format() {
        assert_eq!(expand("{date:yyyy.M.d (E)}", &ctx()).text, "2026.11.4 (화)");
        assert_eq!(expand("{time:HH시 mm분 ss초}", &ctx()).text, "09시 05분 07초");
    }

    #[test]
    fn cursor_and_escapes() {
        let e = expand("<b>{cursor}</b> {{x}}", &ctx());
        assert_eq!(e.text, "<b></b> {x}");
        assert_eq!(e.cursor_back, Some("</b> {x}".chars().count()));
        let e = expand("안녕{cursor}\r\n하세요", &ctx());
        assert_eq!(e.cursor_back, Some(4));
    }

    #[test]
    fn unknown_left_alone() {
        assert_eq!(expand("{nope} {unterminated", &ctx()).text, "{nope} {unterminated");
    }
}
