use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use gf_core::hash;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::error::{ApiError, Code};

pub const STORE: &str = "store";
pub const OPTION_ROLES: [&str; 3] = ["macro", "eyes", "face_morphs"];
const SHORTCUTS: [&str; 5] = ["store", "macro", "eyes", "face_morphs", "body"];
const SKIPPED_NAMES: [&str; 3] = ["thumbs.db", "desktop.ini", "icon\r"];
const PORTABLE_FIELDS: [&str; 4] = ["kind", "identity", "file", "by"];

pub fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn fold(s: &str) -> String {
    if cfg!(windows) { s.to_ascii_lowercase() } else { s.to_string() }
}

pub fn hint(path: &Path, root: &Path) -> String {
    let (p, r) = (slashed(&absolute(path)), slashed(&absolute(root)));
    let r = r.trim_end_matches('/');
    let (fp, fr) = (fold(&p), fold(r));
    if fp.len() > fr.len() + 1 && fp.starts_with(&fr) && fp.as_bytes()[fr.len()] == b'/' {
        return p[r.len() + 1..].to_string();
    }
    p
}

pub fn file_name(r: &Value) -> String {
    if let Some(f) = r.get("file").and_then(Value::as_str) {
        return f.to_string();
    }
    let h = r.get("hint").and_then(Value::as_str).unwrap_or("").trim_end_matches(['/', '\\']);
    h.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

pub fn expected_kind(role: &str) -> Option<&'static str> {
    match role {
        STORE => Some("store"),
        "macro" | "eyes" => Some("file"),
        "face_morphs" => Some("dir"),
        _ => None,
    }
}

pub fn bare_name_problem(name: &str) -> Option<&'static str> {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_uppercase();
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|d| d.chars().count() == 1 && (d.chars().all(|c| c.is_ascii_digit()) || ["¹", "²", "³"].contains(&d)));
    if name.is_empty() {
        Some("is empty")
    } else if name.len() > 255 {
        Some("is longer than 255 bytes")
    } else if name.contains(['/', '\\']) {
        Some("holds a path separator (a bare file name was expected)")
    } else if name == "." || name.contains("..") {
        Some("holds '..' or is '.'")
    } else if name.contains(':') {
        Some("holds ':' (a drive or a stream name)")
    } else if name.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        Some("holds a character file names cannot hold")
    } else if name.ends_with(['.', ' ']) || name.starts_with(' ') {
        Some("starts with a space or ends with '.' or a space")
    } else if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str()) || numbered("COM") || numbered("LPT") {
        Some("is a reserved device name")
    } else {
        None
    }
}

