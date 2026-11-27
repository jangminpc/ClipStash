//! 카카오톡 "나에게 보내기": 로그인(인가 코드 받기), 토큰 갱신, 메시지 보내기.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clipstash::kakao::{self, Token};

use super::net;

pub const TOKEN_FILE: &str = "kakao_token.json";

pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

pub fn load_token(data_dir: &Path) -> Option<Token> {
    let s = std::fs::read(token_path(data_dir)).ok()?;
    serde_json::from_slice(&s).ok()
}

pub fn save_token(data_dir: &Path, t: &Token) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(t).map_err(std::io::Error::other)?;
    clipstash::storage::write_atomic(&token_path(data_dir), &json)
}

/// 로그인 뒤 돌아올 주소(localhost)를 기다릴 소켓. 브라우저를 열기 전에 미리 만듭니다.
pub struct Listener(Vec<TcpListener>);

pub fn listen(port: u16) -> Result<Listener, String> {
    let mut v = Vec::new();
    for addr in [format!("127.0.0.1:{}", port), format!("[::1]:{}", port)] {
        if let Ok(l) = TcpListener::bind(&addr) {
            let _ = l.set_nonblocking(true);
            v.push(l);
        }
    }
    if v.is_empty() {
        return Err(format!("{} 번 포트를 열 수 없습니다. 설정 파일의 kakao_port 를 바꿔 보세요.", port));
    }
    Ok(Listener(v))
}

fn respond(mut s: &TcpStream, ok: bool, msg: &str) {
    let color = if ok { "#2a7" } else { "#d44" };
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>ClipStash</title>\
         <body style=\"font-family:sans-serif;text-align:center;padding:60px\">\
         <h2 style=\"color:{}\">{}</h2><p>이 창은 닫아도 됩니다.</p></body>",
        color, msg
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = s.write_all(resp.as_bytes());
}

/// 브라우저에서 로그인을 마칠 때까지(최대 5분) 기다렸다가 토큰을 받아 옵니다.
pub fn finish_login(l: Listener, key: &str, secret: &str, port: u16) -> Result<Token, String> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        for listener in &l.0 {
            let Ok((stream, _)) = listener.accept() else { continue };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = vec![0u8; 8192];
            let mut n = 0;
            let mut s = &stream;
            while n < buf.len() {
                match s.read(&mut buf[n..]) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => n += k,
                }
                if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let first = req.lines().next().unwrap_or("");
            // 파비콘 같은 다른 요청은 무시
            if !first.contains("?") {
                respond(&stream, false, "ClipStash 가 로그인을 기다리는 중입니다");
                continue;
            }
            return match kakao::code_from_request(first) {
                Ok(code) => {
                    let body = kakao::code_exchange_body(key, secret, port, &code);
                    let r = net::post_form(kakao::AUTH_HOST, kakao::TOKEN_PATH, None, &body)
                        .and_then(|(_, b)| kakao::parse_token(&b, super::util::now_unix(), None));
                    match &r {
                        Ok(_) => respond(&stream, true, "카카오톡 연결 완료!"),
                        Err(e) => respond(&stream, false, &format!("연결 실패: {}", e)),
                    }
                    r
                }
                Err(e) => {
                    respond(&stream, false, &format!("로그인 실패: {}", e));
                    Err(e)
                }
            };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("5분 안에 로그인이 끝나지 않았습니다. 다시 시도해 주세요.".into())
}

fn refresh(key: &str, secret: &str, t: &Token) -> Result<Token, String> {
    let now = super::util::now_unix();
    if !t.refresh_valid(now) {
        return Err("카카오 로그인이 만료되었습니다. 트레이 메뉴 → 카카오톡 연결로 다시 연결해 주세요.".into());
    }
    let body = kakao::refresh_body(key, secret, &t.refresh_token);
    let (_, b) = net::post_form(kakao::AUTH_HOST, kakao::TOKEN_PATH, None, &body)?;
    kakao::parse_token(&b, now, Some(t))
}

/// 글 조각들을 차례로 보냅니다. (결과, 갱신된 토큰)
pub fn send(key: &str, secret: &str, mut token: Token, parts: &[String]) -> (Result<usize, String>, Option<Token>) {
    let mut changed = false;
    if !token.access_valid(super::util::now_unix()) {
        match refresh(key, secret, &token) {
            Ok(t) => {
                token = t;
                changed = true;
            }
            Err(e) => return (Err(e), None),
        }
    }
    let mut sent = 0;
    let mut retried = false;
    let mut i = 0;
    while i < parts.len() {
        let body = kakao::memo_body(&parts[i]);
        let r = net::post_form(kakao::API_HOST, kakao::MEMO_PATH, Some(&token.access_token), &body)
            .and_then(|(status, b)| {
                if status == 401 {
                    Err("401".to_string())
                } else {
                    kakao::check_memo_response(status, &b)
                }
            });
        match r {
            Ok(()) => {
                sent += 1;
                i += 1;
            }
            Err(e) if e == "401" && !retried => {
                retried = true;
                match refresh(key, secret, &token) {
                    Ok(t) => {
                        token = t;
                        changed = true;
                    }
                    Err(e) => return (Err(e), changed.then_some(token)),
                }
            }
            Err(e) => {
                let e = if e == "401" { kakao::check_memo_response(401, "").unwrap_err() } else { e };
                return (Err(e), changed.then_some(token));
            }
        }
    }
    (Ok(sent), changed.then_some(token))
}
