use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::ApiError;

pub const DEFAULT_ENTRY_BYTES: u64 = 256 << 20;
pub const DEFAULT_TOTAL_BYTES: u64 = 2 << 30;
pub const DEFAULT_JSON_BYTES: u64 = 16 << 20;
pub const STALE_SECONDS: u64 = 3600;
const PREFIX: &str = "gf-import-";
const MAX_CENTRAL_BYTES: u64 = 64 << 20;
static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct Caps {
    pub entry: u64,
    pub total: u64,
    pub json: u64,
}

fn ours(name: &str) -> bool {
    name.strip_prefix(PREFIX).and_then(|r| r.split_once('-')).is_some_and(|(a, b)| {
        let digits = |x: &str| !x.is_empty() && x.len() <= 20 && x.bytes().all(|c| c.is_ascii_digit());
        digits(a) && digits(b)
    })
}

pub fn sweep_stale(dir: &Path, older_than: std::time::Duration) -> Vec<String> {
    let mut removed = Vec::new();
    let now = std::time::SystemTime::now();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !ours(&name) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
        let old = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= older_than);
        if meta.is_dir() && !meta.file_type().is_symlink() && old && std::fs::remove_dir_all(e.path()).is_ok() {
            removed.push(name);
        }
    }
    removed
}

pub struct Extracted {
    pub dir: PathBuf,
    pub entries: Vec<(String, PathBuf, u64)>,
}

impl Drop for Extracted {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn le16(b: &[u8], i: usize) -> u64 {
    u64::from(u16::from_le_bytes([b[i], b[i + 1]]))
}

fn le32(b: &[u8], i: usize) -> u64 {
    u64::from(u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]))
}

fn le64(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]))
}

fn read_at(f: &mut File, at: u64, len: u64) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(at))?;
    let mut b = vec![0u8; len as usize];
    f.read_exact(&mut b)?;
    Ok(b)
}

pub fn central_names(f: &mut File) -> Result<Vec<Vec<u8>>, String> {
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len < 22 {
        return Err("too short to be a zip".into());
    }
    let tail_len = len.min(22 + 65_535 + 20);
    let tail = read_at(f, len - tail_len, tail_len).map_err(|e| e.to_string())?;
    let pos = (0..=tail.len() - 22).rev().find(|&i| tail[i..i + 4] == [0x50, 0x4b, 5, 6]).ok_or("no end of central directory record")?;
    let (mut count, mut size, mut offset) = (le16(&tail, pos + 10), le32(&tail, pos + 12), le32(&tail, pos + 16));
    if count == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        if pos < 20 || tail[pos - 20..pos - 16] != [0x50, 0x4b, 6, 7] {
            return Err("zip64 end record locator missing".into());
        }
        let at = le64(&tail, pos - 20 + 8);
        let rec = read_at(f, at, 56).map_err(|e| format!("zip64 end record: {e}"))?;
        if rec[0..4] != [0x50, 0x4b, 6, 6] {
            return Err("zip64 end record missing".into());
        }
        (count, size, offset) = (le64(&rec, 32), le64(&rec, 40), le64(&rec, 48));
    }
    if size > MAX_CENTRAL_BYTES || offset.checked_add(size).is_none_or(|end| end > len) {
        return Err(format!("central directory of {size} bytes at {offset} does not fit the file"));
    }
    let cd = read_at(f, offset, size).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    let mut i = 0usize;
    while (names.len() as u64) < count {
        if i + 46 > cd.len() || cd[i..i + 4] != [0x50, 0x4b, 1, 2] {
            return Err(format!("central directory entry {} is malformed", names.len()));
        }
        let (n, x, c) = (le16(&cd, i + 28) as usize, le16(&cd, i + 30) as usize, le16(&cd, i + 32) as usize);
        if i + 46 + n > cd.len() {
            return Err(format!("central directory entry {} is truncated", names.len()));
        }
        names.push(cd[i + 46..i + 46 + n].to_vec());
        i += 46 + n + x + c;
    }
    Ok(names)
}

