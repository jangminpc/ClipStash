//! ClipStash 핵심 로직.
//!
//! 이 크레이트의 `lib` 부분은 운영체제와 무관한 순수 로직만 담고 있어서
//! Linux/macOS 에서도 `cargo test` 로 검증할 수 있습니다.
//! Windows 전용 코드(클립보드 감시, 팝업 창, 트레이 아이콘 등)는 `src/win/` 에 있습니다.

pub mod ai_ocr;
pub mod b64;
pub mod backup;
pub mod bmp;
pub mod config;
pub mod hash;
pub mod history;
pub mod hotkey;
pub mod kakao;
pub mod search;
pub mod storage;
pub mod template;
pub mod transform;
