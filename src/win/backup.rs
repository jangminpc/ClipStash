//! 백업 파일 내보내기/가져오기, 동기화 폴더로 PC 간 기록 맞추기.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clipstash::backup::{self, Backup};
use clipstash::history::ClipItem;
use clipstash::{b64, storage};
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::{self, wide};
use super::App;

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into())
}

/// 파일 저장/열기 대화상자. 고른 경로 또는 None(취소).
fn file_dialog(owner: windows_sys::Win32::Foundation::HWND, save: bool, default_name: &str) -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    let name: Vec<u16> = default_name.encode_utf16().take(buf.len() - 1).collect();
    buf[..name.len()].copy_from_slice(&name);
    let filter: Vec<u16> = "ClipStash 백업 (*.json)\0*.json\0모든 파일\0*.*\0\0".encode_utf16().collect();
    let ext = wide("json");
    let title = wide(if save { "기록 백업 파일로 내보내기" } else { "백업 파일에서 가져오기" });
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrDefExt = ext.as_ptr();
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST | if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST };
    let ok = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(owner);
        if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) }
    };
    (ok != 0).then(|| PathBuf::from(util::from_wide(&buf)))
}

impl App {
    pub fn export_backup(&mut self) {
        let t = util::local_time();
        let name = format!("ClipStash-백업-{:04}-{:02}-{:02}.json", t.year, t.month, t.day);
        let Some(path) = file_dialog(self.main, true, &name) else { return };
        let mut b = Backup::new(&computer_name(), util::now_unix(), self.history.items.clone());
        for item in &self.history.items {
            if let Some(f) = backup::image_file(item) {
                if let Ok(data) = std::fs::read(storage::image_path(&self.data_dir, f)) {
                    b.images.insert(f.to_string(), b64::encode(&data));
                }
            }
        }
        b.templates = self.config.templates.clone();
        match storage::write_atomic(&path, &b.to_json()) {
            Ok(()) => self.balloon(
                "백업 파일을 만들었습니다",
                &format!("기록 {}개, 템플릿 {}개\n{}", b.items.len(), b.templates.len(), path.display()),
                false,
            ),
            Err(e) => self.balloon("백업 파일을 만들지 못했습니다", &e.to_string(), true),
        }
    }

    pub fn import_backup(&mut self) {
        let Some(path) = file_dialog(self.main, false, "") else { return };
        let b = match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| Backup::parse(&d)) {
            Ok(b) => b,
            Err(e) => {
                util::message_box(self.main, &format!("가져오지 못했습니다.\n\n{}", e), MB_ICONERROR);
                return;
            }
        };
        // 이미지 파일부터 풀어 놓고, 이미지가 없는 이미지 항목은 건너뜁니다.
        for (name, data) in &b.images {
            if !backup::safe_image_name(name) {
                continue;
            }
            let p = storage::image_path(&self.data_dir, name);
            if !p.exists() {
                if let Some(bytes) = b64::decode(data) {
                    let _ = storage::write_atomic(&p, &bytes);
                }
            }
        }
        let items = self.with_images(&b.items, None);
        let out = self.history.merge(&items, &[], false, self.config.max_items);
        storage::delete_item_files(&self.data_dir, &out.removed);
        let new_t = backup::new_templates(&self.config.templates, &b.templates);
        let mut t_count = new_t.len();
        if !new_t.is_empty() {
            if let Err(e) = self.append_templates(&new_t) {
                self.balloon("템플릿을 가져오지 못했습니다", &e, true);
                t_count = 0;
            }
        }
        self.mark_dirty();
        super::popup::on_history_changed(self);
        util::message_box(
            self.main,
            &format!(
                "가져오기 완료\n\n새 기록 {}개 (이미 있던 {}개는 건너뜀)\n새 템플릿 {}개",
                out.added,
                b.items.len().saturating_sub(out.added),
                t_count
            ),
            MB_ICONINFORMATION,
        );
    }

    /// 이미지 파일이 실제로 있는 항목만(`from` 이 있으면 거기서 복사해 옴).
    fn with_images(&self, items: &[ClipItem], from: Option<&Path>) -> Vec<ClipItem> {
        items
            .iter()
            .filter(|i| match backup::image_file(i) {
                None => true,
                Some(f) if !backup::safe_image_name(f) => false,
                Some(f) => {
                    let local = storage::image_path(&self.data_dir, f);
                    if local.exists() {
                        return true;
                    }
                    match from {
                        Some(dir) => {
                            let src = dir.join(f);
                            std::fs::create_dir_all(local.parent().unwrap_or(&self.data_dir)).is_ok()
                                && std::fs::copy(&src, &local).is_ok()
                        }
                        None => false,
                    }
                }
            })
            .cloned()
            .collect()
    }

    fn sync_dir(&self) -> Option<PathBuf> {
        let s = self.config.sync_folder.trim();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }

    /// 이 PC 의 기록을 동기화 폴더에 씁니다(저장할 때마다).
    pub fn sync_push(&mut self) {
        let Some(dir) = self.sync_dir() else { return };
        let img_dir = dir.join(storage::IMAGE_DIR);
        if std::fs::create_dir_all(&img_dir).is_err() {
            if !self.sync_warned {
                self.sync_warned = true;
                self.balloon("동기화 폴더를 쓸 수 없습니다", &dir.display().to_string(), true);
            }
            return;
        }
        for item in &self.history.items {
            if let Some(f) = backup::image_file(item) {
                let dst = img_dir.join(f);
                if !dst.exists() {
                    let _ = std::fs::copy(storage::image_path(&self.data_dir, f), &dst);
                }
            }
        }
        let mut b = Backup::new(&computer_name(), util::now_unix(), self.history.items.clone());
        b.tombstones = self.history.tombstones.clone();
        let path = dir.join(backup::sync_file_name(&computer_name()));
        if storage::write_atomic(&path, &b.to_json()).is_ok() {
            // 자기 파일은 다시 읽지 않도록 기억
            if let Ok(t) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                self.sync_seen.insert(path, t);
            }
        }
    }

    /// 다른 PC 들이 동기화 폴더에 쓴 기록을 합칩니다(1분마다, 팝업을 열 때).
    pub fn sync_pull(&mut self) {
        let Some(dir) = self.sync_dir() else { return };
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        let mine = backup::sync_file_name(&computer_name());
        let mut changed = false;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == mine || !name.starts_with("ClipStash-") || !name.ends_with(".json") {
                continue;
            }
            let path = e.path();
            let Ok(mtime) = e.metadata().and_then(|m| m.modified()) else { continue };
            if self.sync_seen.get(&path) == Some(&mtime) {
                continue;
            }
            self.sync_seen.insert(path.clone(), mtime);
            let Ok(b) = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| Backup::parse(&d)) else {
                continue;
            };
            let items = self.with_images(&b.items, Some(&dir.join(storage::IMAGE_DIR)));
            let out = self.history.merge(&items, &b.tombstones, true, self.config.max_items);
            if out.changed {
                storage::delete_item_files(&self.data_dir, &out.removed);
                changed = true;
            }
        }
        if changed {
            self.mark_dirty();
            super::popup::on_history_changed(self);
        }
    }
}

pub type SyncSeen = HashMap<PathBuf, SystemTime>;