fn simple(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

pub fn check_portable_refs(refs: &Map<String, Value>) -> Result<(), ApiError> {
    let mut bad = Vec::new();
    for (role, r) in refs {
        let mut why: Vec<String> = Vec::new();
        let known = role == STORE || OPTION_ROLES.contains(&role.as_str()) || role.strip_prefix("panels.").and_then(|x| x.split_once('.')).is_some_and(|(id, k)| simple(id) && simple(k));
        if !known {
            why.push("is not a known role".into());
        }
        match r.as_object() {
            None => why.push("is not an object".into()),
            Some(o) => {
                if let Some(k) = o.keys().find(|k| !PORTABLE_FIELDS.contains(&k.as_str())) {
                    why.push(format!("has the field {k:?} (a portable resource holds kind, identity and file only)"));
                }
                let kind = o.get("kind").and_then(Value::as_str).unwrap_or("");
                if !["store", "file", "dir"].contains(&kind) {
                    why.push(format!("kind {} is not store, file or dir", o.get("kind").unwrap_or(&Value::Null)));
                } else if let Some(want) = expected_kind(role)
                    && want != kind
                {
                    why.push(format!("kind {kind} where the role is a {want}"));
                }
                if o.get("identity").and_then(Value::as_str).is_none_or(|i| hash::parse_reference(i).is_err()) {
                    why.push("identity is not a sha256 reference".into());
                }
                match o.get("file") {
                    None => {}
                    Some(Value::String(f)) if f.is_empty() => {}
                    Some(Value::String(f)) => {
                        if let Some(p) = bare_name_problem(f) {
                            why.push(format!("file {f:?} {p}"));
                        }
                    }
                    Some(_) => why.push("file is not a string".into()),
                }
            }
        }
        if !why.is_empty() {
            bad.push(format!("{role}: {}", why.join(", ")));
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    Err(ApiError::invalid(format!("the portable project names {} unsafe or malformed resources, so nothing was looked up or written: {}", bad.len(), bad.join("; "))).details(json!({"refused": bad})))
}

pub fn file_identity(path: &Path) -> Result<String, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut f = std::fs::File::open(path).map_err(shown)?;
    let mut h = hash::Sha256Stream::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(shown)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hash::reference(&h.hex()))
}

fn walk(dir: &Path, rel: &str, chain: &mut Vec<PathBuf>, out: &mut Vec<(String, PathBuf, Option<u64>)>) -> Result<(), String> {
    let shown = |e: std::io::Error| format!("{}: {e}", dir.display());
    let real = std::fs::canonicalize(dir).map_err(shown)?;
    if chain.contains(&real) {
        return Err(format!("{}: a link inside the folder leads back to {} (a cycle), so its content cannot be identified", dir.display(), real.display()));
    }
    chain.push(real);
    for e in std::fs::read_dir(dir).map_err(shown)? {
        let e = e.map_err(shown)?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || SKIPPED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let p = e.path();
        let meta = if e.file_type().map_err(shown)?.is_symlink() { std::fs::metadata(&p).ok() } else { Some(e.metadata().map_err(shown)?) };
        match meta {
            None => out.push((r, p, None)),
            Some(m) if m.is_dir() => walk(&p, &r, chain, out)?,
            Some(m) if m.is_file() => out.push((r, p, Some(m.len()))),
            Some(_) => {}
        }
    }
    chain.pop();
    Ok(())
}

pub fn dir_identity(path: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    walk(path, "", &mut Vec::new(), &mut files)?;
    files.sort();
    let mut lines = String::new();
    for (rel, p, size) in &files {
        match size {
            Some(size) => lines.push_str(&format!("{rel} {size} {}\n", file_identity(p)?)),
            None => lines.push_str(&format!("{rel} - dangling link\n")),
        }
    }
    Ok(hash::reference(&hash::sha256_hex(lines.as_bytes())))
}

pub type StoreCheck<'a> = dyn FnMut(&Path) -> Result<String, ApiError> + 'a;

pub fn reference(role: &str, path: &Path, root: &Path, store_id: &mut StoreCheck) -> Result<Value, ApiError> {
    let shown = path.display();
    let (kind, identity) = if path.is_dir() {
        if path.join("packs").is_dir() {
            ("store", store_id(path).map_err(|e| ApiError { message: format!("{role}: {}", e.message), ..e })?)
        } else {
            ("dir", dir_identity(path).map_err(ApiError::internal)?)
        }
    } else if path.is_file() {
        ("file", file_identity(path).map_err(ApiError::internal)?)
    } else {
        return Err(ApiError::not_found(format!("{role}: {shown} does not exist")));
    };
    Ok(json!({"kind": kind, "identity": identity, "hint": hint(path, root)}))
}

pub fn unresolved_reference(role: &str, path: &Path, root: &Path, why: &str) -> Value {
    json!({"kind": expected_kind(role).unwrap_or("file"), "identity": null, "hint": hint(path, root), "unresolved": why})
}

pub fn portable(r: &Value) -> Value {
    json!({"kind": r["kind"], "identity": r["identity"], "file": file_name(r)})
}

pub enum Miss {
    Missing(String),
    Mismatch(String),
    Failed(ApiError),
}

impl Miss {
    fn text(&self) -> String {
        match self {
            Miss::Missing(m) | Miss::Mismatch(m) => m.clone(),
            Miss::Failed(e) => e.message.clone(),
        }
    }
}

fn candidate(r: &Value, p: &Path, store_check: &mut StoreCheck) -> Result<(), Miss> {
    if !p.exists() {
        return Err(Miss::Missing(format!("{} does not exist", p.display())));
    }
    let want = r.get("identity").and_then(Value::as_str);
    let have = match r.get("kind").and_then(Value::as_str).unwrap_or("file") {
        "store" => store_check(p).map_err(Miss::Failed)?,
        "dir" if !p.is_dir() => return Err(Miss::Mismatch(format!("{} is a file, the project expects a folder", p.display()))),
        "dir" => dir_identity(p).map_err(|m| Miss::Failed(ApiError::invalid(m)))?,
        _ if p.is_dir() => return Err(Miss::Mismatch(format!("{} is a folder, the project expects a file", p.display()))),
        _ => file_identity(p).map_err(Miss::Missing)?,
    };
    match want {
        Some(w) if w != have => Err(Miss::Mismatch(format!("{} is {have}, the project expects {w}", p.display()))),
        _ => Ok(()),
    }
}

pub fn explicit(input: &Value, role: &str) -> Option<String> {
    let given = |v: Option<&Value>| v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let last = role.rsplit('.').next().unwrap_or(role);
    given(input.pointer(&format!("/resources/{}", role.replace('~', "~0").replace('/', "~1"))))
        .or_else(|| given(input.get(role)).filter(|_| SHORTCUTS.contains(&role)))
        .or_else(|| given(input.get(last)).filter(|_| SHORTCUTS.contains(&last)))
}

pub fn search_dirs(input: &Value) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = input.get("search").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect();
    if let Some(env) = std::env::var_os("GAZEFORGE_RESOURCES") {
        dirs.extend(std::env::split_paths(&env).filter(|p| !p.as_os_str().is_empty()));
    }
    dirs
}

