//! Named backups of the whole clip history.
//!
//! A backup is one `.wispclips` file: a small SQLite database holding the
//! clips, their file lists and their images (as PNG blobs), plus a `meta`
//! table with the backup's name, time and format version. One file per
//! backup makes them easy to copy between computers.

use crate::store::{Error, Result, Store};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const EXTENSION: &str = "wispclips";
const FORMAT_VERSION: i64 = 1;

/// Safety snapshots kept before Wisp deletes the oldest.
pub const KEEP_SNAPSHOTS: usize = 5;

const BACKUP_SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE clips (
    id INTEGER PRIMARY KEY, kind TEXT NOT NULL, text TEXT NOT NULL, hash INTEGER NOT NULL,
    source_app TEXT, created_at INTEGER NOT NULL, last_used_at INTEGER, pinned INTEGER NOT NULL,
    pin_order INTEGER, size_bytes INTEGER NOT NULL
);
CREATE TABLE clip_files (clip_id INTEGER NOT NULL, path TEXT NOT NULL, is_dir INTEGER NOT NULL);
CREATE TABLE clip_images (
    clip_id INTEGER PRIMARY KEY, file_name TEXT NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
    thumb_name TEXT NOT NULL, png BLOB NOT NULL, thumb BLOB NOT NULL
);
";

const CLIP_COLUMNS: &str =
    "id, kind, text, hash, source_app, created_at, last_used_at, pinned, pin_order, size_bytes";

/// A backup file as the Backups page lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct BackupInfo {
    pub name: String,
    pub path: PathBuf,
    pub clips: i64,
    /// Unix seconds.
    pub created_at: i64,
}

/// Turn a typed name into a safe file name: no characters Windows forbids,
/// no leading or trailing dots or spaces. `None` if nothing is left.
pub fn file_stem_for(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ');
    let cleaned: String = cleaned.chars().take(80).collect();
    // Names Windows reserves for devices.
    let upper = cleaned.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or("");
    let reserved = matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if cleaned.is_empty() || reserved {
        None
    } else {
        Some(cleaned)
    }
}

/// Read a backup's name, time and size without loading it.
pub fn info(path: &Path) -> Option<BackupInfo> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let meta = |key: &str| -> Option<String> {
        conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok()?
    };
    if meta("format")?.parse::<i64>().ok()? > FORMAT_VERSION {
        return None;
    }
    let name = meta("name").unwrap_or_else(|| {
        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    });
    let created_at = meta("created_at").and_then(|v| v.parse().ok()).unwrap_or(0);
    let clips = conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0)).ok()?;
    Some(BackupInfo { name, path: path.to_path_buf(), clips, created_at })
}

/// Backups in `dir`, newest first. Files that aren't Wisp backups are skipped.
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<BackupInfo> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(EXTENSION)))
        .filter_map(|p| info(&p))
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Delete all but the newest `keep` backups in `dir`.
pub fn prune(dir: &Path, keep: usize) {
    for old in list(dir).into_iter().skip(keep) {
        let _ = std::fs::remove_file(old.path);
    }
}

