//! # vinci-clips — clip library
//!
//! One physical file per clip, in the user's clips folder; SQLite only stores metadata
//! (title, game, duration, codec, favourite, thumbnail path…). The folder is the source
//! of truth: [`Library::scan`] picks up files added/removed outside VINCI.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use vinci_encoding::MediaInfo;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("clip {0} not found")]
    NotFound(i64),
}

pub type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    pub id: i64,
    pub path: PathBuf,
    pub title: String,
    pub game: Option<String>,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    pub duration: f64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub encoder: Option<String>,
    pub favorite: bool,
    pub thumbnail: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct NewClip {
    pub path: PathBuf,
    pub title: String,
    pub game: Option<String>,
    pub created_at: i64,
    pub encoder: Option<String>,
    pub media: MediaInfo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Newest,
    Oldest,
    Largest,
    Smallest,
    Longest,
    Shortest,
    Name,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ClipQuery {
    pub search: Option<String>,
    pub game: Option<String>,
    pub favorites_only: bool,
    pub sort: SortBy,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LibraryStats {
    pub count: u64,
    pub total_bytes: u64,
    pub total_duration: f64,
    pub games: Vec<GameCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameCount {
    pub game: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct ScanReport {
    pub added: usize,
    pub removed: usize,
    pub updated: usize,
}

pub const VIDEO_EXTS: [&str; 4] = ["mp4", "mkv", "mov", "webm"];

const MIGRATIONS: &[&str] = &[
    // v1
    "CREATE TABLE clips (
        id INTEGER PRIMARY KEY,
        path TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        game TEXT,
        created_at INTEGER NOT NULL,
        duration REAL NOT NULL DEFAULT 0,
        size INTEGER NOT NULL DEFAULT 0,
        width INTEGER NOT NULL DEFAULT 0,
        height INTEGER NOT NULL DEFAULT 0,
        fps REAL NOT NULL DEFAULT 0,
        video_codec TEXT,
        audio_codec TEXT,
        encoder TEXT,
        favorite INTEGER NOT NULL DEFAULT 0,
        thumbnail TEXT,
        mtime INTEGER NOT NULL DEFAULT 0
     );
     CREATE INDEX idx_clips_created ON clips(created_at);
     CREATE INDEX idx_clips_game ON clips(game);",
];

pub struct Library {
    conn: Connection,
    thumbs_dir: PathBuf,
}

fn mtime_of(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_to_clip(r: &Row) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: r.get(0)?,
        path: PathBuf::from(r.get::<_, String>(1)?),
        title: r.get(2)?,
        game: r.get(3)?,
        created_at: r.get(4)?,
        duration: r.get(5)?,
        size: r.get::<_, i64>(6)? as u64,
        width: r.get::<_, i64>(7)? as u32,
        height: r.get::<_, i64>(8)? as u32,
        fps: r.get(9)?,
        video_codec: r.get(10)?,
        audio_codec: r.get(11)?,
        encoder: r.get(12)?,
        favorite: r.get::<_, i64>(13)? != 0,
        thumbnail: r.get::<_, Option<String>>(14)?.map(PathBuf::from),
    })
}

const COLS: &str =
    "id, path, title, game, created_at, duration, size, width, height, fps, video_codec, audio_codec, encoder, favorite, thumbnail";

impl Library {
    pub fn open(db: &Path, thumbs_dir: &Path) -> Result<Self> {
        if let Some(d) = db.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::create_dir_all(thumbs_dir)?;
        let conn = Connection::open(db)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
        let lib = Self { conn, thumbs_dir: thumbs_dir.to_path_buf() };
        lib.migrate()?;
        Ok(lib)
    }

    pub fn open_in_memory(thumbs_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(thumbs_dir)?;
        let lib = Self { conn: Connection::open_in_memory()?, thumbs_dir: thumbs_dir.to_path_buf() };
        lib.migrate()?;
        Ok(lib)
    }

    fn migrate(&self) -> Result<()> {
        let v: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, m) in MIGRATIONS.iter().enumerate().skip(v as usize) {
            self.conn.execute_batch(&format!("BEGIN; {m}; PRAGMA user_version = {}; COMMIT;", i + 1))?;
        }
        Ok(())
    }

    pub fn insert(&self, c: NewClip) -> Result<Clip> {
        let size = std::fs::metadata(&c.path).map(|m| m.len()).unwrap_or(c.media.size_bytes);
        self.conn.execute(
            "INSERT INTO clips (path, title, game, created_at, duration, size, width, height, fps, video_codec, audio_codec, encoder, mtime)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(path) DO UPDATE SET duration=excluded.duration, size=excluded.size, width=excluded.width, height=excluded.height,
               fps=excluded.fps, video_codec=excluded.video_codec, audio_codec=excluded.audio_codec, mtime=excluded.mtime",
            params![
                c.path.to_string_lossy(),
                c.title,
                c.game,
                c.created_at,
                c.media.duration_secs,
                size as i64,
                c.media.width as i64,
                c.media.height as i64,
                c.media.fps,
                (!c.media.video_codec.is_empty()).then_some(c.media.video_codec.clone()),
                c.media.audio_codec,
                c.encoder,
                mtime_of(&c.path)
            ],
        )?;
        let id: i64 = self.conn.query_row("SELECT id FROM clips WHERE path = ?1", [c.path.to_string_lossy()], |r| r.get(0))?;
        self.get(id)
    }

    pub fn get(&self, id: i64) -> Result<Clip> {
        self.conn
            .query_row(&format!("SELECT {COLS} FROM clips WHERE id = ?1"), [id], row_to_clip)
            .optional()?
            .ok_or(LibraryError::NotFound(id))
    }

    pub fn by_path(&self, path: &Path) -> Result<Option<Clip>> {
        Ok(self.conn.query_row(&format!("SELECT {COLS} FROM clips WHERE path = ?1"), [path.to_string_lossy()], row_to_clip).optional()?)
    }

    pub fn list(&self, q: &ClipQuery) -> Result<Vec<Clip>> {
        let mut sql = format!("SELECT {COLS} FROM clips WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = q.search.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            sql.push_str(" AND (title LIKE ?1 ESCAPE '\\' OR IFNULL(game,'') LIKE ?1 ESCAPE '\\')");
            let esc = s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            args.push(Box::new(format!("%{esc}%")));
        }
        if let Some(g) = &q.game {
            sql.push_str(&format!(" AND game = ?{}", args.len() + 1));
            args.push(Box::new(g.clone()));
        }
        if q.favorites_only {
            sql.push_str(" AND favorite = 1");
        }
        sql.push_str(match q.sort {
            SortBy::Newest => " ORDER BY created_at DESC",
            SortBy::Oldest => " ORDER BY created_at ASC",
            SortBy::Largest => " ORDER BY size DESC",
            SortBy::Smallest => " ORDER BY size ASC",
            SortBy::Longest => " ORDER BY duration DESC",
            SortBy::Shortest => " ORDER BY duration ASC",
            SortBy::Name => " ORDER BY title COLLATE NOCASE ASC",
        });
        sql.push_str(&format!(" LIMIT {} OFFSET {}", q.limit.unwrap_or(10_000), q.offset.unwrap_or(0)));
        let mut stmt = self.conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), row_to_clip)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn stats(&self) -> Result<LibraryStats> {
        let (count, bytes, dur): (i64, i64, f64) =
            self.conn.query_row("SELECT COUNT(*), IFNULL(SUM(size),0), IFNULL(SUM(duration),0) FROM clips", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
        let mut stmt =
            self.conn.prepare("SELECT game, COUNT(*) FROM clips WHERE game IS NOT NULL GROUP BY game ORDER BY COUNT(*) DESC, game")?;
        let games = stmt
            .query_map([], |r| Ok(GameCount { game: r.get(0)?, count: r.get::<_, i64>(1)? as u64 }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(LibraryStats { count: count as u64, total_bytes: bytes as u64, total_duration: dur, games })
    }

    pub fn set_favorite(&self, id: i64, fav: bool) -> Result<Clip> {
        self.conn.execute("UPDATE clips SET favorite = ?2 WHERE id = ?1", params![id, fav as i64])?;
        self.get(id)
    }

    pub fn set_game(&self, id: i64, game: Option<&str>) -> Result<Clip> {
        self.conn.execute("UPDATE clips SET game = ?2 WHERE id = ?1", params![id, game])?;
        self.get(id)
    }

    /// Rename the clip — the file on disk is renamed too, keeping the extension.
    pub fn rename(&self, id: i64, title: &str) -> Result<Clip> {
        let clip = self.get(id)?;
        let title = title.trim();
        let title = if title.is_empty() { clip.title.as_str() } else { title };
        let ext = clip.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        let dir = clip.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = vinci_storage::sanitize_filename(title);
        let current_stem = clip.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let new_path = if stem == current_stem { clip.path.clone() } else { vinci_storage::unique_path(&dir, &stem, &ext) };
        if new_path != clip.path && clip.path.exists() {
            std::fs::rename(&clip.path, &new_path)?;
        }
        self.conn.execute(
            "UPDATE clips SET title = ?2, path = ?3, mtime = ?4 WHERE id = ?1",
            params![id, title, new_path.to_string_lossy(), mtime_of(&new_path)],
        )?;
        self.get(id)
    }

    /// Move the clip file to another folder.
    pub fn move_to(&self, id: i64, dest_dir: &Path) -> Result<Clip> {
        let clip = self.get(id)?;
        let stem = clip.path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_string();
        let ext = clip.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        std::fs::create_dir_all(dest_dir)?;
        let dest = vinci_storage::unique_path(dest_dir, &stem, &ext);
        vinci_storage::move_file(&clip.path, &dest)?;
        self.conn.execute("UPDATE clips SET path = ?2 WHERE id = ?1", params![id, dest.to_string_lossy()])?;
        self.get(id)
    }

    /// Remove from the library and delete the file (and its thumbnail).
    pub fn delete(&self, id: i64, delete_file: bool) -> Result<()> {
        let clip = self.get(id)?;
        if delete_file && clip.path.exists() {
            std::fs::remove_file(&clip.path)?;
        }
        if let Some(t) = &clip.thumbnail {
            let _ = std::fs::remove_file(t);
        }
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The file was replaced in place (e.g. compressed): refresh technical metadata.
    pub fn refresh_media(&self, id: i64, media: &MediaInfo, encoder: Option<&str>) -> Result<Clip> {
        let clip = self.get(id)?;
        let size = std::fs::metadata(&clip.path).map(|m| m.len()).unwrap_or(media.size_bytes);
        self.conn.execute(
            "UPDATE clips SET duration=?2, size=?3, width=?4, height=?5, fps=?6, video_codec=?7, audio_codec=?8, encoder=IFNULL(?9, encoder), mtime=?10 WHERE id=?1",
            params![id, media.duration_secs, size as i64, media.width as i64, media.height as i64, media.fps, media.video_codec, media.audio_codec, encoder, mtime_of(&clip.path)],
        )?;
        self.get(id)
    }

    /// Change the path of a clip (used when a compressed copy replaces the original
    /// under a different extension).
    pub fn set_path(&self, id: i64, path: &Path) -> Result<Clip> {
        self.conn.execute("UPDATE clips SET path = ?2 WHERE id = ?1", params![id, path.to_string_lossy()])?;
        self.get(id)
    }

    /// Synchronise with the clips folder (depth ≤ 2: `<clips>/<Game>/<file>`).
    /// `probe` reads media info for newly discovered files.
    pub fn scan(&self, clips_dir: &Path, probe: &dyn Fn(&Path) -> Option<MediaInfo>) -> Result<ScanReport> {
        let mut report = ScanReport::default();
        let mut stack = vec![(clips_dir.to_path_buf(), 0u8)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if p.is_dir() {
                    if depth < 2 {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
                if !VIDEO_EXTS.contains(&ext.as_str()) {
                    continue;
                }
                match self.by_path(&p)? {
                    Some(c) => {
                        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        if size != c.size {
                            if let Some(m) = probe(&p) {
                                self.refresh_media(c.id, &m, None)?;
                                report.updated += 1;
                            }
                        }
                    }
                    None => {
                        let media = probe(&p).unwrap_or_default();
                        let game = (depth >= 1).then(|| dir.file_name().map(|n| n.to_string_lossy().into_owned())).flatten();
                        let created = mtime_of(&p);
                        let title = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                        self.insert(NewClip { path: p.clone(), title, game, created_at: created, encoder: None, media })?;
                        report.added += 1;
                    }
                }
            }
        }
        // Rows whose file is gone (deleted or moved away in the file manager).
        let mut stmt = self.conn.prepare("SELECT id, path, thumbnail FROM clips")?;
        let rows: Vec<(i64, String, Option<String>)> =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, path, thumb) in rows {
            let p = PathBuf::from(&path);
            if !p.exists() {
                if let Some(t) = thumb {
                    let _ = std::fs::remove_file(t);
                }
                self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Return the thumbnail, generating it (480 px JPEG) with FFmpeg if needed.
    pub fn ensure_thumbnail(&self, id: i64, ffmpeg: &Path) -> Result<Option<PathBuf>> {
        let clip = self.get(id)?;
        if let Some(t) = &clip.thumbnail {
            if t.exists() {
                return Ok(Some(t.clone()));
            }
        }
        if !clip.path.exists() {
            return Ok(None);
        }
        let out = self.thumbs_dir.join(format!("{}-{}.jpg", clip.id, mtime_of(&clip.path)));
        let tmp = self.thumbs_dir.join(format!(".{}.tmp.jpg", clip.id));
        let at = if clip.duration > 0.0 { (clip.duration / 3.0).min(2.0) } else { 0.0 };
        let ok = vinci_hw::process::command(ffmpeg)
            .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"])
            .arg(&clip.path)
            .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4", "-f", "image2"])
            .arg(&tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok || !tmp.exists() {
            let _ = std::fs::remove_file(&tmp);
            return Ok(None);
        }
        std::fs::rename(&tmp, &out)?;
        if let Some(old) = &clip.thumbnail {
            let _ = std::fs::remove_file(old);
        }
        self.conn.execute("UPDATE clips SET thumbnail = ?2 WHERE id = ?1", params![id, out.to_string_lossy()])?;
        Ok(Some(out))
    }
}

/// Default human title, e.g. "Valorant — Nov 4, 21:03".
pub fn default_title(game: Option<&str>, at: chrono::DateTime<chrono::Local>) -> String {
    let when = at.format("%b %-d, %H:%M:%S");
    match game {
        Some(g) => format!("{g} — {when}"),
        None => format!("Clip — {when}"),
    }
}

/// Default file stem, e.g. "Valorant 2026-11-04 21-03-15".
pub fn default_file_stem(game: Option<&str>, at: chrono::DateTime<chrono::Local>) -> String {
    let when = at.format("%Y-%m-%d %H-%M-%S");
    vinci_storage::sanitize_filename(&format!("{} {when}", game.unwrap_or("Clip")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(d: f64) -> MediaInfo {
        MediaInfo {
            duration_secs: d,
            width: 1920,
            height: 1080,
            fps: 60.0,
            video_codec: "hevc".into(),
            audio_codec: Some("aac".into()),
            size_bytes: 0,
            bitrate_kbps: 0,
        }
    }

    fn setup() -> (tempfile::TempDir, Library, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let clips = d.path().join("Clips");
        std::fs::create_dir_all(clips.join("Valorant")).unwrap();
        let lib = Library::open(&d.path().join("db/library.db"), &d.path().join("thumbs")).unwrap();
        (d, lib, clips)
    }

    fn add(lib: &Library, path: &Path, game: Option<&str>, t: i64, bytes: usize) -> Clip {
        std::fs::write(path, vec![0u8; bytes]).unwrap();
        lib.insert(NewClip {
            path: path.into(),
            title: path.file_stem().unwrap().to_string_lossy().into(),
            game: game.map(String::from),
            created_at: t,
            encoder: Some("hevc_nvenc".into()),
            media: media(30.0),
        })
        .unwrap()
    }

    #[test]
    fn crud_search_sort() {
        let (_d, lib, clips) = setup();
        let a = add(&lib, &clips.join("Valorant/Ace.mp4"), Some("Valorant"), 1000, 100);
        let b = add(&lib, &clips.join("clutch_100%.mp4"), None, 2000, 300);
        assert_eq!(lib.list(&ClipQuery::default()).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), vec![b.id, a.id]);
        assert_eq!(lib.list(&ClipQuery { sort: SortBy::Smallest, ..Default::default() }).unwrap()[0].id, a.id);
        assert_eq!(lib.list(&ClipQuery { search: Some("valo".into()), ..Default::default() }).unwrap().len(), 1);
        assert_eq!(
            lib.list(&ClipQuery { search: Some("100%".into()), ..Default::default() }).unwrap().len(),
            1,
            "LIKE wildcards are escaped"
        );
        assert_eq!(lib.list(&ClipQuery { search: Some("_".into()), ..Default::default() }).unwrap().len(), 1);
        assert_eq!(lib.list(&ClipQuery { game: Some("Valorant".into()), ..Default::default() }).unwrap().len(), 1);
        lib.set_favorite(b.id, true).unwrap();
        assert_eq!(lib.list(&ClipQuery { favorites_only: true, ..Default::default() }).unwrap()[0].id, b.id);
        let st = lib.stats().unwrap();
        assert_eq!((st.count, st.total_bytes), (2, 400));
        assert_eq!(st.games, vec![GameCount { game: "Valorant".into(), count: 1 }]);

        // rename renames the file
        let r = lib.rename(a.id, "Insane: 1v5 ace").unwrap();
        assert_eq!(r.path, clips.join("Valorant/Insane_ 1v5 ace.mp4"));
        assert!(r.path.exists() && !a.path.exists());
        // move
        let dest = clips.join("Best");
        let m = lib.move_to(a.id, &dest).unwrap();
        assert!(m.path.starts_with(&dest) && m.path.exists());
        // delete
        lib.delete(b.id, true).unwrap();
        assert!(!b.path.exists());
        assert!(matches!(lib.get(b.id), Err(LibraryError::NotFound(_))));
    }

    #[test]
    fn scan_syncs_with_folder() {
        let (_d, lib, clips) = setup();
        std::fs::write(clips.join("Valorant/external.mp4"), b"1234").unwrap();
        std::fs::write(clips.join("root.mkv"), b"12").unwrap();
        std::fs::write(clips.join("notes.txt"), b"x").unwrap();
        std::fs::write(clips.join(".hidden.mp4.part0.mp4"), b"x").unwrap();
        let probe = |_: &Path| Some(media(12.0));
        let r = lib.scan(&clips, &probe).unwrap();
        assert_eq!(r, ScanReport { added: 2, removed: 0, updated: 0 });
        let ext = lib.by_path(&clips.join("Valorant/external.mp4")).unwrap().unwrap();
        assert_eq!(ext.game.as_deref(), Some("Valorant"));
        assert_eq!(ext.duration, 12.0);
        std::fs::remove_file(clips.join("root.mkv")).unwrap();
        std::fs::write(clips.join("Valorant/external.mp4"), b"12345678").unwrap();
        let r = lib.scan(&clips, &probe).unwrap();
        assert_eq!(r, ScanReport { added: 0, removed: 1, updated: 1 });
        assert_eq!(lib.scan(&clips, &probe).unwrap(), ScanReport::default(), "idempotent");
    }

    #[test]
    fn migrations_are_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("l.db");
        drop(Library::open(&db, d.path()).unwrap());
        let lib = Library::open(&db, d.path()).unwrap();
        assert_eq!(lib.stats().unwrap().count, 0);
    }

    #[test]
    fn real_thumbnail() {
        let Ok(ff) = vinci_hw::ffmpeg::locate(None) else { return };
        let (_d, lib, clips) = setup();
        let p = clips.join("t.mp4");
        let ok = std::process::Command::new(&ff.path)
            .args([
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .arg(&p)
            .status()
            .unwrap();
        assert!(ok.success());
        let c = lib.insert(NewClip { path: p, title: "t".into(), created_at: 1, media: media(3.0), ..Default::default() }).unwrap();
        let t = lib.ensure_thumbnail(c.id, &ff.path).unwrap().unwrap();
        assert!(t.exists() && std::fs::metadata(&t).unwrap().len() > 1000);
        assert_eq!(lib.ensure_thumbnail(c.id, &ff.path).unwrap().unwrap(), t, "cached");
    }

    #[test]
    fn titles() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 11, 4, 21, 3, 15).unwrap();
        assert_eq!(default_title(Some("Valorant"), t), "Valorant — Nov 4, 21:03:15");
        assert_eq!(default_file_stem(None, t), "Clip 2026-11-04 21-03-15");
    }
}
