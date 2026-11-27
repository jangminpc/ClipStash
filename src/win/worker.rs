//! 오래 걸리는 일(글자 추출, 인터넷 요청)을 별도 스레드에서 하고 결과를 메인 창에 알립니다.

use std::sync::Mutex;

use clipstash::kakao::Token;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const WM_APP_WORKER: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 10;

pub enum Done {
    /// 이미지에서 뽑은 글자. (글자, 쓴 방법 설명) 또는 실패 이유
    Ocr(Result<(String, String), String>),
    /// 카카오 로그인 결과
    KakaoAuth(Result<Token, String>),
    /// 카카오톡 보내기 결과(보낸 메시지 수). 토큰이 갱신되었으면 함께 돌려줌
    KakaoSent { result: Result<usize, String>, token: Option<Token>, truncated: bool },
}

static DONE: Mutex<Vec<Done>> = Mutex::new(Vec::new());

pub fn spawn(main: HWND, job: impl FnOnce() -> Done + Send + 'static) {
    let target = main as isize;
    std::thread::spawn(move || {
        let d = job();
        if let Ok(mut q) = DONE.lock() {
            q.push(d);
        }
        unsafe { PostMessageW(target as HWND, WM_APP_WORKER, 0, 0) };
    });
}

pub fn take() -> Vec<Done> {
    DONE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}