impl Store {
    /// Save every clip to a backup file at `path`, replacing any file there.
    /// Returns the number of clips saved.
    pub fn backup_to(&self, path: &Path, name: &str, now: i64) -> Result<usize> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write beside the target and rename, so a failed backup never
        // leaves a half-written file under the real name.
        let tmp = path.with_extension("wispclips-partial");
        let _ = std::fs::remove_file(&tmp);
        let result = self.write_backup(&tmp, name, now);
        match result {
            Ok(n) => {
                std::fs::rename(&tmp, path)?;
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    fn write_backup(&self, path: &Path, name: &str, now: i64) -> Result<usize> {
        let out = Connection::open(path)?;
        out.execute_batch(BACKUP_SCHEMA)?;
        let tx = out.unchecked_transaction()?;
        for (k, v) in [
            ("format", FORMAT_VERSION.to_string()),
            ("name", name.to_string()),
            ("created_at", now.to_string()),
            ("app", format!("Wisp {}", env!("CARGO_PKG_VERSION"))),
        ] {
            tx.execute("INSERT INTO meta (key, value) VALUES (?1, ?2)", params![k, v])?;
        }

        // Read from a snapshot of the history, so clips captured meanwhile
        // don't leave the backup half-consistent.
        let src = self.conn();
        let read = src.unchecked_transaction()?;
        let mut count = 0;
        {
            let mut st = read.prepare(&format!("SELECT {CLIP_COLUMNS} FROM clips"))?;
            let mut rows = st.query([])?;
            let mut ins = tx.prepare(&format!("INSERT INTO clips ({CLIP_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"))?;
            while let Some(r) = rows.next()? {
                ins.execute(params![
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<i64>>(8)?,
                    r.get::<_, i64>(9)?,
                ])?;
                count += 1;
            }
        }
        {
            let mut st = read.prepare("SELECT clip_id, path, is_dir FROM clip_files")?;
            let mut rows = st.query([])?;
            let mut ins = tx.prepare("INSERT INTO clip_files (clip_id, path, is_dir) VALUES (?1, ?2, ?3)")?;
            while let Some(r) = rows.next()? {
                ins.execute(params![r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?])?;
            }
        }
        {
            let mut st = read.prepare("SELECT clip_id, file_name, width, height, thumb_name FROM clip_images")?;
            let mut rows = st.query([])?;
            let mut ins = tx.prepare(
                "INSERT INTO clip_images (clip_id, file_name, width, height, thumb_name, png, thumb)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            while let Some(r) = rows.next()? {
                let file: String = r.get(1)?;
                let thumb: String = r.get(4)?;
                // An image whose file went missing is left out of the backup;
                // its clip still restores, without a picture.
                let (Ok(png), Ok(thumb_png)) = (
                    std::fs::read(self.images_dir().join(&file)),
                    std::fs::read(self.images_dir().join(&thumb)),
                ) else {
                    continue;
                };
                ins.execute(params![
                    r.get::<_, i64>(0)?,
                    file,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    thumb,
                    png,
                    thumb_png
                ])?;
            }
        }
        read.finish()?;
        tx.commit()?;
        Ok(count)
    }

    /// Replace every clip, pinned ones included, with the clips in the
    /// backup at `path`. Nothing changes if the backup can't be read.
    /// Returns the number of clips restored.
    pub fn restore_from(&self, path: &Path) -> Result<usize> {
        if info(path).is_none() {
            return Err(Error::Backup("This file is not a Wisp backup, or it was made by a newer Wisp.".into()));
        }
        let src = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let images_dir = self.images_dir().to_path_buf();

        // Image files now on disk, to clean up after the swap.
        let old_files: HashSet<String> = {
            let mut st = self.conn().prepare("SELECT file_name, thumb_name FROM clip_images")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut set = HashSet::new();
            for r in rows {
                let (a, b) = r?;
                set.insert(a);
                set.insert(b);
            }
            set
        };

        let mut written: Vec<String> = Vec::new();
        let result = (|| -> Result<usize> {
            let tx = self.conn().unchecked_transaction()?;
            tx.execute("DELETE FROM clips", [])?;
            let mut count = 0;
            {
                let mut st = src.prepare(&format!("SELECT {CLIP_COLUMNS} FROM clips"))?;
                let mut rows = st.query([])?;
                let mut ins = tx.prepare(&format!(
                    "INSERT OR IGNORE INTO clips ({CLIP_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
                ))?;
                while let Some(r) = rows.next()? {
                    count += ins.execute(params![
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, Option<i64>>(8)?,
                        r.get::<_, i64>(9)?,
                    ])?;
                }
            }
            {
                let mut st = src.prepare("SELECT clip_id, path, is_dir FROM clip_files")?;
                let mut rows = st.query([])?;
                let mut ins = tx.prepare(
                    "INSERT INTO clip_files (clip_id, path, is_dir)
                     SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM clips WHERE id = ?1)",
                )?;
                while let Some(r) = rows.next()? {
                    ins.execute(params![r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?])?;
                }
            }
            {
                let mut st = src.prepare("SELECT clip_id, file_name, width, height, thumb_name, png, thumb FROM clip_images")?;
                let mut rows = st.query([])?;
                let mut ins = tx.prepare(
                    "INSERT INTO clip_images (clip_id, file_name, width, height, thumb_name)
                     SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM clips WHERE id = ?1)",
                )?;
                while let Some(r) = rows.next()? {
                    let file: String = r.get(1)?;
                    let thumb: String = r.get(4)?;
                    // Only plain file names: a backup must not write elsewhere.
                    if !is_plain_name(&file) || !is_plain_name(&thumb) {
                        continue;
                    }
                    for (name, col) in [(&file, 5), (&thumb, 6)] {
                        if !old_files.contains(name) {
                            std::fs::write(images_dir.join(name), r.get::<_, Vec<u8>>(col)?)?;
                            written.push(name.clone());
                        }
                    }
                    ins.execute(params![r.get::<_, i64>(0)?, file, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, thumb])?;
                }
            }
            tx.commit()?;
            Ok(count)
        })();

        match result {
            Ok(n) => {
                // Drop image files no restored clip uses.
                let keep: HashSet<String> = {
                    let mut st = self.conn().prepare("SELECT file_name, thumb_name FROM clip_images")?;
                    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                    let mut set = HashSet::new();
                    for r in rows.flatten() {
                        set.insert(r.0);
                        set.insert(r.1);
                    }
                    set
                };
                for f in old_files.difference(&keep) {
                    let _ = std::fs::remove_file(images_dir.join(f));
                }
                Ok(n)
            }
            Err(e) => {
                for f in written {
                    let _ = std::fs::remove_file(images_dir.join(f));
                }
                Err(e)
            }
        }
    }
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Captured, ClipContent};
    use crate::images;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wisp-backup-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn text(t: &str) -> Captured {
        Captured { content: ClipContent::Text(t.into()), source_app: Some("Notepad".into()) }
    }

    fn image(shade: u8) -> Captured {
        let img = images::Rgba { width: 8, height: 4, pixels: vec![shade; 8 * 4 * 4] };
        Captured {
            content: ClipContent::Image { png: images::encode_png(&img).unwrap(), width: 8, height: 4 },
            source_app: None,
        }
    }

    fn texts(s: &Store) -> Vec<String> {
        s.recent(100).unwrap().into_iter().map(|c| c.text).collect()
    }

    #[test]
    fn names_become_safe_file_names() {
        assert_eq!(file_stem_for("  Starfall: launch / copy  ").as_deref(), Some("Starfall launch copy"));
        assert_eq!(file_stem_for("..hidden.."), Some("hidden".into()));
        assert_eq!(file_stem_for("???"), None);
        assert_eq!(file_stem_for("con"), None);
        assert_eq!(file_stem_for("COM1.txt"), None);
        assert_eq!(file_stem_for("Console"), Some("Console".into()));
    }

    #[test]
    fn backup_and_restore_round_trip() {
        let dir = temp_dir();
        let s = Store::open(&dir.join("data")).unwrap();
        s.insert(&text("Moonforge invoice"), 1).unwrap();
        let pinned = s.insert(&text("Mirelle address"), 2).unwrap();
        s.set_pinned(pinned, true).unwrap();
        let files = vec![PathBuf::from("C:\\Work\\Starfall.pdf")];
        s.insert(&Captured { content: ClipContent::Files(files.clone()), source_app: None }, 3).unwrap();
        let img = s.insert(&image(90), 4).unwrap();
        let png_before = s.content(img).unwrap();

        let file = dir.join("backups").join("Work.wispclips");
        assert_eq!(s.backup_to(&file, "Work", 100).unwrap(), 4);
        let i = info(&file).unwrap();
        assert_eq!((i.name.as_str(), i.clips, i.created_at), ("Work", 4, 100));

        // Change everything, then restore.
        s.clear(false).unwrap();
        s.insert(&text("something new"), 5).unwrap();
        s.insert(&image(200), 6).unwrap();
        assert_eq!(s.restore_from(&file).unwrap(), 4);

        assert_eq!(s.count().unwrap(), 4);
        let r = s.recent(10).unwrap();
        assert_eq!(r[0].text, "Mirelle address");
        assert!(r[0].pinned);
        assert!(!texts(&s).contains(&"something new".to_string()));
        let f = r.iter().find(|c| c.kind == crate::Kind::Files).unwrap();
        assert_eq!(s.content(f.id).unwrap(), Some(ClipContent::Files(files)));
        assert_eq!(s.content(img).unwrap(), png_before);
        // The new image's files are gone, the restored one's are back.
        assert_eq!(std::fs::read_dir(s.images_dir()).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bad_backup_changes_nothing() {
        let dir = temp_dir();
        let s = Store::open(&dir.join("data")).unwrap();
        s.insert(&text("keep me"), 1).unwrap();
        let junk = dir.join("junk.wispclips");
        std::fs::write(&junk, b"not a database").unwrap();
        assert!(s.restore_from(&junk).is_err());
        assert!(s.restore_from(&dir.join("missing.wispclips")).is_err());
        assert_eq!(texts(&s), ["keep me"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn list_is_newest_first_and_prune_keeps_newest() {
        let dir = temp_dir();
        let s = Store::open(&dir.join("data")).unwrap();
        s.insert(&text("a"), 1).unwrap();
        let b = dir.join("b");
        for (n, t) in [("Old", 10), ("Newest", 30), ("Middle", 20)] {
            s.backup_to(&b.join(format!("{n}.wispclips")), n, t).unwrap();
        }
        std::fs::write(b.join("notes.txt"), "x").unwrap();
        let names: Vec<String> = list(&b).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["Newest", "Middle", "Old"]);
        prune(&b, 2);
        let names: Vec<String> = list(&b).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["Newest", "Middle"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn backup_overwrites_same_name() {
        let dir = temp_dir();
        let s = Store::open(&dir.join("data")).unwrap();
        s.insert(&text("one"), 1).unwrap();
        let f = dir.join("Same.wispclips");
        s.backup_to(&f, "Same", 1).unwrap();
        s.insert(&text("two"), 2).unwrap();
        s.backup_to(&f, "Same", 2).unwrap();
        assert_eq!(info(&f).unwrap().clips, 2);
        std::fs::remove_dir_all(dir).ok();
    }
}