pub fn bad_name(raw: &[u8]) -> Option<&'static str> {
    let Ok(s) = std::str::from_utf8(raw) else { return Some("is not UTF-8") };
    if s.is_empty() {
        Some("is empty")
    } else if s.starts_with('/') || s.starts_with('\\') {
        Some("is absolute")
    } else if s.contains('\\') {
        Some("contains a backslash")
    } else if s.contains(':') {
        Some("contains ':' (a drive letter or a stream name)")
    } else if s.contains('\0') {
        Some("contains a NUL byte")
    } else if s.contains("..") {
        Some("contains '..'")
    } else if s.split('/').any(|c| c.is_empty() || c == ".") {
        Some("has an empty or '.' path part")
    } else {
        None
    }
}

fn copy_entry(archive: &mut zip::ZipArchive<File>, i: usize, listed: &[u8], caps: &Caps, total: &mut u64, refuse: &dyn Fn(String) -> ApiError, sink: &mut dyn Write) -> Result<u64, ApiError> {
    let mut entry = archive.by_index(i).map_err(|e| refuse(format!("entry {i}: {e}")))?;
    let name = entry.name().to_string();
    if entry.name_raw() != listed {
        return Err(refuse(format!("entry {i} is named {name:?} in the archive index and {:?} in the central directory", String::from_utf8_lossy(listed))));
    }
    if entry.is_symlink() {
        return Err(refuse(format!("entry {name:?} is a symbolic link")));
    }
    if entry.is_dir() || entry.unix_mode().is_some_and(|m| !matches!(m & 0o170000, 0 | 0o100000)) {
        return Err(refuse(format!("entry {name:?} is a folder or not a regular file (mode {:o})", entry.unix_mode().unwrap_or(0))));
    }
    let (limit, what) = if name.ends_with(".json") { (caps.entry.min(caps.json), "max_project_bytes") } else { (caps.entry, "max_entry_bytes") };
    if entry.size() > limit {
        return Err(refuse(format!("entry {name:?} declares {} bytes, over the cap of {limit} ({what})", entry.size())));
    }
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        let k = entry.read(&mut buf).map_err(|e| refuse(format!("entry {name:?}: {e}")))?;
        if k == 0 {
            break;
        }
        n += k as u64;
        *total += k as u64;
        if n > limit {
            return Err(refuse(format!("entry {name:?} decompresses past the cap of {limit} bytes ({what}; its header says {})", entry.size())));
        }
        if *total > caps.total {
            return Err(refuse(format!("the entries decompress past the total cap of {} bytes (max_total_bytes) at {name:?}", caps.total)));
        }
        sink.write_all(&buf[..k]).map_err(|e| ApiError::internal(format!("{name}: {e}")))?;
    }
    Ok(n)
}

pub type Plan<'a> = dyn FnMut(&[u8]) -> Result<BTreeSet<String>, ApiError> + 'a;

