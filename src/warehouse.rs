//! The warehouse: a folder of pending items, one directory per item.
//!
//! ```text
//! <warehouse>/<id>/meta.json
//! <warehouse>/<id>/text            (kind = text)
//! <warehouse>/<id>/payload/<name>  (kind = file, may be a directory)
//! ```
//!
//! IDs start with a zero-padded millisecond timestamp, so lexical order is
//! chronological. Entries starting with `.` (e.g. `.incoming`) are ignored.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub kind: Kind,
    /// File name for files; `None` for text.
    pub name: Option<String>,
    pub is_dir: bool,
    pub size: u64,
    pub from: String,
    /// Unix millis.
    pub sent_at: u64,
    /// First line of text, for display.
    pub preview: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub dir: PathBuf,
    pub meta: Meta,
}

impl Item {
    pub fn payload_path(&self) -> PathBuf {
        match self.meta.kind {
            Kind::Text => self.dir.join("text"),
            Kind::File => self
                .dir
                .join("payload")
                .join(self.meta.name.as_deref().unwrap_or("unnamed")),
        }
    }

    pub fn read_text(&self) -> Result<String> {
        fs::read_to_string(self.payload_path()).context("reading text item")
    }

    pub fn remove(&self) -> Result<()> {
        fs::remove_dir_all(&self.dir).with_context(|| format!("removing {}", self.dir.display()))
    }

    /// Move a file item into `dest_dir` without clobbering anything there.
    /// Returns the final path.
    pub fn move_into(&self, dest_dir: &Path) -> Result<PathBuf> {
        let src = self.payload_path();
        let name = self.meta.name.as_deref().unwrap_or("unnamed");
        let dest = unique_dest(dest_dir, name);
        move_path(&src, &dest)?;
        self.remove()?;
        Ok(dest)
    }
}

pub fn root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("YEET_WAREHOUSE") {
        return Ok(PathBuf::from(dir));
    }
    Ok(yeet_home()?.join("warehouse"))
}

pub fn yeet_home() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("cannot determine home directory")?
        .join(".yeet"))
}

/// All pending items, newest first.
pub fn list() -> Result<Vec<Item>> {
    let root = root()?;
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", root.display())),
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if id.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        // Skip anything that isn't a complete item rather than failing the whole listing.
        let Ok(raw) = fs::read(dir.join("meta.json")) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<Meta>(&raw) else {
            continue;
        };
        items.push(Item { id, dir, meta });
    }
    items.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(items)
}

pub fn new_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let salt = (now.subsec_nanos() ^ std::process::id().rotate_left(16)) & 0xffff;
    format!("{:013}-{:04x}", now.as_millis(), salt)
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .expect("unbounded search")
}

/// Rename, falling back to copy + delete across filesystems.
pub fn move_path(src: &Path, dest: &Path) -> Result<()> {
    match fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy_tree(src, dest, false)?;
            if src.is_dir() {
                fs::remove_dir_all(src)?;
            } else {
                fs::remove_file(src)?;
            }
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("moving to {}", dest.display())),
    }
}

/// Recursively copy `src` to `dest`, hardlinking files when `link` is set
/// (falling back to a real copy when hardlinking fails).
pub fn copy_tree(src: &Path, dest: &Path, link: bool) -> Result<()> {
    let md = fs::metadata(src).with_context(|| format!("reading {}", src.display()))?;
    if md.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()), link)?;
        }
    } else if md.is_file() {
        if !(link && fs::hard_link(src, dest).is_ok()) {
            fs::copy(src, dest).with_context(|| format!("copying {}", src.display()))?;
        }
    } else {
        bail!("unsupported file type: {}", src.display());
    }
    Ok(())
}

pub fn tree_size(path: &Path) -> u64 {
    let Ok(md) = fs::metadata(path) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| tree_size(&e.path()))
        .sum()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

pub fn human_age(sent_at: u64) -> String {
    let secs = now_millis().saturating_sub(sent_at) / 1000;
    match secs {
        0..=59 => format!("{secs}s ago"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// One-line human label used by `yeet list` and the TUI.
pub fn describe(item: &Item) -> String {
    match item.meta.kind {
        Kind::File => {
            let name = item.meta.name.as_deref().unwrap_or("unnamed");
            let slash = if item.meta.is_dir { "/" } else { "" };
            format!("{name}{slash}")
        }
        Kind::Text => format!("\"{}\"", item.meta.preview.as_deref().unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_dest_avoids_clobbering() {
        let dir = std::env::temp_dir().join(format!("yeet-test-{}", new_id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_dest(&dir, "a.txt"), dir.join("a.txt"));
        fs::write(dir.join("a.txt"), "x").unwrap();
        assert_eq!(unique_dest(&dir, "a.txt"), dir.join("a (1).txt"));
        fs::write(dir.join("a (1).txt"), "x").unwrap();
        assert_eq!(unique_dest(&dir, "a.txt"), dir.join("a (2).txt"));
        fs::write(dir.join(".rc"), "x").unwrap();
        assert_eq!(unique_dest(&dir, ".rc"), dir.join(".rc (1)"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ids_sort_chronologically() {
        let a = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = new_id();
        assert!(b > a);
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(1536), "1.5 KB");
    }
}
