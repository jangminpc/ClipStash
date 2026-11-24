//! 백업 파일(내보내기/가져오기)과 동기화 폴더에 쓰는 파일 형식.
//!
//! - 백업 파일: 기록 + 템플릿 + 이미지(Base64)를 JSON 파일 하나에 담습니다.
//! - 동기화 폴더: PC 마다 `ClipStash-<PC이름>.json` 을 쓰고(이미지는 `images/` 폴더에 따로),
//!   다른 PC 의 파일을 읽어서 합칩니다.

use crate::history::{ClipContent, ClipItem};
use crate::template::Template;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const APP_TAG: &str = "ClipStash";
pub const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Backup {
    pub app: String,
    pub version: u32,
    /// 만든 시각(UNIX 초)
    pub exported: i64,
    #[serde(default)]
    pub computer: String,
    pub items: Vec<ClipItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstones: Vec<String>,
    /// 이미지 파일 이름 → Base64 BMP (백업 파일에만, 동기화 파일은 비어 있음)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub images: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Template>,
}

impl Backup {
    pub fn new(computer: &str, exported: i64, items: Vec<ClipItem>) -> Backup {
        Backup {
            app: APP_TAG.into(),
            version: VERSION,
            exported,
            computer: computer.into(),
            items,
            ..Default::default()
        }
    }

    pub fn parse(data: &[u8]) -> Result<Backup, String> {
        let b: Backup = serde_json::from_slice(data).map_err(|e| format!("파일을 읽을 수 없습니다: {}", e))?;
        if b.app != APP_TAG {
            return Err("ClipStash 백업 파일이 아닙니다.".into());
        }
        Ok(b)
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// 다른 곳에서 온 이미지 파일 이름이 안전한지(폴더 밖으로 나가지 않는지)
pub fn safe_image_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.ends_with(".bmp")
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// 항목이 쓰는 이미지 파일 이름(이미지가 아니면 None)
pub fn image_file(item: &ClipItem) -> Option<&str> {
    match &item.content {
        ClipContent::Image { file, .. } => Some(file),
        _ => None,
    }
}

/// 동기화 폴더 안의 이 PC 파일 이름. PC 이름에 이상한 글자가 있으면 바꿉니다.
pub fn sync_file_name(computer: &str) -> String {
    let clean: String = computer
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(60)
        .collect();
    format!("ClipStash-{}.json", if clean.is_empty() { "PC".into() } else { clean })
}

/// 가져온 템플릿 중 아직 없는 것(이름과 내용이 모두 같으면 같은 것으로 봄)
pub fn new_templates(existing: &[Template], incoming: &[Template]) -> Vec<Template> {
    let mut out: Vec<Template> = Vec::new();
    for t in incoming {
        let dup = existing.iter().chain(out.iter()).any(|e| e.name == t.name && e.text == t.text);
        if !dup {
            out.push(t.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;

    #[test]
    fn roundtrip_and_validation() {
        let mut h = History::new();
        h.add(ClipContent::text("백업"), "h1".into(), Some("notepad.exe".into()), 5, 10);
        let mut b = Backup::new("MY-PC", 100, h.items.clone());
        b.images.insert("a.bmp".into(), crate::b64::encode(b"BM"));
        b.templates.push(Template { name: "t".into(), group: None, text: "x".into() });
        let back = Backup::parse(&b.to_json()).unwrap();
        assert_eq!(back.items, h.items);
        assert_eq!(back.templates.len(), 1);
        assert_eq!(back.images.len(), 1);
        assert!(Backup::parse(b"{\"app\":\"Other\",\"version\":1,\"exported\":0,\"items\":[]}").is_err());
        assert!(Backup::parse(b"nope").is_err());
    }

    #[test]
    fn image_names() {
        assert!(safe_image_name("0123abcd.bmp"));
        assert!(!safe_image_name("..\\..\\evil.bmp"));
        assert!(!safe_image_name("a/b.bmp"));
        assert!(!safe_image_name("x.exe"));
        assert!(!safe_image_name(".bmp"));
    }

    #[test]
    fn sync_names_and_templates() {
        assert_eq!(sync_file_name("DESKTOP-1"), "ClipStash-DESKTOP-1.json");
        assert_eq!(sync_file_name("a b/c"), "ClipStash-a_b_c.json");
        assert_eq!(sync_file_name(""), "ClipStash-PC.json");
        let t = |n: &str, x: &str| Template { name: n.into(), group: None, text: x.into() };
        let got = new_templates(&[t("a", "1")], &[t("a", "1"), t("a", "2"), t("b", "1"), t("b", "1")]);
        assert_eq!(got, vec![t("a", "2"), t("b", "1")]);
    }
}