pub fn extract(path: &Path, caps: &Caps, project: &str, plan: &mut Plan) -> Result<(Vec<u8>, Extracted), ApiError> {
    let shown = path.display().to_string();
    let refuse = |m: String| ApiError::invalid(format!("{shown}: {m}; the file was refused and nothing was imported"));
    let mut file = File::open(path).map_err(|e| ApiError::not_found(format!("{shown}: {e}")))?;
    let names = central_names(&mut file).map_err(|e| refuse(format!("not a readable zip ({e})")))?;
    let mut seen = BTreeSet::new();
    let mut texts = Vec::with_capacity(names.len());
    for raw in &names {
        let text = String::from_utf8_lossy(raw).to_string();
        if let Some(why) = bad_name(raw) {
            return Err(refuse(format!("entry {text:?} {why}")));
        }
        if !seen.insert(text.to_ascii_lowercase()) {
            return Err(refuse(format!("entry {text:?} appears more than once")));
        }
        texts.push(text);
    }
    let at = texts.iter().position(|n| n == project).ok_or_else(|| refuse(format!("it has no {project}")))?;
    file.seek(SeekFrom::Start(0)).map_err(|e| ApiError::internal(format!("{shown}: {e}")))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| refuse(format!("not a readable zip ({e})")))?;
    if archive.len() != names.len() {
        return Err(refuse(format!("its central directory lists {} entries, {} are readable", names.len(), archive.len())));
    }
    let mut total = 0u64;
    let mut doc = Vec::new();
    copy_entry(&mut archive, at, &names[at], caps, &mut total, &refuse, &mut doc)?;
    let allowed = plan(&doc)?;
    if texts.len() > allowed.len() {
        return Err(refuse(format!("it holds {} entries where the project can reference at most {} (the project, its index and the blobs its characters name)", texts.len(), allowed.len())));
    }
    if let Some(n) = texts.iter().find(|n| !allowed.contains(*n)) {
        return Err(refuse(if n.starts_with("blobs/") { format!("blob entry {n} is not referenced by any character") } else { format!("unexpected entry {n:?}") }));
    }
    sweep_stale(&std::env::temp_dir(), std::time::Duration::from_secs(STALE_SECONDS));
    let dir = std::env::temp_dir().join(format!("{PREFIX}{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    let mut out = Extracted { dir, entries: Vec::new() };
    for (i, listed) in names.iter().enumerate() {
        if i == at {
            continue;
        }
        let target = out.dir.join(format!("{i}"));
        let mut f = File::create(&target).map_err(|e| ApiError::internal(format!("{}: {e}", target.display())))?;
        let n = copy_entry(&mut archive, i, listed, caps, &mut total, &refuse, &mut f)?;
        out.entries.push((texts[i].clone(), target, n));
    }
    Ok((doc, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_entry_names_are_refused() {
        for bad in ["/abs", "\\abs", "a\\b", "C:x", "c:/x", "../x", "a/../b", "a/..", "a//b", "./a", "a/", "", "x\0y"] {
            assert!(bad_name(bad.as_bytes()).is_some(), "{bad:?}");
        }
        for good in ["project.gfproj.json", "portable.json", "blobs/0123abcd"] {
            assert!(bad_name(good.as_bytes()).is_none(), "{good:?}");
        }
        assert!(bad_name(&[0xff, 0xfe]).is_some());
    }

    #[test]
    fn entries_the_project_does_not_name_are_refused_before_any_is_extracted() {
        let top = std::env::temp_dir().join(format!("gf_plan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        std::fs::create_dir_all(&top).unwrap();
        let zp = top.join("t.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&zp).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            for (n, d) in [("project.gfproj.json", &b"{}"[..]), ("blobs/aa", b"x"), ("blobs/bb", b"y"), ("extra.txt", b"z")] {
                z.start_file(n, o).unwrap();
                z.write_all(d).unwrap();
            }
            z.finish().unwrap();
        }
        let caps = Caps { entry: 1 << 20, total: 1 << 20, json: 1 << 20 };
        let mut calls = 0;
        let mut seen = Vec::new();
        let mut plan = |b: &[u8]| {
            calls += 1;
            seen = b.to_vec();
            Ok(["project.gfproj.json", "blobs/aa", "blobs/bb", "blobs/cc"].into_iter().map(String::from).collect())
        };
        let e = extract(&zp, &caps, "project.gfproj.json", &mut plan).err().unwrap();
        assert!(e.message.contains("unexpected entry \"extra.txt\""), "{}", e.message);
        assert_eq!((calls, seen.as_slice()), (1, &b"{}"[..]));
        let mut few = |_: &[u8]| Ok(["project.gfproj.json", "blobs/aa"].into_iter().map(String::from).collect());
        assert!(extract(&zp, &caps, "project.gfproj.json", &mut few).err().unwrap().message.contains("at most 2"));
        let mut all = |_: &[u8]| Ok(["project.gfproj.json", "blobs/aa", "blobs/bb", "extra.txt"].into_iter().map(String::from).collect());
        let (doc, ex) = extract(&zp, &caps, "project.gfproj.json", &mut all).unwrap();
        assert_eq!((doc.as_slice(), ex.entries.len()), (&b"{}"[..], 3));
        drop(ex);
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn only_old_import_folders_named_like_ours_are_swept() {
        let top = std::env::temp_dir().join(format!("gf_sweep_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        for n in ["gf-import-1-2", "gf-import-x-1", "gf-import-3", "other-1-2"] {
            std::fs::create_dir_all(top.join(n)).unwrap();
            std::fs::write(top.join(n).join("0"), b"x").unwrap();
        }
        std::fs::write(top.join("gf-import-4-5"), b"file").unwrap();
        assert!(sweep_stale(&top, std::time::Duration::from_secs(3600)).is_empty());
        assert_eq!(sweep_stale(&top, std::time::Duration::ZERO), ["gf-import-1-2"]);
        for n in ["gf-import-x-1", "gf-import-3", "other-1-2", "gf-import-4-5"] {
            assert!(top.join(n).exists(), "{n}");
        }
        let _ = std::fs::remove_dir_all(&top);
    }
}
