//! AI 글자 추출: Claude API(이미지 입력)로 사진·캡처 속 글자를 읽습니다.
//! 요청 본문 만들기와 응답 해석만 여기 있고, HTTPS 요청은 Windows 쪽에서 합니다.

use serde_json::{json, Value};

pub const HOST: &str = "api.anthropic.com";
pub const PATH: &str = "/v1/messages";
pub const API_VERSION: &str = "2026-10-21";
/// 거절(안전 분류기)되면 서버가 다른 모델로 다시 시도하게 하는 베타 기능
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// 모델이 받는 최대 해상도(긴 변)
pub const MAX_LONG_EDGE: u32 = 2576;
/// 이미지 한 장 최대 크기(Base64 로 바꾸기 전, 5MB 제한보다 여유 있게)
pub const MAX_IMAGE_BYTES: usize = 3_600_000;

const NO_TEXT: &str = "NO_TEXT";

const SYSTEM: &str = "You are a precise OCR engine for a clipboard manager. \
Transcribe every piece of text visible in the image exactly as written, in its original language \
(Korean, English, numbers, symbols, mixed scripts). Keep the natural reading order and line breaks. \
Join words that are split only because of line wrapping inside one paragraph. \
Render tables as rows with columns separated by a single tab character so they paste into a spreadsheet. \
Keep list markers, numbering, punctuation, URLs, emails and codes exactly; do not translate, correct, summarize or add anything. \
Handwriting and stylized text should be read as best you can. \
Output only the transcribed text with no preamble, labels, code fences or commentary. \
If the image contains no readable text, output exactly NO_TEXT.";

/// 서버 쪽 "거절 시 다른 모델로" 기능을 쓸 수 있는 모델인지
pub fn supports_fallback(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5", "claude-sonnet-5-5"].iter().any(|m| model.starts_with(m))
}

/// Messages API 요청 본문(JSON)
pub fn request_body(model: &str, png_base64: &str) -> String {
    let mut body = json!({
        "model": model,
        "max_tokens": 16000,
        "output_config": { "effort": "medium" },
        "system": SYSTEM,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": png_base64 } },
                { "type": "text", "text": "Transcribe all text in this image." }
            ]
        }]
    });
    if supports_fallback(model) {
        body["fallbacks"] = json!("default");
    }
    body.to_string()
}

pub fn headers(api_key: &str, model: &str) -> Vec<String> {
    let mut h = vec![
        "Content-Type: application/json".to_string(),
        format!("x-api-key: {}", api_key.trim()),
        format!("anthropic-version: {}", API_VERSION),
    ];
    if supports_fallback(model) {
        h.push(format!("anthropic-beta: {}", FALLBACK_BETA));
    }
    h
}

/// 응답을 읽어 글자만 돌려줍니다. 실패는 사람이 읽을 이유.
pub fn parse_response(status: u32, body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| format!("HTTP {} — 알 수 없는 응답", status))?;
    if status != 200 {
        let msg = v["error"]["message"].as_str().unwrap_or("").to_string();
        return Err(match status {
            401 => "API 키가 올바르지 않습니다(anthropic_api_key 확인).".into(),
            402 | 403 => format!("API 사용 권한/잔액을 확인해 주세요. {}", msg),
            429 => "요청이 너무 많습니다. 잠시 뒤 다시 해 보세요.".into(),
            500..=599 => "AI 서버가 바쁩니다. 잠시 뒤 다시 해 보세요.".into(),
            _ => format!("HTTP {} {}", status, msg),
        });
    }
    if v["stop_reason"].as_str() == Some("refusal") {
        return Err("AI 가 이 이미지 처리를 거절했습니다.".into());
    }
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let text = text.trim().trim_matches('`').trim().to_string();
    if text.is_empty() || text == NO_TEXT {
        return Err("이미지에서 글자를 찾지 못했습니다.".into());
    }
    Ok(text)
}

/// 긴 변이 `MAX_LONG_EDGE` 를 넘지 않게 줄일 배율(늘리지는 않음)
pub fn fit_scale(w: u32, h: u32) -> f64 {
    let long = w.max(h).max(1) as f64;
    (MAX_LONG_EDGE as f64 / long).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_shape() {
        let b: Value = serde_json::from_str(&request_body("claude-opus-5-5", "QUJD")).unwrap();
        assert_eq!(b["model"], "claude-opus-5-5");
        assert_eq!(b["messages"][0]["content"][0]["source"]["data"], "QUJD");
        assert_eq!(b["messages"][0]["content"][0]["source"]["media_type"], "image/png");
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("thinking").is_none());
        assert!(headers(" k ", "claude-opus-5-5").contains(&"x-api-key: k".to_string()));
        assert_eq!(headers("k", "claude-opus-5-5").len(), 4);
        let h: Value = serde_json::from_str(&request_body("claude-haiku-5-5", "QUJD")).unwrap();
        assert!(h.get("fallbacks").is_none());
        assert_eq!(headers("k", "claude-haiku-5-5").len(), 3);
    }

    #[test]
    fn parsing() {
        let ok = r#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"안녕하세요\n2026년"}],"stop_reason":"end_turn"}"#;
        assert_eq!(parse_response(200, ok).unwrap(), "안녕하세요\n2026년");
        let none = r#"{"content":[{"type":"text","text":"NO_TEXT"}],"stop_reason":"end_turn"}"#;
        assert!(parse_response(200, none).is_err());
        let refused = r#"{"content":[],"stop_reason":"refusal"}"#;
        assert!(parse_response(200, refused).unwrap_err().contains("거절"));
        let bad_key = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert!(parse_response(401, bad_key).unwrap_err().contains("API 키"));
        assert!(parse_response(200, "nope").is_err());
    }

    #[test]
    fn scaling() {
        assert_eq!(fit_scale(800, 600), 1.0);
        assert!((fit_scale(5152, 1000) - 0.5).abs() < 1e-9);
    }
}