pub struct Resolved {
    pub paths: BTreeMap<String, PathBuf>,
    pub report: Map<String, Value>,
}

pub fn resolve_all(refs: &Map<String, Value>, input: &Value, root: &Path, from_zip: bool, store_check: &mut StoreCheck) -> Result<Resolved, ApiError> {
    let search = search_dirs(input);
    let skip: Vec<&str> = input.get("skip").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let mut out = Resolved { paths: BTreeMap::new(), report: Map::new() };
    let mut failed = Vec::new();
    let mut conflict = false;
    for (role, r) in refs {
        let mut tried: Vec<(PathBuf, &str)> = Vec::new();
        let given = explicit(input, role);
        if !from_zip
            && given.is_none()
            && let Some(why) = r.get("unresolved").and_then(Value::as_str)
        {
            let shown = r.get("hint").and_then(Value::as_str).unwrap_or("");
            out.report.insert(role.clone(), json!({"unresolved": why, "hint": shown, "from": "autosave"}));
            if !shown.is_empty() {
                let p = Path::new(shown);
                out.paths.insert(role.clone(), if p.is_absolute() { p.to_path_buf() } else { root.join(p) });
            }
            continue;
        }
        match given {
            Some(p) => tried.push((PathBuf::from(p), "option")),
            None => {
                let name = file_name(r);
                if !name.is_empty() && bare_name_problem(&name).is_none() && r.get("identity").is_some_and(|i| !i.is_null()) {
                    for d in &search {
                        tried.push((d.join(&name), "search"));
                    }
                    if !from_zip {
                        tried.push((root.join(&name), "root"));
                    }
                }
                if !from_zip && let Some(h) = r.get("hint").and_then(Value::as_str).filter(|h| !h.is_empty()) {
                    let p = Path::new(h);
                    tried.push((if p.is_absolute() { p.to_path_buf() } else { root.join(p) }, "hint"));
                }
            }
        }
        let mut misses = Vec::new();
        let mut found = None;
        for (p, from) in &tried {
            match candidate(r, p, store_check) {
                Ok(()) => {
                    found = Some((p.clone(), *from));
                    break;
                }
                Err(m) => misses.push((p.clone(), *from, m)),
            }
        }
        match found {
            Some((p, from)) => {
                out.report.insert(role.clone(), json!({"path": p.display().to_string(), "from": from, "identity": r["identity"]}));
                out.paths.insert(role.clone(), p);
            }
            None if skip.contains(&role.as_str()) && role != STORE => {
                out.report.insert(role.clone(), json!({"skipped": true, "identity": r["identity"]}));
            }
            None => {
                let shown: Vec<_> = misses.iter().filter(|(_, _, m)| !matches!(m, Miss::Missing(_))).collect();
                conflict |= misses.iter().any(|(_, _, m)| match m {
                    Miss::Mismatch(_) => true,
                    Miss::Failed(e) => e.code == Code::Conflict,
                    Miss::Missing(_) => false,
                });
                let reason = if tried.is_empty() {
                    "no path recorded and none given".to_string()
                } else if shown.is_empty() {
                    format!("not found (tried {})", tried.iter().map(|(p, f)| format!("{f} {}", p.display())).collect::<Vec<_>>().join(", "))
                } else {
                    shown.iter().map(|(_, f, m)| format!("{f}: {}", m.text())).collect::<Vec<_>>().join("; ")
                };
                failed.push(json!({"role": role, "kind": r["kind"], "file": file_name(r), "identity": r["identity"], "reason": reason, "optional": role != STORE}));
            }
        }
    }
    if failed.is_empty() {
        return Ok(out);
    }
    let roles: Vec<String> = failed.iter().map(|f| format!("{} ({})", f["role"].as_str().unwrap_or(""), f["reason"].as_str().unwrap_or(""))).collect();
    let where_ = if from_zip { "search: [folders holding them by file name]" } else { "search: [folders holding them by file name] (the projects folder and the saved location are tried too)" };
    Err(ApiError::new(if conflict { Code::Conflict } else { Code::NotFound }, format!("{} of the project's resources could not be resolved: {}", failed.len(), roles.join("; ")))
        .hint(format!("pass each as an option (store, macro, eyes, face_morphs, body, or resources: {{role: path}}), or {where_}; skip: [roles] opens without the optional ones"))
        .details(json!({"unresolved": failed})))
}

