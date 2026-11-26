//! Clip history in SQLite (`wisp.db`), with full images and thumbnails as PNG
//! files in an `images` folder beside it.

use crate::clip::{classify, content_hash, Captured, Clip, ClipContent, Kind};
use crate::images;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clips (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL,
    text         TEXT    NOT NULL DEFAULT '',
    hash         INTEGER NOT NULL UNIQUE,
    source_app   TEXT,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER,
    pinned       INTEGER NOT NULL DEFAULT 0,
    pin_order    INTEGER,
    size_bytes   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS clips_order ON clips (pinned DESC, created_at DESC);
CREATE TABLE IF NOT EXISTS clip_files (
    clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
    path    TEXT    NOT NULL,
    is_dir  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS clip_files_clip ON clip_files (clip_id);
CREATE TABLE IF NOT EXISTS clip_images (
    clip_id    INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,
    file_name  TEXT    NOT NULL,
    width      INTEGER NOT NULL,
    height     INTEGER NOT NULL,
    thumb_name TEXT    NOT NULL
);
";

const SELECT_CLIP: &str = "
SELECT c.id, c.kind, c.text, c.source_app, c.created_at, c.pinned, c.size_bytes,
       i.width, i.height, i.thumb_name
FROM clips c LEFT JOIN clip_images i ON i.clip_id = c.id";

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Db(rusqlite::Error),
    Io(std::io::Error),
    Image(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Db(e) => write!(f, "database: {e}"),
            Error::Io(e) => write!(f, "file: {e}"),
            Error::Image(e) => write!(f, "image: {e}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub struct Store {
    conn: Connection,
    images_dir: PathBuf,
}

fn row_to_clip(r: &Row) -> rusqlite::Result<Clip> {
    let w: Option<u32> = r.get(7)?;
    let h: Option<u32> = r.get(8)?;
    Ok(Clip {
        id: r.get(0)?,
        kind: Kind::parse(&r.get::<_, String>(1)?),
        text: r.get(2)?,
        source_app: r.get(3)?,
        created_at: r.get(4)?,
        pinned: r.get::<_, i64>(5)? != 0,
        size_bytes: r.get(6)?,
        image_size: w.zip(h),
        thumb: r.get(9)?,
    })
}

impl Store {
    /// Open (or create) the store in `dir`. Several connections may be open
    /// at once: WAL mode lets the capture thread write while the UI reads.
    pub fn open(dir: &Path) -> Result<Store> {
        std::fs::create_dir_all(dir)?;
        let images_dir = dir.join("images");
        std::fs::create_dir_all(&images_dir)?;
        let conn = Connection::open(dir.join("wisp.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn, images_dir })
    }

    pub fn images_dir(&self) -> &Path {
        &self.images_dir
    }

    /// Store a capture. A repeat of an existing clip moves it to the top
    /// instead of adding a duplicate. Returns the clip's id.
    pub fn insert(&self, cap: &Captured, now: i64) -> Result<i64> {
        let (kind, text, hash, size) = match &cap.content {
            ClipContent::Text(t) => {
                let kind = classify(t);
                (kind, t.clone(), content_hash("text", t.as_bytes()), t.len() as i64)
            }
            ClipContent::Files(paths) => {
                let text = paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n");
                let hash = content_hash("files", text.as_bytes());
                (Kind::Files, text, hash, 0)
            }
            ClipContent::Image { png, .. } => (Kind::Image, String::new(), content_hash("image", png), png.len() as i64),
        };

        let existing: Option<i64> =
            self.conn.query_row("SELECT id FROM clips WHERE hash = ?1", [hash], |r| r.get(0)).optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE clips SET created_at = ?2, source_app = COALESCE(?3, source_app) WHERE id = ?1",
                params![id, now, cap.source_app],
            )?;
            return Ok(id);
        }

        // Write image files before the row, so a row never points at nothing.
        let image_meta = match &cap.content {
            ClipContent::Image { png, width, height } => {
                let decoded = images::decode_png(png).ok_or_else(|| Error::Image("cannot decode PNG".into()))?;
                let thumb = images::encode_png(&images::thumbnail(&decoded, images::THUMB_MAX))
                    .map_err(|e| Error::Image(e.to_string()))?;
                let base = format!("{:016x}", hash as u64);
                let file_name = format!("{base}.png");
                let thumb_name = format!("{base}.thumb.png");
                std::fs::write(self.images_dir.join(&file_name), png)?;
                std::fs::write(self.images_dir.join(&thumb_name), thumb)?;
                Some((file_name, *width, *height, thumb_name))
            }
            _ => None,
        };

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO clips (kind, text, hash, source_app, created_at, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![kind.as_str(), text, hash, cap.source_app, now, size],
        )?;
        let id = tx.last_insert_rowid();
        if let ClipContent::Files(paths) = &cap.content {
            for p in paths {
                tx.execute(
                    "INSERT INTO clip_files (clip_id, path, is_dir) VALUES (?1, ?2, ?3)",
                    params![id, p.to_string_lossy(), p.is_dir() as i64],
                )?;
            }
        }
        if let Some((file_name, w, h, thumb_name)) = image_meta {
            tx.execute(
                "INSERT INTO clip_images (clip_id, file_name, width, height, thumb_name) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, file_name, w, h, thumb_name],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Newest first, pinned clips on top.
    pub fn recent(&self, limit: usize) -> Result<Vec<Clip>> {
        let sql = format!("{SELECT_CLIP} ORDER BY c.pinned DESC, c.created_at DESC, c.id DESC LIMIT ?1");
        let mut st = self.conn.prepare_cached(&sql)?;
        let rows = st.query_map([limit as i64], row_to_clip)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Case-insensitive substring search over text, file paths and source
    /// app. (Fuzzy, ranked search replaces this in milestone 3.)
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Clip>> {
        let q = query.trim();
        if q.is_empty() {
            return self.recent(limit);
        }
        let escaped = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("%{escaped}%");
        let sql = format!(
            "{SELECT_CLIP} WHERE c.text LIKE ?1 ESCAPE '\\' OR c.source_app LIKE ?1 ESCAPE '\\'
             ORDER BY c.pinned DESC, c.created_at DESC, c.id DESC LIMIT ?2"
        );
        let mut st = self.conn.prepare_cached(&sql)?;
        let rows = st.query_map(params![pattern, limit as i64], row_to_clip)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0))?)
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn.execute("UPDATE clips SET pinned = ?2 WHERE id = ?1", params![id, pinned as i64])?;
        Ok(())
    }

    /// Delete clips matching `where_sql` (which may use ?1) and their image files.
    fn delete_where(&self, where_sql: &str, arg: i64) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let files: Vec<(String, String)> = {
            let sql = format!(
                "SELECT i.file_name, i.thumb_name FROM clip_images i JOIN clips c ON c.id = i.clip_id WHERE {where_sql}"
            );
            let mut st = tx.prepare(&sql)?;
            let rows = st.query_map([arg], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let n = tx.execute(&format!("DELETE FROM clips AS c WHERE {where_sql}"), [arg])?;
        tx.commit()?;
        for (a, b) in files {
            let _ = std::fs::remove_file(self.images_dir.join(a));
            let _ = std::fs::remove_file(self.images_dir.join(b));
        }
        Ok(n)
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.delete_where("c.id = ?1", id).map(|_| ())
    }

    /// Remove all clips, or all except pinned ones.
    pub fn clear(&self, keep_pinned: bool) -> Result<usize> {
        if keep_pinned {
            self.delete_where("c.pinned = 0 AND ?1 = ?1", 0)
        } else {
            self.delete_where("?1 = ?1", 0)
        }
    }

    /// Drop unpinned clips older than `keep_days` (0 = keep forever) and
    /// unpinned clips beyond the newest `max_clips` (0 = no limit).
    pub fn apply_retention(&self, keep_days: u32, max_clips: u32, now: i64) -> Result<usize> {
        let mut removed = 0;
        if keep_days > 0 {
            let cutoff = now - keep_days as i64 * 86_400;
            removed += self.delete_where("c.pinned = 0 AND c.created_at < ?1", cutoff)?;
        }
        if max_clips > 0 {
            removed += self.delete_where(
                "c.pinned = 0 AND c.id IN (SELECT id FROM clips WHERE pinned = 0
                   ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?1)",
                max_clips as i64,
            )?;
        }
        Ok(removed)
    }

    /// Absolute path of an image clip's thumbnail.
    pub fn thumb_path(&self, clip: &Clip) -> Option<PathBuf> {
        clip.thumb.as_ref().map(|t| self.images_dir.join(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_store() -> (Store, PathBuf) {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wisp-store-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (Store::open(&dir).unwrap(), dir)
    }

    fn text(t: &str) -> Captured {
        Captured { content: ClipContent::Text(t.into()), source_app: Some("Notepad".into()) }
    }

    fn png(w: u32, h: u32) -> Captured {
        let img = images::Rgba { width: w, height: h, pixels: vec![200; (w * h * 4) as usize] };
        Captured {
            content: ClipContent::Image { png: images::encode_png(&img).unwrap(), width: w, height: h },
            source_app: None,
        }
    }

    #[test]
    fn inserts_and_lists_newest_first() {
        let (s, dir) = temp_store();
        s.insert(&text("first"), 100).unwrap();
        s.insert(&text("second"), 200).unwrap();
        let r = s.recent(10).unwrap();
        assert_eq!(r.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["second", "first"]);
        assert_eq!(r[0].source_app.as_deref(), Some("Notepad"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn repeat_copy_moves_to_top_without_duplicate() {
        let (s, dir) = temp_store();
        let a = s.insert(&text("Starfall"), 100).unwrap();
        s.insert(&text("Moonforge"), 200).unwrap();
        let again = s.insert(&text("Starfall"), 300).unwrap();
        assert_eq!(a, again);
        assert_eq!(s.count().unwrap(), 2);
        assert_eq!(s.recent(1).unwrap()[0].text, "Starfall");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn detects_kinds_on_insert() {
        let (s, dir) = temp_store();
        s.insert(&text("https://example.com"), 1).unwrap();
        s.insert(&text("#2F6BFF"), 2).unwrap();
        let r = s.recent(10).unwrap();
        assert_eq!(r[0].kind, Kind::Color);
        assert_eq!(r[1].kind, Kind::Url);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn files_are_stored_and_searchable_by_name() {
        let (s, dir) = temp_store();
        let cap = Captured {
            content: ClipContent::Files(vec!["C:\\Work\\Mirelle notes.txt".into(), "C:\\Work\\logo.svg".into()]),
            source_app: Some("explorer".into()),
        };
        s.insert(&cap, 1).unwrap();
        let hits = s.search("mirelle", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, Kind::Files);
        assert_eq!(hits[0].title(), "Mirelle notes.txt and 1 more");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn images_get_files_and_thumbnail() {
        let (s, dir) = temp_store();
        s.insert(&png(640, 320), 1).unwrap();
        let c = &s.recent(1).unwrap()[0];
        assert_eq!(c.kind, Kind::Image);
        assert_eq!(c.image_size, Some((640, 320)));
        let thumb = images::decode_png(&std::fs::read(s.thumb_path(c).unwrap()).unwrap()).unwrap();
        assert_eq!((thumb.width, thumb.height), (160, 80));
        s.delete(c.id).unwrap();
        assert_eq!(std::fs::read_dir(s.images_dir()).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn search_is_case_insensitive_and_escapes_wildcards() {
        let (s, dir) = temp_store();
        s.insert(&text("Invoice 100% paid"), 1).unwrap();
        s.insert(&text("invoice draft"), 2).unwrap();
        s.insert(&text("unrelated"), 3).unwrap();
        assert_eq!(s.search("INVOICE", 10).unwrap().len(), 2);
        assert_eq!(s.search("100%", 10).unwrap().len(), 1);
        assert_eq!(s.search("_", 10).unwrap().len(), 0);
        assert_eq!(s.search("notepad", 10).unwrap().len(), 3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn retention_by_age_and_count_keeps_pinned() {
        let (s, dir) = temp_store();
        let day = 86_400;
        let old = s.insert(&text("old"), 0).unwrap();
        s.insert(&text("old pinned"), 0).unwrap();
        s.set_pinned(s.recent(1).unwrap()[0].id, true).unwrap();
        for i in 0..5 {
            s.insert(&text(&format!("new {i}")), 40 * day + i).unwrap();
        }
        // 30 days: "old" goes, the pinned one stays.
        assert_eq!(s.apply_retention(30, 0, 40 * day + 10).unwrap(), 1);
        assert!(s.recent(10).unwrap().iter().all(|c| c.id != old));
        // Keep 3 unpinned: two oldest "new" go.
        assert_eq!(s.apply_retention(0, 3, 40 * day + 10).unwrap(), 2);
        let left: Vec<String> = s.recent(10).unwrap().into_iter().map(|c| c.text).collect();
        assert_eq!(left, ["old pinned", "new 4", "new 3", "new 2"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn clear_can_keep_pinned() {
        let (s, dir) = temp_store();
        s.insert(&text("a"), 1).unwrap();
        let b = s.insert(&text("b"), 2).unwrap();
        s.set_pinned(b, true).unwrap();
        assert_eq!(s.clear(true).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 1);
        assert_eq!(s.clear(false).unwrap(), 1);
        assert_eq!(s.count().unwrap(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn two_connections_see_each_others_writes() {
        let (writer, dir) = temp_store();
        let reader = Store::open(&dir).unwrap();
        writer.insert(&text("shared"), 1).unwrap();
        assert_eq!(reader.recent(5).unwrap()[0].text, "shared");
        std::fs::remove_dir_all(dir).ok();
    }
}
