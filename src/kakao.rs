//! 카카오톡 "나에게 보내기"(카카오 REST API) 에 필요한 순수 로직.
//! 실제 HTTPS 요청은 Windows 쪽(WinHTTP)에서 합니다.
//!
//! 흐름: 브라우저로 카카오 로그인 → `http://localhost:<포트>` 로 돌아온 인가 코드를 받아
//! 토큰으로 바꿈 → 토큰으로 `/v2/api/talk/memo/default/send` 호출.
//! 액세스 토큰은 몇 시간 뒤 만료되므로 리프레시 토큰(약 두 달)으로 자동 갱신합니다.

use serde::{Deserialize, Serialize};

pub const AUTH_HOST: &str = "kauth.kakao.com";
pub const API_HOST: &str = "kapi.kakao.com";
pub const TOKEN_PATH: &str = "/oauth/token";
pub const MEMO_PATH: &str = "/v2/api/talk/memo/default/send";
/// 카카오 텍스트 메시지 한 개에 넣을 수 있는 최대 글자 수
pub const MAX_TEXT: usize = 200;
/// 긴 글을 나눠 보낼 때 최대 메시지 개수
pub const MAX_MESSAGES: usize = 10;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Token {
    pub access_token: String,
    /// 액세스 토큰 만료 시각(UNIX 초)
    pub expires_at: i64,
    pub refresh_token: String,
    /// 리프레시 토큰 만료 시각(UNIX 초, 모르면 0)
    #[serde(default)]
    pub refresh_expires_at: i64,
}

impl Token {
    pub fn access_valid(&self, now: i64) -> bool {
        !self.access_token.is_empty() && now + 60 < self.expires_at
    }
    pub fn refresh_valid(&self, now: i64) -> bool {
        !self.refresh_token.is_empty() && (self.refresh_expires_at == 0 || now < self.refresh_expires_at)
    }
}

pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{}", port)
}

/// application/x-www-form-urlencoded 인코딩
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or(()) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", k, url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 브라우저로 열 카카오 로그인 주소
pub fn authorize_url(rest_key: &str, port: u16) -> String {
    format!(
        "https://{}/oauth/authorize?{}",
        AUTH_HOST,
        form(&[
            ("response_type", "code"),
            ("client_id", rest_key),
            ("redirect_uri", &redirect_uri(port)),
            ("scope", "talk_message"),
        ])
    )
}

pub fn code_exchange_body(rest_key: &str, secret: &str, port: u16, code: &str) -> String {
    form(&[
        ("grant_type", "authorization_code"),
        ("client_id", rest_key),
        ("client_secret", secret),
        ("redirect_uri", &redirect_uri(port)),
        ("code", code),
    ])
}

pub fn refresh_body(rest_key: &str, secret: &str, refresh_token: &str) -> String {
    form(&[
        ("grant_type", "refresh_token"),
        ("client_id", rest_key),
        ("client_secret", secret),
        ("refresh_token", refresh_token),
    ])
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<i64>,
    error: Option<String>,
    error_description: Option<String>,
}

/// 토큰 응답을 읽습니다. 갱신 응답에는 리프레시 토큰이 없을 수 있어 이전 값을 유지합니다.
pub fn parse_token(json: &str, now: i64, old: Option<&Token>) -> Result<Token, String> {
    let r: TokenResponse = serde_json::from_str(json).map_err(|_| format!("알 수 없는 응답: {}", short(json)))?;
    if let Some(e) = r.error {
        return Err(format!("{} {}", e, r.error_description.unwrap_or_default()).trim().to_string());
    }
    let access_token = r.access_token.ok_or_else(|| format!("토큰이 없습니다: {}", short(json)))?;
    let mut t = old.cloned().unwrap_or_default();
    t.access_token = access_token;
    t.expires_at = now + r.expires_in.unwrap_or(3600);
    if let Some(rt) = r.refresh_token {
        t.refresh_token = rt;
        t.refresh_expires_at = r.refresh_token_expires_in.map(|s| now + s).unwrap_or(0);
    }
    Ok(t)
}

fn short(s: &str) -> String {
    s.chars().take(200).collect()
}

/// 긴 글을 카카오 메시지 크기에 맞게 나눕니다. (조각들, 잘렸는지)
pub fn split_text(text: &str) -> (Vec<String>, bool) {
    let chars: Vec<char> = text.trim().chars().filter(|c| *c != '\r').collect();
    let mut parts: Vec<String> = chars.chunks(MAX_TEXT).map(|c| c.iter().collect()).collect();
    let truncated = parts.len() > MAX_MESSAGES;
    parts.truncate(MAX_MESSAGES);
    (parts, truncated)
}