pub struct Hit {
    pub pointer: String,
    pub why: String,
    pub fragment: String,
}

pub struct Scan {
    needles: Vec<(String, &'static str)>,
    words: Vec<(String, &'static str)>,
}

fn invisible(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{34F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

fn lookalike(c: char) -> char {
    match c {
        '\u{2044}' | '\u{2215}' | '\u{2571}' | '\u{27CB}' | '\u{29F8}' | '\u{1735}' | '\u{2F03}' | '\u{3033}' | '\u{30CE}' | '\u{4E3F}' => '/',
        '\u{2216}' | '\u{2572}' | '\u{27CD}' | '\u{29F5}' | '\u{29F9}' | '\u{FE68}' => '\\',
        '\u{2D0}' | '\u{589}' | '\u{5C3}' | '\u{2236}' | '\u{A789}' | '\u{FE13}' => ':',
        c if c.is_whitespace() => ' ',
        c => c,
    }
}

fn percent_decoded(s: &str) -> Option<String> {
    if !s.contains('%') {
        return None;
    }
    let b = s.as_bytes();
    let hex = |i: usize| b.get(i).and_then(|&c| char::from(c).to_digit(16));
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    (out.len() != b.len()).then(|| String::from_utf8_lossy(&out).into_owned())
}

pub fn normalised(s: &str) -> String {
    let mut cur = s.to_string();
    for _ in 0..4 {
        let mut next = cur.clone();
        for _ in 0..8 {
            match percent_decoded(&next) {
                Some(d) => next = d,
                None => break,
            }
        }
        let next: String = next.nfkc().filter(|c| !invisible(*c)).map(lookalike).collect();
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn push_word(words: &mut Vec<(String, &'static str)>, v: &str, why: &'static str) {
    let v = normalised(v);
    let v = v.trim();
    if v.chars().count() >= 2 && !words.iter().any(|(w, _)| w.to_lowercase() == v.to_lowercase()) {
        words.push((v.to_string(), why));
    }
}

pub fn scanner(root: &Path) -> Scan {
    let mut needles = Vec::new();
    let mut add = |p: Option<PathBuf>, why: &'static str| {
        if let Some(p) = p {
            let s = normalised(&slashed(&absolute(&p))).trim_end_matches('/').to_ascii_lowercase();
            if s.len() > 3 {
                needles.push((s, why));
            }
        }
    };
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    add(Some(root.to_path_buf()), "the projects folder");
    add(home.clone(), "the home folder");
    add(Some(std::env::temp_dir()), "the temp folder");
    add(std::env::current_dir().ok(), "the working folder");
    let mut words = Vec::new();
    for k in ["USERNAME", "USER", "LOGNAME"] {
        if let Ok(v) = std::env::var(k) {
            push_word(&mut words, &v, "the user name");
        }
    }
    if let Some(n) = home.as_deref().and_then(Path::file_name).and_then(|n| n.to_str()) {
        push_word(&mut words, n, "the user name");
    }
    for k in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(k) {
            push_word(&mut words, &v, "the host name");
        }
    }
    Scan { needles, words }
}

fn token(s: &str, start: usize, end: usize) -> String {
    let a = s[..start].char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(i, c)| i + c.len_utf8());
    let b = s[end..].char_indices().find(|(_, c)| c.is_whitespace()).map_or(s.len(), |(i, _)| end + i);
    let t = &s[a..b];
    if t.chars().count() > 160 { t.chars().take(160).collect::<String>() + "..." } else { t.to_string() }
}

fn joined(edge: char, next: char) -> bool {
    (edge.is_alphabetic() && next.is_alphabetic()) || (edge.is_numeric() && next.is_numeric())
}

fn find_word(s: &str, word: &str) -> Option<(usize, usize)> {
    let w: Vec<char> = word.chars().collect();
    let (first, last) = (*w.first()?, *w.last()?);
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let mut prev: Option<char> = None;
    for (st, c0) in s.char_indices() {
        if same(c0, first) && !prev.is_some_and(|p| joined(first, p)) {
            let mut it = s[st..].char_indices();
            if w.iter().all(|&wc| it.next().is_some_and(|(_, c)| same(c, wc))) {
                let after = it.next();
                if !after.is_some_and(|(_, c)| joined(last, c)) {
                    return Some((st, after.map_or(s.len(), |(k, _)| st + k)));
                }
            }
        }
        prev = Some(c0);
    }
    None
}

fn name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b'~' | b'$' | b'%' | b'@') || c >= 0x80
}

pub fn local_reason(s: &str, scan: &Scan) -> Option<(String, String)> {
    if let Some(r) = reason_in(s, scan) {
        return Some(r);
    }
    if s.is_ascii() && !s.contains('%') {
        return None;
    }
    let n = normalised(s);
    if n == s {
        return None;
    }
    reason_in(&n, scan).map(|(why, fragment)| (format!("{why}, once decoded and normalised"), fragment))
}

fn reason_in(s: &str, scan: &Scan) -> Option<(String, String)> {
    let b = s.as_bytes();
    let low = s.to_ascii_lowercase();
    let hit = |why: &str, a: usize, e: usize| Some((why.to_string(), token(s, a, e.min(s.len()))));
    for (i, &c) in b.iter().enumerate() {
        if !c.is_ascii() {
            continue;
        }
        let at = |k: usize| b.get(i + k).copied().unwrap_or(0);
        let prev = s[..i].chars().next_back();
        let starts = prev.is_none_or(|p| p.is_whitespace() || !(p.is_alphanumeric() || "._-/\\:%~$@)]}+#&!?^".contains(p)));
        if c.is_ascii_alphabetic() && at(1) == b':' {
            if matches!(at(2), b'/' | b'\\') {
                let from = low[..i].rfind(|ch: char| !(ch.is_ascii_alphanumeric() || "+-.".contains(ch))).map_or(0, |k| k + 1);
                let scheme = &low[from..=i];
                if scheme == "file" {
                    return hit("a file URL", i, i + 3);
                }
                if scheme.len() == 1 {
                    return hit("a drive path", i, i + 3);
                }
                if at(2) != b'/' || at(3) != b'/' {
                    return hit("a path after a word and ':'", from, i + 3);
                }
                if scheme != "http" && scheme != "https" && (name_byte(at(4)) || at(4) == b'[') {
                    return hit(&format!("a {scheme}:// URL"), from, i + 4);
                }
            } else if prev.is_none_or(|p| !p.is_alphanumeric()) && name_byte(at(2)) {
                let end = b[i..].iter().position(u8::is_ascii_whitespace).map_or(b.len(), |k| i + k);
                if b[i + 2..end].iter().any(|x| matches!(x, b'/' | b'\\')) {
                    return hit("a drive-relative path", i, end);
                }
            }
        }
        if c == b'\\' && at(1) == b'\\' && prev != Some('\\') && (at(2).is_ascii_alphanumeric() || matches!(at(2), b'?' | b'.') || at(2) >= 0x80) {
            return hit("a network path", i, i + 2);
        }
        if c == b'/' && at(1) == b'/' && (at(2).is_ascii_alphanumeric() || at(2) >= 0x80) && prev.is_none_or(|p| !(p.is_alphanumeric() || p == ':' || p == '/')) {
            return hit("a network path", i, i + 2);
        }
        if c == b'~' && starts {
            let n = b[i + 1..].iter().take_while(|x| x.is_ascii_alphanumeric() || matches!(x, b'_' | b'.' | b'-')).count();
            if matches!(at(1 + n), b'/' | b'\\') {
                return hit("a home path", i, i + 2 + n);
            }
        }
        if c == b'%'
            && let Some(k) = s[i + 1..].find('%')
        {
            let name = &s[i + 1..i + 1 + k];
            if (1..=64).contains(&name.len())
                && name.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
                && name.chars().all(|ch| ch.is_ascii_alphanumeric() || "_()".contains(ch))
            {
                return hit("an environment-variable path", i, i + k + 2);
            }
        }
        if c == b'$' {
            let n = b[i + 1..].iter().take_while(|x| x.is_ascii_alphanumeric() || **x == b'_').count();
            if at(1) == b'{' || low[i + 1..].starts_with("env:") || (n > 0 && b[i + 1].is_ascii_alphabetic() && matches!(at(1 + n), b'/' | b'\\')) {
                return hit("an environment-variable path", i, i + 1 + n);
            }
        }
        if c == b'\\' && starts && at(1) != b'\\' && name_byte(at(1)) {
            let end = b[i..].iter().position(u8::is_ascii_whitespace).map_or(b.len(), |k| i + k);
            if b[i + 1..end].iter().any(|x| matches!(x, b'/' | b'\\')) {
                return hit("a backslash-rooted path", i, end);
            }
        }
        if c == b'/' && starts && at(1) != b'/' && name_byte(at(1)) {
            return hit("an absolute path", i, i + 1);
        }
    }
    let flat = low.replace('\\', "/");
    for pat in ["/users/", "/home/", "/documents and settings/"] {
        if let Some(k) = flat.find(pat) {
            return hit(&format!("a user folder ({})", pat.trim_matches('/')), k, k + pat.len());
        }
    }
    for (n, why) in &scan.needles {
        if let Some(k) = flat.find(n.as_str()) {
            return hit(why, k, k + n.len());
        }
    }
    for (w, why) in &scan.words {
        if let Some((a, e)) = find_word(s, w) {
            return hit(why, a, e);
        }
    }
    None
}

fn pointer_part(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

pub fn private_hits(value: &Value, root: &Path) -> Vec<Hit> {
    hits_with(value, &scanner(root))
}

pub fn hits_with(value: &Value, scan: &Scan) -> Vec<Hit> {
    fn walk(v: &Value, at: String, scan: &Scan, hits: &mut Vec<Hit>) {
        match v {
            Value::String(s) => {
                if let Some((why, fragment)) = local_reason(s, scan) {
                    hits.push(Hit { pointer: at, why, fragment });
                }
            }
            Value::Array(a) => {
                for (k, x) in a.iter().enumerate() {
                    walk(x, format!("{at}/{k}"), scan, hits);
                }
            }
            Value::Object(m) => {
                for (k, x) in m {
                    let here = format!("{at}/{}", pointer_part(k));
                    if let Some((why, fragment)) = local_reason(k, scan) {
                        hits.push(Hit { pointer: here.clone(), why: format!("{why} in a key"), fragment });
                    }
                    walk(x, here, scan, hits);
                }
            }
            _ => {}
        }
    }
    let mut hits = Vec::new();
    walk(value, String::new(), scan, &mut hits);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan() -> Scan {
        Scan { needles: vec![("q:/gf_scan_root".into(), "the projects folder")], words: vec![("Ürsula Bøb".into(), "the user name"), ("ws-ursula01".into(), "the host name")] }
    }

    #[test]
    fn hints_are_relative_under_the_root_and_absolute_elsewhere() {
        let root = std::env::temp_dir().join("gf_hint_root");
        assert_eq!(hint(&root.join("assets").join("eyes.json"), &root), "assets/eyes.json");
        let outside = std::env::temp_dir().join("gf_hint_other").join("x.json");
        assert_eq!(hint(&outside, &root), slashed(&absolute(&outside)));
        assert_eq!(file_name(&json!({"hint": "a/b/eyes.json"})), "eyes.json");
        assert_eq!(file_name(&json!({"hint": "Q:/data/body/"})), "body");
    }

    #[test]
    fn local_paths_and_user_details_are_found_anywhere_in_a_string() {
        let s = scan();
        let found = [
            "C:/x/y", "/home/q", "see \\\\server\\share", "copied from //nas01/share/u/chars", "copied from /q/data/out/chars", "eyes from D:chars\\eyes.json",
            "made by Ürsula Bøb on a box", "made by ürsula bøb", "on ws-ursula01 today", "see share/Ürsula Bøb/res", "\\\\?\\Q:\\data\\x", "file:///q:/data/x",
            "kept in ~/chars", "%USERPROFILE%\\chars", "on /Volumes/U/chars", "on /mnt/c/Users2/u", "abcC:/Windows", "$HOME/chars", "(see /tmp/x)", "Q:/GF_SCAN_ROOT/p",
        ];
        for t in found {
            assert!(local_reason(t, &s).is_some(), "{t:?} was not found");
        }
        let fine = [
            "faceunits/", "http://example.org/a", "notes about users", "nose/nose-scale-horiz-incr", "sha256:00ab", "2026-10-19T05:27:48.123Z", "a/b", "and/or",
            "16:9", "50% done", "Ursula", "ws-ursula011", "gazeforge.character/1", "",
        ];
        for t in fine {
            assert!(local_reason(t, &s).is_none(), "{t:?} was flagged: {:?}", local_reason(t, &s));
        }
        let (why, frag) = local_reason("copied from \\\\fileserver\\u\\chars before", &s).unwrap();
        assert_eq!((why.as_str(), frag.as_str()), ("a network path", "\\\\fileserver\\u\\chars"));
        let doc = json!({"notes": "x C:/a", "weights": {"a/b": 1, "C:/k": 2}, "list": ["ok", "/home/q"]});
        let mut pointers: Vec<String> = hits_with(&doc, &s).into_iter().map(|h| h.pointer).collect();
        pointers.sort();
        assert_eq!(pointers, ["/list/1", "/notes", "/weights/C:~1k"]);
    }

    #[test]
    fn obfuscated_paths_and_names_are_found_once_normalised() {
        let mut s = scan();
        s.words.push(("qhome".into(), "the user name"));
        let found = [
            "copied from \\data\\gf\\out\\chars", "\\data\\gf\\out", "C%3A%5Cdata%5Cx", "file:%2F%2F%2FC%3A%2Fx", "C%253A%255Cx", "C%3A%5CUsers%5Cqhome%5Cchars",
            "smb://nas01/share/x", "nfs://10.0.0.5/export", "sftp://h/x", "git+ssh://h/x", "C:\u{2215}data\u{2215}x", "\u{FF3C}\u{FF3C}fileserver\u{FF3C}share",
            "C\u{FF1A}/x", "C:\u{29F5}x", "made by U\u{308}rsula Bøb", "made by Ürsula\u{A0}Bøb", "made by Ürs\u{200B}ula Bøb", "backup_qhome_2026", "C_Users_qhome",
            "qhome2026", "by QHOME", "\u{2192}/srv/data/chars", "\u{AB}/opt/secret/chars\u{BB}", "ws-ursula01.corp.example", "\u{FF0F}srv\u{FF0F}x",
        ];
        for t in found {
            assert!(local_reason(t, &s).is_some(), "{t:?} was not found");
        }
        for t in ["https://example.org/a/b", "http://example.org", "ws-ursula011", "qhomes", "aqhome", "1/2", "and/or", "50% done", "100%", "a\\b", "x (1)/2"] {
            assert!(local_reason(t, &s).is_none(), "{t:?} was flagged: {:?}", local_reason(t, &s));
        }
        assert_eq!(local_reason("path:/srv/team/chars", &s).unwrap().0, "a path after a word and ':'");
        assert_eq!(local_reason("smb://nas01/share", &s).unwrap().0, "a smb:// URL");
        assert_eq!(local_reason("C%3A%5Cdata", &s).unwrap().0, "a drive path, once decoded and normalised");
        assert_eq!(normalised("a\u{A0}b\u{2009}c"), "a b c");
    }

    #[test]
    fn folder_identity_follows_links_and_refuses_cycles() {
        let top = std::env::temp_dir().join(format!("gf_link_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&top);
        let (real, copy, linked) = (top.join("real"), top.join("copy"), top.join("linked"));
        for d in [real.join("sub"), copy.join("sub"), linked.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        for d in [&real, &copy] {
            std::fs::write(d.join("sub").join("a.npz"), b"abc").unwrap();
        }
        std::fs::write(linked.join("b.npz"), b"b").unwrap();
        std::fs::write(copy.join("b.npz"), b"b").unwrap();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(real.join("b.npz").with_file_name("lnk")).arg(&linked).output().is_ok_and(|o| o.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&linked, real.join("lnk")).is_ok();
        if made {
            std::fs::rename(copy.join("b.npz"), copy.join("lnk_b")).unwrap();
            std::fs::create_dir_all(copy.join("lnk")).unwrap();
            std::fs::rename(copy.join("lnk_b"), copy.join("lnk").join("b.npz")).unwrap();
            assert_eq!(dir_identity(&real).unwrap(), dir_identity(&copy).unwrap());
            std::fs::write(linked.join("b.npz"), b"c").unwrap();
            assert_ne!(dir_identity(&real).unwrap(), dir_identity(&copy).unwrap());
            #[cfg(windows)]
            let cyc = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(linked.join("back")).arg(&real).output().is_ok_and(|o| o.status.success());
            #[cfg(unix)]
            let cyc = std::os::unix::fs::symlink(&real, linked.join("back")).is_ok();
            if cyc {
                assert!(dir_identity(&real).unwrap_err().contains("cycle"));
                let _ = std::fs::remove_dir(linked.join("back"));
            }
            let _ = std::fs::remove_dir(real.join("lnk"));
            let _ = std::fs::remove_file(real.join("lnk"));
        }
        let _ = std::fs::remove_dir_all(&top);
        assert!(made, "no link could be made to test with");
    }

    #[test]
    fn only_bare_file_names_pass() {
        for bad in ["", ".", "..", "a/b", "a\\b", "C:", "C:x", "x:y.json", "../x", "con", "CON.json", "Com1", "lpt9.txt", "COM¹", "nul.txt", "a..b", "x ", "x.", "a\u{0}b", "a?b"] {
            assert!(bare_name_problem(bad).is_some(), "{bad:?}");
        }
        for good in ["eyes.json", "eyes ✓.json", "body", "macro.json", "console.json", "com10"] {
            assert!(bare_name_problem(good).is_none(), "{good:?}");
        }
    }
}
