//! 클립보드 기록 목록. 최신 항목이 맨 앞(인덱스 0)에 옵니다.

use crate::search::Query;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClipContent {
    /// 일반 텍스트. `html` 은 서식 있는 붙여넣기를 위해 함께 저장되는 원본(있으면).
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html: Option<String>,
    },
    /// 이미지. 데이터 폴더의 `images/` 아래에 BMP 파일로 저장됩니다.
    Image { file: String, width: u32, height: u32 },
    /// 탐색기에서 복사한 파일/폴더 목록.
    Files { paths: Vec<String> },
}

impl ClipContent {
    pub fn text(s: impl Into<String>) -> Self {
        ClipContent::Text { text: s.into(), html: None }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            ClipContent::Text { .. } => "text",
            ClipContent::Image { .. } => "image",
            ClipContent::Files { .. } => "files",
        }
    }

    /// 텍스트로 붙여넣을 때 쓰는 문자열(파일은 경로 목록).
    pub fn as_plain_text(&self) -> Option<String> {
        match self {
            ClipContent::Text { text, .. } => Some(text.clone()),
            ClipContent::Files { paths } => Some(paths.join("\r\n")),
            ClipContent::Image { .. } => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ClipItem {
    pub id: u64,
    pub content: ClipContent,
    pub hash: String,
    /// 처음 복사한 시각(UNIX 초)
    pub created: i64,
    /// 마지막으로 복사/붙여넣기한 시각(UNIX 초)
    pub last_used: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_app: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub use_count: u32,
}

impl ClipItem {
    /// 목록에 보여줄 한 줄 미리보기.
    pub fn preview(&self, max_chars: usize) -> String {
        match &self.content {
            ClipContent::Text { text, .. } => one_line(text, max_chars),
            ClipContent::Image { width, height, .. } => format!("[이미지 {}×{}]", width, height),
            ClipContent::Files { paths } => {
                let names: Vec<&str> = paths.iter().map(|p| file_name(p)).collect();
                let label = if paths.len() == 1 {
                    "[파일]".to_string()
                } else {
                    format!("[파일 {}개]", paths.len())
                };
                one_line(&format!("{} {}", label, names.join(", ")), max_chars)
            }
        }
    }

    /// 검색 대상 문자열.
    pub fn search_text(&self) -> String {
        let mut s = match &self.content {
            ClipContent::Text { text, .. } => text.clone(),
            ClipContent::Image { width, height, .. } => format!("이미지 image {}x{}", width, height),
            ClipContent::Files { paths } => format!("파일 files {}", paths.join(" ")),
        };
        if let Some(app) = &self.source_app {
            s.push(' ');
            s.push_str(app);
        }
        s
    }
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path)
}

/// 줄바꿈/탭을 기호로 바꾸고 앞뒤 공백을 정리해 한 줄로 만듭니다.
pub fn one_line(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut last_space = false;
    for c in text.trim().chars() {
        let mapped = match c {
            '\r' => continue,
            '\n' => '↵',
            '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if mapped == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        if count >= max_chars {
            out.push('…');
            return out;
        }
        out.push(mapped);
        count += 1;
    }
    out
}

/// "방금", "5분 전" 같은 상대 시간.
pub fn relative_time(now: i64, then: i64) -> String {
    let d = (now - then).max(0);
    match d {
        0..=59 => "방금".into(),
        60..=3599 => format!("{}분 전", d / 60),
        3600..=86399 => format!("{}시간 전", d / 3600),
        86400..=2_591_999 => format!("{}일 전", d / 86400),
        2_592_000..=31_535_999 => format!("{}개월 전", d / 2_592_000),
        _ => format!("{}년 전", d / 31_536_000),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    All,
    Pinned,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct History {
    pub items: Vec<ClipItem>,
    #[serde(default)]
    pub next_id: u64,
}

#[derive(Debug, Default)]
pub struct AddOutcome {
    pub id: u64,
    /// 이미 있던 항목을 맨 위로 올렸으면 true
    pub was_duplicate: bool,
    /// 개수 제한 때문에 밀려난 항목(이미지 파일 정리에 사용)
    pub evicted: Vec<ClipItem>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    fn alloc_id(&mut self) -> u64 {
        // 저장 파일이 손상되어 next_id 가 작아도 겹치지 않도록 보정
        let max = self.items.iter().map(|i| i.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(max) + 1;
        self.next_id
    }

    /// 새 항목 추가. 같은 내용이 이미 있으면 새로 만들지 않고 맨 위로 올립니다.
    pub fn add(
        &mut self,
        content: ClipContent,
        hash: String,
        source_app: Option<String>,
        now: i64,
        max_items: usize,
    ) -> AddOutcome {
        if let Some(pos) = self.items.iter().position(|i| i.hash == hash) {
            let mut item = self.items.remove(pos);
            item.last_used = now;
            if source_app.is_some() {
                item.source_app = source_app;
            }
            // HTML 서식이 새로 생겼다면 갱신
            if let (ClipContent::Text { html: old, .. }, ClipContent::Text { html: new, .. }) =
                (&mut item.content, &content)
            {
                if new.is_some() {
                    *old = new.clone();
                }
            }
            let id = item.id;
            self.items.insert(0, item);
            return AddOutcome { id, was_duplicate: true, evicted: self.trim(max_items) };
        }
        let id = self.alloc_id();
        self.items.insert(
            0,
            ClipItem {
                id,
                content,
                hash,
                created: now,
                last_used: now,
                source_app,
                pinned: false,
                use_count: 0,
            },
        );
        AddOutcome { id, was_duplicate: false, evicted: self.trim(max_items) }
    }

    /// 고정되지 않은 항목이 `max_items` 개를 넘으면 오래된 것부터 지웁니다.
    pub fn trim(&mut self, max_items: usize) -> Vec<ClipItem> {
        let mut unpinned_seen = 0;
        let mut evicted = Vec::new();
        let mut kept = Vec::with_capacity(self.items.len());
        for item in self.items.drain(..) {
            if item.pinned {
                kept.push(item);
            } else if unpinned_seen < max_items {
                unpinned_seen += 1;
                kept.push(item);
            } else {
                evicted.push(item);
            }
        }
        self.items = kept;
        evicted
    }

    pub fn get(&self, id: u64) -> Option<&ClipItem> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<ClipItem> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(pos))
    }

    pub fn toggle_pin(&mut self, id: u64) -> Option<bool> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        item.pinned = !item.pinned;
        Some(item.pinned)
    }

    /// 붙여넣기에 사용됨을 기록하고, 원하면 맨 위로 올립니다.
    pub fn mark_used(&mut self, id: u64, now: i64, move_to_top: bool) {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            let item = &mut self.items[pos];
            item.last_used = now;
            item.use_count = item.use_count.saturating_add(1);
            if move_to_top && pos != 0 {
                let item = self.items.remove(pos);
                self.items.insert(0, item);
            }
        }
    }

    /// 고정 항목만 남기고 모두 지웁니다.
    pub fn clear_unpinned(&mut self) -> Vec<ClipItem> {
        let (keep, gone): (Vec<_>, Vec<_>) = self.items.drain(..).partition(|i| i.pinned);
        self.items = keep;
        gone
    }

    /// 여러 항목의 텍스트를 **복사한 순서(오래된 것 → 최근 것)** 로 이어 붙입니다.
    /// 이미지는 건너뛰고, 파일은 경로로 넣습니다. 붙일 것이 없으면 None.
    pub fn merge_text(&self, ids: &[u64], separator: &str) -> Option<String> {
        let parts: Vec<String> = self
            .items
            .iter()
            .rev()
            .filter(|i| ids.contains(&i.id))
            .filter_map(|i| i.content.as_plain_text())
            .collect();
        (!parts.is_empty()).then(|| parts.join(separator))
    }

    pub fn filter(&self, query: &str, view: View) -> Vec<&ClipItem> {
        let q = Query::new(query);
        self.items
            .iter()
            .filter(|i| view == View::All || i.pinned)
            .filter(|i| q.is_empty() || q.matches(&i.search_text()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_text(h: &mut History, s: &str, now: i64, max: usize) -> AddOutcome {
        h.add(ClipContent::text(s), crate::hash::content_hash("text", s.as_bytes()), None, now, max)
    }

    #[test]
    fn newest_first_and_dedup() {
        let mut h = History::new();
        add_text(&mut h, "a", 1, 10);
        add_text(&mut h, "b", 2, 10);
        let out = add_text(&mut h, "a", 3, 10);
        assert!(out.was_duplicate);
        assert_eq!(h.items.len(), 2);
        assert_eq!(h.items[0].content, ClipContent::text("a"));
        assert_eq!(h.items[0].created, 1);
        assert_eq!(h.items[0].last_used, 3);
    }

    #[test]
    fn trim_keeps_pinned() {
        let mut h = History::new();
        let first = add_text(&mut h, "keep me", 0, 3).id;
        h.toggle_pin(first);
        for i in 0..5 {
            add_text(&mut h, &format!("x{}", i), i, 3);
        }
        assert_eq!(h.items.len(), 4);
        assert!(h.get(first).is_some());
        let evicted = add_text(&mut h, "y", 10, 3).evicted;
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].content, ClipContent::text("x2"));
    }

    #[test]
    fn ids_are_unique_even_if_next_id_is_stale() {
        let mut h = History::new();
        add_text(&mut h, "a", 0, 10);
        add_text(&mut h, "b", 0, 10);
        h.next_id = 0;
        let id = add_text(&mut h, "c", 0, 10).id;
        assert_eq!(id, 3);
    }

    #[test]
    fn mark_used_moves_to_top() {
        let mut h = History::new();
        let a = add_text(&mut h, "a", 0, 10).id;
        add_text(&mut h, "b", 0, 10);
        h.mark_used(a, 5, true);
        assert_eq!(h.items[0].id, a);
        assert_eq!(h.items[0].use_count, 1);
    }

    #[test]
    fn filter_by_view_and_query() {
        let mut h = History::new();
        let a = add_text(&mut h, "회의록 정리", 0, 10).id;
        add_text(&mut h, "hello", 0, 10);
        h.toggle_pin(a);
        assert_eq!(h.filter("", View::Pinned).len(), 1);
        assert_eq!(h.filter("ㅎㅇ", View::All).len(), 1);
        assert_eq!(h.filter("HEL", View::All).len(), 1);
        assert_eq!(h.filter("zzz", View::All).len(), 0);
    }

    #[test]
    fn clear_unpinned() {
        let mut h = History::new();
        let a = add_text(&mut h, "a", 0, 10).id;
        add_text(&mut h, "b", 0, 10);
        h.toggle_pin(a);
        let gone = h.clear_unpinned();
        assert_eq!(gone.len(), 1);
        assert_eq!(h.items.len(), 1);
    }

    #[test]
    fn merge_in_copy_order() {
        let mut h = History::new();
        let a = add_text(&mut h, "첫째", 0, 10).id;
        let b = add_text(&mut h, "둘째", 1, 10).id;
        let img = h
            .add(ClipContent::Image { file: "x.bmp".into(), width: 1, height: 1 }, "img".into(), None, 2, 10)
            .id;
        let c = add_text(&mut h, "셋째", 3, 10).id;
        // 고른 순서와 상관없이 복사한 순서대로, 이미지는 건너뜀
        assert_eq!(h.merge_text(&[c, a, img, b], "\n").as_deref(), Some("첫째\n둘째\n셋째"));
        assert_eq!(h.merge_text(&[c, a], ", ").as_deref(), Some("첫째, 셋째"));
        assert_eq!(h.merge_text(&[img], "\n"), None);
        assert_eq!(h.merge_text(&[], "\n"), None);
    }

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(100, 90), "방금");
        assert_eq!(relative_time(1000, 0), "16분 전");
        assert_eq!(relative_time(7200, 0), "2시간 전");
        assert_eq!(relative_time(86400 * 3, 0), "3일 전");
        assert_eq!(relative_time(0, 100), "방금");
    }

    #[test]
    fn previews() {
        assert_eq!(one_line("  a\r\n\tb   c ", 100), "a↵ b c");
        assert_eq!(one_line("abcdef", 3), "abc…");
        let item = ClipItem {
            id: 1,
            content: ClipContent::Files {
                paths: vec!["C:\\a\\x.txt".into(), "C:\\b\\folder\\".into()],
            },
            hash: String::new(),
            created: 0,
            last_used: 0,
            source_app: None,
            pinned: false,
            use_count: 0,
        };
        assert_eq!(item.preview(100), "[파일 2개] x.txt, folder");
    }
}
