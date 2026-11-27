//! 기록을 JSON 파일로 저장/불러오기. 저장은 임시 파일에 쓴 뒤 이름을 바꿔서
//! 도중에 꺼져도 파일이 깨지지 않게 합니다.

use crate::history::{ClipContent, ClipItem, History};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const HISTORY_FILE: &str = "history.json";
pub const CONFIG_FILE: &str = "config.toml";
pub const IMAGE_DIR: &str = "images";

pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

pub fn save_history(dir: &Path, h: &History) -> io::Result<()> {
    let json = serde_json::to_vec(h).map_err(io::Error::other)?;
    write_atomic(&dir.join(HISTORY_FILE), &json)
}

/// 파일이 없으면 빈 기록. 손상되었으면 `.broken` 으로 옮겨 두고 빈 기록으로 시작합니다.
pub fn load_history(dir: &Path) -> History {
    let path = dir.join(HISTORY_FILE);
    let Ok(data) = fs::read(&path) else {
        return History::new();
    };
    match serde_json::from_slice::<History>(&data) {
        Ok(mut h) => {
            // 이미지 파일이 사라진 항목은 버립니다.
            h.items.retain(|i| match &i.content {
                ClipContent::Image { file, .. } => image_path(dir, file).exists(),
                _ => true,
            });
            h
        }
        Err(_) => {
            let _ = fs::rename(&path, path.with_extension("json.broken"));
            History::new()
        }
    }
}

pub fn image_path(dir: &Path, file: &str) -> PathBuf {
    dir.join(IMAGE_DIR).join(file)
}

/// 지워진 항목의 이미지 파일을 정리합니다.
pub fn delete_item_files(dir: &Path, items: &[ClipItem]) {
    for i in items {
        if let ClipContent::Image { file, .. } = &i.content {
            let _ = fs::remove_file(image_path(dir, file));
        }
    }
}

/// 기록에 없는 이미지 파일(이전 비정상 종료 등으로 남은 것)을 지웁니다.
pub fn remove_orphan_images(dir: &Path, h: &History) {
    let Ok(entries) = fs::read_dir(dir.join(IMAGE_DIR)) else { return };
    let used: std::collections::HashSet<&str> = h
        .items
        .iter()
        .filter_map(|i| match &i.content {
            ClipContent::Image { file, .. } => Some(file.as_str()),
            _ => None,
        })
        .collect();
    for e in entries.flatten() {
        let name = e.file_name();
        if let Some(n) = name.to_str() {
            if !used.contains(n) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::ClipContent;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clipstash-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn roundtrip_and_orphans() {
        let dir = temp_dir("roundtrip");
        let mut h = History::new();
        h.add(ClipContent::text("안녕"), "h1".into(), Some("notepad.exe".into()), 1, 10);
        fs::create_dir_all(dir.join(IMAGE_DIR)).unwrap();
        fs::write(dir.join(IMAGE_DIR).join("1.bmp"), b"x").unwrap();
        fs::write(dir.join(IMAGE_DIR).join("orphan.bmp"), b"x").unwrap();
        h.add(ClipContent::Image { file: "1.bmp".into(), width: 1, height: 1 }, "h2".into(), None, 2, 10);
        h.add(ClipContent::Image { file: "missing.bmp".into(), width: 1, height: 1 }, "h3".into(), None, 3, 10);
        save_history(&dir, &h).unwrap();

        let loaded = load_history(&dir);
        assert_eq!(loaded.items.len(), 2, "missing image dropped");
        assert_eq!(loaded.items[1].source_app.as_deref(), Some("notepad.exe"));
        remove_orphan_images(&dir, &loaded);
        assert!(dir.join(IMAGE_DIR).join("1.bmp").exists());
        assert!(!dir.join(IMAGE_DIR).join("orphan.bmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = temp_dir("corrupt");
        fs::write(dir.join(HISTORY_FILE), b"{not json").unwrap();
        let h = load_history(&dir);
        assert!(h.items.is_empty());
        assert!(dir.join("history.json.broken").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