/// 텍스트 메시지 보내기 요청 본문
pub fn memo_body(text: &str) -> String {
    let template = serde_json::json!({
        "object_type": "text",
        "text": text,
        "link": { "web_url": "", "mobile_web_url": "" },
    });
    form(&[("template_object", &template.to_string())])
}

/// 응답이 성공인지. 실패면 사람이 읽을 이유.
pub fn check_memo_response(status: u32, body: &str) -> Result<(), String> {
    if status == 200 && body.contains("\"result_code\":0") {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct E {
        msg: Option<String>,
        code: Option<i64>,
    }
    let detail = serde_json::from_str::<E>(body)
        .ok()
        .map(|e| format!("{} (코드 {})", e.msg.unwrap_or_default(), e.code.unwrap_or(0)))
        .unwrap_or_else(|| short(body));
    Err(match status {
        401 => format!("로그인이 만료되었습니다 — 다시 연결해 주세요. {}", detail),
        403 => format!("메시지 보내기 권한이 없습니다(동의항목 talk_message 확인). {}", detail),
        _ => format!("HTTP {} {}", status, detail),
    })
}

/// 로그인 뒤 브라우저가 보낸 요청 첫 줄("GET /?code=... HTTP/1.1")에서 인가 코드를 꺼냅니다.
pub fn code_from_request(request_line: &str) -> Result<String, String> {
    let target = request_line.split_whitespace().nth(1).ok_or("잘못된 요청")?;
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut error = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "code" => code = Some(url_decode(v)),
            "error_description" => error = Some(url_decode(v)),
            "error" if error.is_none() => error = Some(url_decode(v)),
            _ => {}
        }
    }
    match (code, error) {
        (Some(c), _) if !c.is_empty() => Ok(c),
        (_, Some(e)) => Err(e),
        _ => Err("인가 코드가 없습니다".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding() {
        assert_eq!(url_encode("a b&c=한"), "a+b%26c%3D%ED%95%9C");
        assert_eq!(url_decode("a+b%26c%3D%ED%95%9C"), "a b&c=한");
        assert_eq!(url_decode("100%"), "100%");
        let u = authorize_url("KEY", 8765);
        assert!(u.starts_with("https://kauth.kakao.com/oauth/authorize?response_type=code&client_id=KEY"));
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765"));
        assert!(!refresh_body("K", "", "R").contains("client_secret"));
        assert!(refresh_body("K", "S", "R").contains("client_secret=S"));
    }

    #[test]
    fn tokens() {
        let t = parse_token(
            r#"{"access_token":"A","token_type":"bearer","refresh_token":"R","expires_in":21599,"refresh_token_expires_in":5183999}"#,
            1000,
            None,
        )
        .unwrap();
        assert_eq!(t.access_token, "A");
        assert_eq!(t.expires_at, 22599);
        assert!(t.access_valid(1000) && !t.access_valid(22590));
        assert!(t.refresh_valid(5_000_000));
        let t2 = parse_token(r#"{"access_token":"B","expires_in":100}"#, 2000, Some(&t)).unwrap();
        assert_eq!(t2.refresh_token, "R");
        assert_eq!(t2.access_token, "B");
        assert!(parse_token(r#"{"error":"invalid_grant","error_description":"expired"}"#, 0, None)
            .unwrap_err()
            .contains("invalid_grant"));
    }

    #[test]
    fn split_and_body() {
        let (p, cut) = split_text("짧은 글");
        assert_eq!(p, vec!["짧은 글"]);
        assert!(!cut);
        let long: String = "가".repeat(MAX_TEXT * 12);
        let (p, cut) = split_text(&long);
        assert_eq!(p.len(), MAX_MESSAGES);
        assert_eq!(p[0].chars().count(), MAX_TEXT);
        assert!(cut);
        let body = memo_body("hi \"there\"");
        assert!(body.starts_with("template_object=%7B"));
        assert!(check_memo_response(200, r#"{"result_code":0}"#).is_ok());
        assert!(check_memo_response(401, r#"{"msg":"this access token does not exist","code":-401}"#)
            .unwrap_err()
            .contains("만료"));
    }

    #[test]
    fn redirect_parsing() {
        assert_eq!(code_from_request("GET /?code=abc%2Bd HTTP/1.1").unwrap(), "abc+d");
        assert!(code_from_request("GET /?error=access_denied&error_description=User+denied HTTP/1.1")
            .unwrap_err()
            .contains("User denied"));
        assert!(code_from_request("GET /favicon.ico HTTP/1.1").is_err());
    }
}
