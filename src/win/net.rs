//! 아주 작은 HTTPS POST(WinHTTP). 카카오 API 호출에만 씁니다.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::*;

use super::util::wide;

struct H(*mut core::ffi::c_void);
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    let hint = match code {
        12007 => " (서버 이름을 찾을 수 없음 — 인터넷 연결 확인)",
        12002 => " (시간 초과)",
        12029 | 12030 => " (연결 실패)",
        12175 | 12045 => " (보안 연결 오류)",
        _ => "",
    };
    format!("{} 실패: 오류 {}{}", what, code, hint)
}

/// `https://{host}{path}` 로 폼 데이터를 POST 하고 (상태 코드, 응답 본문)을 돌려줍니다.
pub fn post_form(host: &str, path: &str, bearer: Option<&str>, body: &str) -> Result<(u32, String), String> {
    unsafe {
        let agent = wide("ClipStash");
        let session = H(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if session.0.is_null() {
            // Windows 8.1 이전은 AUTOMATIC_PROXY 를 모릅니다.
            return Err(last_error("인터넷 준비"));
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 20_000, 20_000);
        let h = wide(host);
        let conn = H(WinHttpConnect(session.0, h.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if conn.0.is_null() {
            return Err(last_error("서버 연결"));
        }
        let verb = wide("POST");
        let p = wide(path);
        let req = H(WinHttpOpenRequest(conn.0, verb.as_ptr(), p.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE));
        if req.0.is_null() {
            return Err(last_error("요청 준비"));
        }
        let mut headers = String::from("Content-Type: application/x-www-form-urlencoded;charset=utf-8\r\n");
        if let Some(t) = bearer {
            headers.push_str(&format!("Authorization: Bearer {}\r\n", t));
        }
        let hw: Vec<u16> = headers.encode_utf16().collect();
        let data = body.as_bytes();
        if WinHttpSendRequest(
            req.0,
            hw.as_ptr(),
            hw.len() as u32,
            data.as_ptr() as *const _,
            data.len() as u32,
            data.len() as u32,
            0,
        ) == 0
        {
            return Err(last_error("요청 보내기"));
        }
        if WinHttpReceiveResponse(req.0, null_mut()) == 0 {
            return Err(last_error("응답 받기"));
        }
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut _,
            &mut len,
            null_mut(),
        );
        let mut out = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read) == 0 || read == 0 {
                break;
            }
            out.extend_from_slice(&buf[..read as usize]);
            if out.len() > 1 << 20 {
                break;
            }
        }
        Ok((status, String::from_utf8_lossy(&out).into_owned()))
    }
}
