use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gf_core::catalog::{CatalogManifest, GameManifest, PackManifest};
use gf_core::character::{FORMAT as SPEC_FORMAT, Spec, SpecBase};
use gf_core::hash;
use gf_io::character as ch;
use gf_io::store::Store;
use serde_json::{Map, Value, json};

use crate::commands::{obj, shape, text};
use crate::error::{ApiError, Code};
use crate::resources::{self as res, OPTION_ROLES, STORE};
use crate::{exactjson, zipsafe};
use crate::{Ctx, designer};

pub const FORMAT: &str = "gazeforge.project/1";
pub const PORTABLE_FORMAT: &str = "gazeforge.project-portable/1";
pub const EXT: &str = ".gfproj.json";
pub const UNTITLED: &str = "untitled";
const ZIP_PROJECT: &str = "project.gfproj.json";
const ZIP_INDEX: &str = "portable.json";
const HISTORY_LIMIT: usize = 200;
static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn default_root() -> PathBuf {
    if let Some(p) = std::env::var_os("GAZEFORGE_PROJECTS").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let data = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".local").join("share")))
    };
    data.unwrap_or_else(std::env::temp_dir).join("GazeForge").join("projects")
}

fn root(ctx: &Ctx, input: &Value) -> PathBuf {
    match input.get("root").and_then(Value::as_str) {
        Some(r) if !r.is_empty() => PathBuf::from(r),
        _ => ctx.projects.clone(),
    }
}

pub fn check_name(name: &str, allow_untitled: bool) -> Result<(), ApiError> {
    let ok = (1..=64).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'))
        && !name.starts_with(['.', ' ', '-'])
        && !name.ends_with(['.', ' ']);
    if !ok {
        return Err(ApiError::invalid(format!(
            "project name {name:?}: use 1 to 64 letters, digits, spaces, '_', '-' or '.', not starting with '.', ' ' or '-' and not ending with '.' or ' '"
        )));
    }
    let stem = name.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let device = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if device {
        return Err(ApiError::invalid(format!("project name {name:?} is a reserved device name on some systems")));
    }
    if !allow_untitled && name.eq_ignore_ascii_case(UNTITLED) {
        return Err(ApiError::invalid(format!("{UNTITLED:?} is kept for the recovery slot of an unsaved session; choose another name")));
    }
    Ok(())
}

pub fn project_path(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}{EXT}"))
}

pub fn recovery_path(root: &Path, name: &str) -> PathBuf {
    root.join(".recovery").join(format!("{name}{EXT}"))
}

pub fn now_utc() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, d.subsec_millis())
}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> ApiError + '_ {
    move |e| ApiError::internal(format!("{}: {e}", path.display()))
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{base}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(&tmp)(e));
    }
    let mut last = None;
    for attempt in 0..60u64 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(5 * (attempt + 1).min(10)));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(match last {
        Some(e) => io_err(path)(e),
        None => ApiError::internal(format!("{}: could not be replaced", path.display())),
    })
}

fn read_retry(path: &Path) -> Result<Vec<u8>, ApiError> {
    let mut last = None;
    for attempt in 0..30u64 {
        match std::fs::read(path) {
            Ok(b) => return Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ApiError::not_found(format!("{} does not exist", path.display())));
            }
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(5 * (attempt + 1).min(10)));
            }
        }
    }
    Err(match last {
        Some(e) => io_err(path)(e),
        None => ApiError::internal(format!("{}: unreadable", path.display())),
    })
}

fn parse_doc(bytes: &[u8], what: &str) -> Result<Value, ApiError> {
    let doc = exactjson::parse(bytes, &exactjson::Limits::for_bytes(bytes.len())).map_err(|e| ApiError::invalid(format!("{what} is not JSON this loader accepts: {e}")))?;
    if doc.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ApiError::invalid(format!("{what} is not a {FORMAT} project (format {})", doc.get("format").unwrap_or(&Value::Null))));
    }
    Ok(doc)
}

pub fn read_doc(path: &Path) -> Result<Value, ApiError> {
    parse_doc(&read_retry(path)?, &path.display().to_string())
}

fn open_store(path: &Path) -> Result<Store, ApiError> {
    if path.as_os_str().is_empty() {
        return Err(ApiError::invalid("the project names no store; pass store"));
    }
    if !path.join("packs").is_dir() {
        return Err(ApiError::not_found(format!("{} is not a store", path.display())).hint("pass store: the directory holding packs/, catalogs/, game/ and blobs/"));
    }
    Store::open(path).map_err(ApiError::internal)
}

fn macro_source(path: Option<&Path>) -> Result<Option<ch::MacroSource>, ApiError> {
    match path {
        Some(p) if p.is_file() => ch::read_macro_definition(p).map(Some).map_err(ApiError::invalid),
        _ => Ok(None),
    }
}

fn refs_of(doc: &Value) -> Map<String, Value> {
    if let Some(r) = doc.get("resources").and_then(Value::as_object) {
        return r.clone();
    }
    let mut m = Map::new();
    if let Some(s) = doc.get("store").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        m.insert(STORE.into(), json!({"kind": "store", "identity": doc.pointer("/base/fingerprint"), "hint": s}));
    }
    for k in OPTION_ROLES {
        if let Some(p) = doc.get("options").and_then(|o| o.get(k)).and_then(Value::as_str).filter(|s| !s.is_empty()) {
            m.insert(k.into(), json!({"kind": if Path::new(p).is_dir() { "dir" } else { "file" }, "identity": null, "hint": p}));
        }
    }
    m
}

fn api_text(s: &str) -> String {
    s.replace(" (--macro)", " (macro)")
}

fn from_portable(mut doc: Value) -> Value {
    let refs = match doc.get("resources").and_then(Value::as_object) {
        Some(r) => r.clone(),
        None => {
            let mut m = Map::new();
            m.insert(STORE.into(), json!({"kind": "store", "identity": doc.pointer("/base/fingerprint"), "file": ""}));
            m
        }
    };
    doc["resources"] = Value::Object(refs);
    doc["options"] = Value::Object(plain_options(doc.get("options")));
    if let Some(o) = doc.as_object_mut() {
        o.remove("store");
        o.remove("recovery");
    }
    if let Some(panels) = doc.get_mut("panels").and_then(Value::as_object_mut) {
        for st in panels.values_mut() {
            if let Some(o) = st.as_object_mut() {
                o.remove("resources");
            }
        }
    }
    doc
}

fn hint_path(r: &Value, root: &Path) -> Option<PathBuf> {
    let h = r.get("hint").and_then(Value::as_str).filter(|h| !h.is_empty())?;
    let p = Path::new(h);
    Some(if p.is_absolute() { p.to_path_buf() } else { root.join(p) })
}

fn plain_options(options: Option<&Value>) -> Map<String, Value> {
    options.and_then(Value::as_object).map(|o| o.iter().filter(|(k, _)| !OPTION_ROLES.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
}

fn materialize(doc: &Value, paths: &BTreeMap<String, PathBuf>) -> Value {
    let mut out = doc.clone();
    for (role, p) in paths {
        let shown = json!(p.display().to_string());
        if role == STORE {
            out["store"] = shown;
        } else if OPTION_ROLES.contains(&role.as_str()) {
            if !out["options"].is_object() {
                out["options"] = json!({});
            }
            out["options"][role.as_str()] = shown;
        } else if let Some((id, key)) = role.strip_prefix("panels.").and_then(|r| r.rsplit_once('.')) {
            if !out["panels"].is_object() {
                out["panels"] = json!({});
            }
            if !out["panels"][id].is_object() {
                out["panels"][id] = json!({});
            }
            if !out["panels"][id]["resources"].is_object() {
                out["panels"][id]["resources"] = json!({});
            }
            out["panels"][id]["resources"][key] = shown;
        }
    }
    out
}

fn character_spec(c: &Value, store: &str, macro_file: Option<&str>) -> Result<Spec, ApiError> {
    if let Some(s) = c.get("spec").filter(|s| s.is_object()) {
        let spec: Spec = serde_json::from_value(s.clone()).map_err(|e| ApiError::invalid(format!("character spec: {e}")))?;
        return Ok(spec.canonical());
    }
    if let Some(d) = c.get("designer").filter(|d| d.is_object()) {
        let mut input = d.clone();
        input["store"] = json!(store);
        if let Some(obj) = input.as_object_mut() {
            obj.remove("eyes");
            obj.remove("uvs");
            obj.remove("fast");
        }
        if input.get("macro").is_none()
            && let Some(m) = macro_file
        {
            input["macro"] = json!(m);
        }
        return designer::spec_from_input(&input);
    }
    Err(ApiError::invalid("each character needs spec (a spec object) or designer (the designer's character input)"))
}

fn blob_refs(store: &Store, pack: &str, catalog: &str, game: &str) -> Result<BTreeSet<String>, ApiError> {
    let absent = |kind: &str, r: &str, e: String| {
        let altered = e.contains("corrupt");
        let what = if altered { "has been altered in this store" } else { "is not in this store" };
        ApiError::new(if altered { Code::Conflict } else { Code::NotFound }, format!("the project's base {kind} {r} {what} ({e})"))
            .hint("open the project with the store it was made with, or import that base into this store")
    };
    let pm: PackManifest = store.get_manifest("packs", pack).map_err(|e| absent("pack", pack, e))?;
    let cm: CatalogManifest = store.get_manifest("catalogs", catalog).map_err(|e| absent("catalog", catalog, e))?;
    let mut refs: BTreeSet<String> = pm.arrays.values().cloned().collect();
    refs.extend(cm.targets.values().map(|t| t.blob.clone()));
    let mut games = vec![game.to_string()];
    let mut seen = BTreeSet::new();
    while let Some(g) = games.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        let gm: GameManifest = store.get_manifest("game", &g).map_err(|e| absent("game base", &g, e))?;
        refs.extend(gm.arrays.values().cloned());
        games.extend(gm.lods.iter().map(|l| l.game.clone()));
    }
    Ok(refs)
}

pub fn base_state(store: &Store, base: &SpecBase, full: bool) -> Result<Value, ApiError> {
    let game = base.game_base.clone().ok_or_else(|| ApiError::invalid("the character names no game base"))?;
    let refs = blob_refs(store, &base.pack, &base.catalog, &game)?;
    let (mut lines, mut missing, mut corrupt, mut total) = (String::new(), Vec::new(), Vec::new(), 0u64);
    for r in &refs {
        let hex = hash::parse_reference(r).map_err(ApiError::invalid)?;
        let path = store.blob_path(&hex);
        match std::fs::metadata(&path) {
            Ok(m) => {
                total += m.len();
                lines.push_str(&format!("{hex} {}\n", m.len()));
                if full {
                    match std::fs::read(&path) {
                        Ok(bytes) if hash::sha256_hex(&bytes) == hex => {}
                        _ => corrupt.push(r.clone()),
                    }
                }
            }
            Err(_) => missing.push(r.clone()),
        }
    }
    if !missing.is_empty() {
        return Err(ApiError::not_found(format!("the store lacks {} of the {} blobs of the project's base (first: {})", missing.len(), refs.len(), missing[0]))
            .details(json!({"missing": missing})));
    }
    if !corrupt.is_empty() {
        return Err(ApiError::new(Code::Conflict, format!("{} blobs of the project's base no longer match their hashes (first: {})", corrupt.len(), corrupt[0]))
            .details(json!({"altered": corrupt})));
    }
    Ok(json!({"pack": base.pack, "catalog": base.catalog, "game_base": game, "blobs": refs.len(), "bytes": total,
              "fingerprint": hash::reference(&hash::sha256_hex(lines.as_bytes())), "verified": if full { "every blob hashed" } else { "manifests hashed, blob sizes fingerprinted" }}))
}

fn residual_refs(characters: &[Value]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in characters {
        let specs = std::iter::once(c.get("spec")).chain(c.get("history").and_then(Value::as_array).into_iter().flatten().map(|h| h.get("spec")));
        for s in specs.flatten() {
            if let Some(b) = s.pointer("/residual/blob").and_then(Value::as_str) {
                out.insert(b.to_string());
            }
        }
    }
    out
}

fn dense_sha(store: &Store, spec: &Spec, source: Option<&ch::MacroSource>, zipped: &BTreeMap<String, PathBuf>) -> Result<String, ApiError> {
    if spec.macros.is_some() && source.is_none() {
        return Err(ApiError::invalid(format!("{} has macro sliders: pass macro (the macro definition file) to evaluate it", spec.name)));
    }
    let local = spec.residual.as_ref().and_then(|r| hash::parse_reference(&r.blob).ok()).and_then(|h| zipped.get(&h));
    let dense = match (local, &spec.residual) {
        (Some(file), Some(r)) => {
            let mut bare = spec.clone();
            bare.residual = None;
            let (problems, resolved) = ch::problems(store, &bare, source);
            let resolved = match resolved {
                Some(x) if problems.is_empty() => x,
                _ => return Err(ApiError::invalid(api_text(&format!("the spec does not validate: {}", problems.join("; "))))),
            };
            let (mut dense, _) = ch::parametric(store, &resolved, &bare, source).map_err(|e| ApiError::invalid(api_text(&e)))?;
            let bytes = std::fs::read(file).map_err(io_err(file))?;
            if bytes.len() != r.bytes || r.vertices != resolved.pack.vertices {
                return Err(ApiError::invalid(format!("residual blob {} is {} bytes for {} vertices, the spec says {} bytes for {}", r.blob, bytes.len(), resolved.pack.vertices, r.bytes, r.vertices)));
            }
            let (values, _) = gf_core::residual::decode(&bytes).map_err(ApiError::invalid)?;
            if values.len() != dense.len() {
                return Err(ApiError::invalid(format!("residual has {} vertices, the pack {}", values.len(), dense.len())));
            }
            for (p, d) in dense.iter_mut().zip(&values) {
                for (a, b) in p.iter_mut().zip(d) {
                    *a += b;
                }
            }
            dense
        }
        _ => ch::evaluate(store, spec, source).map_err(|e| ApiError::invalid(api_text(&e)))?.0,
    };
    let bytes: Vec<u8> = dense.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
    Ok(hash::reference(&hash::sha256_hex(&bytes)))
}

fn flag(input: &Value, key: &str) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn given(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn base_checker<'a>(base: &'a SpecBase, saved: Option<&'a Value>, full: bool, states: &'a mut Vec<(String, Value)>) -> impl FnMut(&Path) -> Result<String, ApiError> + 'a {
    move |p: &Path| {
        let key = res::slashed(p);
        if let Some((_, st)) = states.iter().find(|(k, _)| *k == key) {
            return Ok(st["fingerprint"].as_str().unwrap_or("").to_string());
        }
        let st = base_state(&open_store(p)?, base, full)?;
        if let Some(saved) = saved
            && saved.get("fingerprint").is_some()
            && st["fingerprint"] != saved["fingerprint"]
        {
            return Err(ApiError::new(
                Code::Conflict,
                format!("{}: the project's base data changed since it was saved: {} blobs / {} bytes now, {} / {} when saved (fingerprint {} vs {})",
                        p.display(), st["blobs"], st["bytes"], saved["blobs"], saved["bytes"], st["fingerprint"], saved["fingerprint"]),
            ));
        }
        let fp = st["fingerprint"].as_str().unwrap_or("").to_string();
        states.push((key, st));
        Ok(fp)
    }
}

fn build(ctx: &Ctx, input: &Value, fresh: bool) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let name = text(input, "name");
    let recovery = flag(input, "recovery");
    check_name(name, recovery)?;
    let path = project_path(&root, name);
    let existing = if !fresh && path.is_file() { Some(read_doc(&path)?) } else { None };
    let old = existing.clone().unwrap_or(Value::Null);
    let old_refs = refs_of(&old);
    let pick = |key: &str, default: Value| input.get(key).cloned().or_else(|| old.get(key).cloned()).unwrap_or(default);
    let store_path = match given(input.get("store")) {
        Some(s) => PathBuf::from(s),
        None => old_refs.get(STORE).and_then(|r| hint_path(r, &root)).unwrap_or_default(),
    };
    let store = open_store(&store_path)?;
    let store_text = store_path.to_string_lossy().to_string();
    if let Some(o) = input.get("options")
        && !o.is_object()
    {
        return Err(ApiError::invalid("options must be an object"));
    }
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut fresh_roles: BTreeSet<String> = BTreeSet::new();
    let mut refs = Map::new();
    let options = match input.get("options") {
        Some(o) => {
            for k in OPTION_ROLES {
                if let Some(p) = given(o.get(k)) {
                    paths.insert(k.into(), PathBuf::from(p));
                    fresh_roles.insert(k.into());
                }
            }
            plain_options(Some(o))
        }
        None => {
            for k in OPTION_ROLES {
                if let Some(r) = old_refs.get(k) {
                    refs.insert(k.into(), r.clone());
                    if let Some(p) = hint_path(r, &root) {
                        paths.insert(k.into(), p);
                    }
                }
            }
            plain_options(old.get("options"))
        }
    };
    if let Some(m) = given(input.get("macro")) {
        paths.insert("macro".into(), PathBuf::from(m));
        fresh_roles.insert("macro".into());
        refs.remove("macro");
    }
    let panels_in = pick("panels", json!({}));
    let Some(panel_map) = panels_in.as_object() else { return Err(ApiError::invalid("panels must be an object")) };
    let mut panels = Map::new();
    for (id, st) in panel_map {
        let mut st = st.clone();
        if let Some(rs) = st.as_object_mut().and_then(|o| o.remove("resources")) {
            for (k, v) in rs.as_object().into_iter().flatten() {
                if k.is_empty() || k.contains('.') || id.is_empty() {
                    return Err(ApiError::invalid(format!("panels.{id}.resources.{k}: a panel id and a resource name must be non-empty and the name must hold no '.'")));
                }
                if let Some(p) = given(Some(v)) {
                    let role = format!("panels.{id}.{k}");
                    paths.insert(role.clone(), PathBuf::from(p));
                    fresh_roles.insert(role);
                }
            }
        }
        panels.insert(id.clone(), st);
    }
    if input.get("panels").is_none() {
        for (role, r) in old_refs.iter().filter(|(k, _)| k.starts_with("panels.")) {
            refs.insert(role.clone(), r.clone());
            if let Some(p) = hint_path(r, &root) {
                paths.insert(role.clone(), p);
            }
        }
    }
    let macro_file = paths.get("macro").cloned();
    let source = macro_source(macro_file.as_deref())?;
    let macro_text = macro_file.as_ref().map(|p| p.to_string_lossy().to_string());
    let given_chars = pick("characters", json!([]));
    let given_chars = given_chars.as_array().filter(|a| !a.is_empty()).ok_or_else(|| ApiError::invalid("characters: give at least one character"))?;
    let old_chars: Vec<Value> = old.get("characters").and_then(Value::as_array).cloned().unwrap_or_default();
    let now = now_utc();
    let mut characters = Vec::new();
    let mut base: Option<SpecBase> = None;
    let mut ids = BTreeSet::new();
    for (k, c) in given_chars.iter().enumerate() {
        let spec = character_spec(c, &store_text, macro_text.as_deref()).map_err(|e| ApiError { message: format!("character {k}: {}", e.message), ..e })?;
        let (problems, _) = ch::problems(&store, &spec, source.as_ref());
        if !problems.is_empty() {
            let problems: Vec<String> = problems.iter().map(|p| api_text(p)).collect();
            return Err(ApiError::invalid(format!("character {k} ({}) does not validate: {}", spec.name, problems.join("; "))).details(json!({"problems": problems})));
        }
        let mut key = spec.base.clone();
        key.placement = String::new();
        match &base {
            None => base = Some(key),
            Some(b) if *b != key => return Err(ApiError::invalid(format!("character {k} ({}) uses another pack, catalog or game base; one project holds one base", spec.name))),
            _ => {}
        }
        let id = c.get("id").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| old_chars.get(k).and_then(|o| o.get("id")).and_then(Value::as_str).map(str::to_string).unwrap_or(format!("c{}", k + 1)));
        if !ids.insert(id.clone()) {
            return Err(ApiError::invalid(format!("character id {id:?} is used twice")));
        }
        let mut history: Vec<Value> = old_chars
            .iter()
            .find(|o| o.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .and_then(|o| o.get("history"))
            .or_else(|| c.get("history"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let revision = spec.revision();
        let spec_value = serde_json::to_value(&spec).map_err(|e| ApiError::internal(e.to_string()))?;
        if !recovery && history.last().and_then(|h| h.get("revision")).and_then(Value::as_str) != Some(revision.as_str()) {
            history.push(json!({"revision": revision, "at": now, "spec": spec_value}));
            let excess = history.len().saturating_sub(HISTORY_LIMIT);
            history.drain(..excess);
        }
        characters.push(json!({"id": id, "name": spec.name, "revision": revision, "spec": spec_value, "history": history}));
    }
    let first: Spec = serde_json::from_value(characters[0]["spec"].clone()).map_err(|e| ApiError::internal(e.to_string()))?;
    let base = base_state(&store, &first.base, false)?;
    refs.insert(STORE.into(), json!({"kind": "store", "identity": base["fingerprint"], "hint": res::hint(&store_path, &root)}));
    paths.insert(STORE.into(), store_path.clone());
    let mut unresolved = Vec::new();
    {
        let mut states = vec![(res::slashed(&store_path), base.clone())];
        let mut check = base_checker(&first.base, None, false, &mut states);
        for role in &fresh_roles {
            let p = &paths[role];
            if recovery && !p.exists() {
                let why = format!("{} does not exist", p.display());
                refs.insert(role.clone(), res::unresolved_reference(role, p, &root, &why));
                unresolved.push(json!({"role": role, "path": p.display().to_string(), "reason": why}));
                continue;
            }
            refs.insert(role.clone(), res::reference(role, p, &root, &mut check)?);
        }
    }
    let mut report = Vec::new();
    for c in &characters {
        let mut r = json!({"id": c["id"], "name": c["name"], "revision": c["revision"], "history": c["history"].as_array().map_or(0, Vec::len)});
        if flag(input, "evaluate") {
            let spec: Spec = serde_json::from_value(c["spec"].clone()).map_err(|e| ApiError::internal(e.to_string()))?;
            r["dense_sha256"] = json!(dense_sha(&store, &spec, source.as_ref(), &BTreeMap::new())?);
        }
        report.push(r);
    }
    let active = pick("active", json!(0)).as_u64().unwrap_or(0).min(characters.len() as u64 - 1);
    let mut doc = json!({
        "format": FORMAT, "name": name, "created": old.get("created").cloned().unwrap_or(json!(now)), "modified": now,
        "resources": refs, "base": base, "options": options, "characters": characters, "active": active,
        "panels": panels, "viewer": pick("viewer", json!({})), "notes": pick("notes", json!("")),
    });
    if !doc["viewer"].is_object() {
        return Err(ApiError::invalid("viewer must be an object"));
    }
    if !doc["notes"].is_string() {
        return Err(ApiError::invalid("notes must be a string"));
    }
    let target = if recovery {
        doc["recovery"] = json!({"of": name, "saved_modified": old.get("modified").cloned().unwrap_or(Value::Null)});
        recovery_path(&root, name)
    } else {
        path.clone()
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| ApiError::internal(e.to_string()))?;
    write_atomic(&target, &bytes)?;
    if !recovery {
        let slot = recovery_path(&root, name);
        if slot.is_file() {
            std::fs::remove_file(&slot).map_err(io_err(&slot))?;
        }
    }
    Ok(json!({"path": target.display().to_string(), "root": root.display().to_string(), "name": name, "recovery": recovery,
              "modified": doc["modified"], "bytes": bytes.len(), "characters": report, "unresolved": unresolved, "project": materialize(&doc, &paths)}))
}

pub fn save(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    build(ctx, input, false)
}

pub fn new(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let name = text(input, "name");
    check_name(name, false)?;
    let path = project_path(&root, name);
    if path.exists() && !flag(input, "overwrite") {
        return Err(ApiError::new(Code::Conflict, format!("a project named {name:?} already exists at {}", path.display())).hint("pass overwrite: true, or another name"));
    }
    let store = text(input, "store");
    let spec = match (input.get("character").filter(|c| c.is_object()), text(input, "spec")) {
        (Some(c), _) => serde_json::from_value::<Spec>(c.clone()).map_err(|e| ApiError::invalid(format!("character: {e}")))?,
        (None, p) if !p.is_empty() => ch::read_spec(Path::new(p)).map_err(ApiError::invalid)?,
        _ => {
            let mut probe = json!({"store": store});
            if let Some(m) = input.get("macro") {
                probe["macro"] = m.clone();
            }
            let s = designer::sliders(ctx, &probe)?;
            Spec {
                format: SPEC_FORMAT.into(),
                name: name.to_string(),
                base: SpecBase {
                    pack: text(&s, "pack").to_string(),
                    catalog: text(&s, "catalog").to_string(),
                    game_base: Some(text(&s, "game").to_string()),
                    placement: s.pointer("/start/placement").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| gf_core::placement::DEFAULT.to_string()),
                },
                weights: BTreeMap::new(),
                macros: None,
                residual: None,
            }
        }
    };
    let mut inner = json!({"name": name, "store": store, "characters": [{"spec": spec}], "root": root.display().to_string()});
    for key in ["options", "notes", "evaluate", "macro", "panels"] {
        if let Some(v) = input.get(key) {
            inner[key] = v.clone();
        }
    }
    build(ctx, &inner, true)
}

type Checked = (Store, Value, Option<ch::MacroSource>, res::Resolved);

fn check_doc(doc: &Value, input: &Value, root: &Path, from_zip: bool, extra_blobs: &BTreeSet<String>) -> Result<Checked, ApiError> {
    let characters = doc.get("characters").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| ApiError::invalid("the project has no characters"))?;
    let saved = doc.get("base").cloned().unwrap_or(Value::Null);
    let mut specs = Vec::new();
    for (k, c) in characters.iter().enumerate() {
        let spec: Spec = serde_json::from_value(c.get("spec").cloned().unwrap_or(Value::Null)).map_err(|e| ApiError::invalid(format!("character {k}: {e}")))?;
        let problems = spec.problems();
        if !problems.is_empty() {
            return Err(ApiError::invalid(format!("character {k} ({}) does not validate: {}", spec.name, problems.join("; "))));
        }
        let stored = c.get("revision").and_then(Value::as_str).unwrap_or("");
        if stored != spec.revision() {
            return Err(ApiError::new(Code::Conflict, format!("character {k} ({}) was edited outside GazeForge: its spec hashes to {}, the project records {stored}", spec.name, spec.revision())));
        }
        let game = spec.base.game_base.clone().unwrap_or_default();
        for (key, have) in [("pack", &spec.base.pack), ("catalog", &spec.base.catalog), ("game_base", &game)] {
            if saved.get(key).and_then(Value::as_str) != Some(have.as_str()) {
                return Err(ApiError::new(Code::Conflict, format!("character {k} ({}) names {key} {have}, the project's base is {}", spec.name, saved.get(key).unwrap_or(&Value::Null))));
            }
        }
        specs.push(spec);
    }
    let mut refs = refs_of(doc);
    if !refs.contains_key(STORE) {
        refs.insert(STORE.into(), json!({"kind": "store", "identity": saved.get("fingerprint"), "file": ""}));
    }
    let full = input.get("verify").and_then(Value::as_str) == Some("full");
    let mut states = Vec::new();
    let resolved = {
        let mut check = base_checker(&specs[0].base, Some(&saved), full, &mut states);
        res::resolve_all(&refs, input, root, from_zip, &mut check)?
    };
    let store_path = resolved.paths.get(STORE).cloned().ok_or_else(|| ApiError::invalid("the store cannot be skipped"))?;
    let store = open_store(&store_path)?;
    let key = res::slashed(&store_path);
    let now = states.iter().find(|(k, _)| *k == key).map(|(_, s)| s.clone()).unwrap_or(Value::Null);
    let mut residuals = Vec::new();
    for r in residual_refs(characters) {
        let hex = hash::parse_reference(&r).map_err(ApiError::invalid)?;
        if extra_blobs.contains(&hex) {
            residuals.push(json!({"blob": r, "from": "portable file"}));
            continue;
        }
        store.get_ref(&r).map_err(|e| ApiError::not_found(format!("a character blob is missing or altered: {e}")).hint("load the project's portable zip (project.export) to bring its character blobs"))?;
        residuals.push(json!({"blob": r, "from": "store"}));
    }
    let wants_macros = specs.iter().any(|s| s.macros.is_some());
    let file = resolved.paths.get("macro").cloned().or_else(|| given(input.get("macro")).filter(|_| !refs.contains_key("macro")).map(PathBuf::from));
    let source = macro_source(file.as_deref())?;
    let shown = file.as_ref().map(|p| p.display().to_string());
    let macro_check = match (&source, wants_macros) {
        (_, false) => json!("no character uses macro sliders"),
        (None, true) => json!(format!("not checked: the macro definition file {} is not available", shown.as_deref().unwrap_or("(none given)"))),
        (Some(s), true) => {
            for spec in &specs {
                if let Some(m) = &spec.macros
                    && m.definition != s.reference
                {
                    return Err(ApiError::new(Code::Conflict, format!("{} expects macro definition {}, the file {} is {}", spec.name, m.definition, shown.as_deref().unwrap_or(""), s.reference)));
                }
            }
            json!(format!("{} matches", s.reference))
        }
    };
    let checked = json!({"base": now, "residual_blobs": residuals, "macro_definition": macro_check, "characters": specs.len(), "resources": resolved.report});
    Ok((store, checked, source, resolved))
}

fn blob_entry(name: &str) -> Option<&str> {
    name.strip_prefix("blobs/").filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
}

fn cap(input: &Value, key: &str, default: u64) -> u64 {
    input.get(key).and_then(Value::as_u64).filter(|v| *v > 0).unwrap_or(default)
}

pub fn load(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let zip_path = text(input, "zip");
    let recovery = flag(input, "recovery");
    let from_zip = !zip_path.is_empty();
    let mut extracted = None;
    let mut blobs: BTreeMap<String, PathBuf> = BTreeMap::new();
    let (doc, path) = if from_zip {
        let caps = zipsafe::Caps {
            entry: cap(input, "max_entry_bytes", zipsafe::DEFAULT_ENTRY_BYTES),
            total: cap(input, "max_total_bytes", zipsafe::DEFAULT_TOTAL_BYTES),
            json: cap(input, "max_project_bytes", zipsafe::DEFAULT_JSON_BYTES),
        };
        let mut parsed = None;
        let ex = {
            let mut plan = |bytes: &[u8]| -> Result<BTreeSet<String>, ApiError> {
                let doc = from_portable(parse_doc(bytes, zip_path)?);
                res::check_portable_refs(&refs_of(&doc)).map_err(|e| ApiError { message: format!("{zip_path}: {}", e.message), ..e })?;
                let chars = doc.get("characters").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut allowed: BTreeSet<String> =
                    residual_refs(&chars).iter().filter_map(|r| hash::parse_reference(r).ok()).map(|h| format!("blobs/{h}")).filter(|n| blob_entry(n).is_some()).collect();
                allowed.insert(ZIP_PROJECT.to_string());
                allowed.insert(ZIP_INDEX.to_string());
                parsed = Some(doc);
                Ok(allowed)
            };
            zipsafe::extract(Path::new(zip_path), &caps, ZIP_PROJECT, &mut plan)?.1
        };
        let doc = parsed.ok_or_else(|| ApiError::internal("the project entry was not parsed"))?;
        for (name, file, _) in &ex.entries {
            let Some(hex) = blob_entry(name) else { continue };
            if res::file_identity(file).map_err(ApiError::internal)? != hash::reference(hex) {
                return Err(ApiError::new(Code::Conflict, format!("{zip_path}: blob {hex} is corrupt; the file was refused and nothing was imported")));
            }
            blobs.insert(hex.to_string(), file.clone());
        }
        extracted = Some(ex);
        let name = match text(input, "name") {
            "" => doc.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            n => n.to_string(),
        };
        check_name(&name, false)?;
        (doc, project_path(&root, &name))
    } else {
        let name = text(input, "name");
        check_name(name, recovery)?;
        let path = if recovery { recovery_path(&root, name) } else { project_path(&root, name) };
        (read_doc(&path)?, path)
    };
    let extra: BTreeSet<String> = blobs.keys().cloned().collect();
    let (store, checked, source, resolved) = check_doc(&doc, input, &root, from_zip, &extra)?;
    if flag(input, "evaluate")
        && source.is_none()
        && let Some(c) = doc["characters"].as_array().into_iter().flatten().find(|c| c.pointer("/spec/macros").is_some_and(|m| !m.is_null()))
    {
        return Err(ApiError::invalid(format!("{} has macro sliders: pass macro (the macro definition file) to evaluate it", c["name"].as_str().unwrap_or("a character"))));
    }
    let mut characters = Vec::new();
    for c in doc["characters"].as_array().into_iter().flatten() {
        let mut r = json!({"id": c["id"], "name": c["name"], "revision": c["revision"]});
        if flag(input, "evaluate") {
            let spec: Spec = serde_json::from_value(c["spec"].clone()).map_err(|e| ApiError::internal(e.to_string()))?;
            r["dense_sha256"] = json!(dense_sha(&store, &spec, source.as_ref(), &blobs)?);
        }
        characters.push(r);
    }
    let mut imported = Vec::new();
    let mut doc = doc;
    let mut written = false;
    if from_zip {
        let name = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(EXT)).unwrap_or("").to_string();
        doc["name"] = json!(name);
        let mut local = Map::new();
        for (role, r) in refs_of(&doc) {
            let mut l = json!({"kind": r["kind"], "identity": r["identity"]});
            if let Some(p) = resolved.paths.get(&role) {
                l["hint"] = json!(res::hint(p, &root));
            } else {
                l["file"] = json!(res::file_name(&r));
            }
            local.insert(role, l);
        }
        doc["resources"] = Value::Object(local);
        let existing = if path.exists() { Some(read_doc(&path)) } else { None };
        let same = matches!(&existing, Some(Ok(e)) if *e == doc);
        if existing.is_some() && !same && !flag(input, "overwrite") {
            return Err(ApiError::new(Code::Conflict, format!("a different project named {name:?} already exists at {} (the whole project is compared, viewer, options and panels included)", path.display()))
                .hint("pass overwrite: true to replace it, or name to import under another name"));
        }
        let out = serde_json::to_vec_pretty(&doc).map_err(|e| ApiError::internal(e.to_string()))?;
        let mut added = Vec::new();
        let undo = |added: &[PathBuf]| {
            for a in added {
                let _ = std::fs::remove_file(a);
            }
        };
        for (hex, file) in &blobs {
            let dest = store.blob_path(hex);
            let fresh = !dest.exists();
            let put = std::fs::read(file).map_err(io_err(file)).and_then(|b| gf_core::catalog::BlobStore::put(&store, &b).map_err(ApiError::internal));
            match put {
                Ok(p) if &p == hex => {}
                other => {
                    undo(&added);
                    return Err(other.err().unwrap_or_else(|| ApiError::internal(format!("blob {hex} was stored under another hash"))));
                }
            }
            if fresh {
                added.push(dest);
            }
            imported.push(hash::reference(hex));
        }
        if !same {
            if let Err(e) = write_atomic(&path, &out) {
                undo(&added);
                return Err(e);
            }
            written = true;
        }
    }
    drop(extracted);
    let recovery_newer = !recovery && !from_zip && {
        let slot = recovery_path(&root, doc["name"].as_str().unwrap_or(""));
        slot.is_file() && read_doc(&slot).is_ok_and(|r| r["modified"].as_str() > doc["modified"].as_str())
    };
    Ok(json!({"path": path.display().to_string(), "root": root.display().to_string(), "recovered": recovery, "imported_blobs": imported, "written": written,
              "recovery_available": recovery_newer, "checked": checked, "characters": characters, "project": materialize(&doc, &resolved.paths)}))
}

pub fn list(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let scan = |dir: &Path| -> Vec<(String, Result<Value, ApiError>)> {
        let mut out: Vec<(String, Result<Value, ApiError>)> = std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter_map(|n| n.strip_suffix(EXT).map(str::to_string))
                    .filter(|n| !n.starts_with('.'))
                    .map(|n| {
                        let doc = read_doc(&dir.join(format!("{n}{EXT}")));
                        (n, doc)
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    };
    let mut projects = Vec::new();
    let mut saved_modified = BTreeMap::new();
    for (name, doc) in scan(&root) {
        match doc {
            Ok(d) => {
                saved_modified.insert(name.clone(), d["modified"].clone());
                let store = refs_of(&d).get(STORE).and_then(|r| r.get("hint")).cloned().unwrap_or(Value::Null);
                projects.push(json!({"name": name, "path": project_path(&root, &name).display().to_string(), "created": d["created"], "modified": d["modified"],
                                     "store": store, "characters": d["characters"].as_array().map_or(0, Vec::len), "notes": d["notes"]}));
            }
            Err(e) => projects.push(json!({"name": name, "error": e.message})),
        }
    }
    projects.sort_by(|a, b| b["modified"].as_str().cmp(&a["modified"].as_str()));
    let mut recoveries = Vec::new();
    for (name, doc) in scan(&root.join(".recovery")) {
        if let Ok(d) = doc {
            let saved = saved_modified.get(&name).cloned().unwrap_or(Value::Null);
            let newer = saved.as_str().is_none_or(|s| d["modified"].as_str().is_some_and(|m| m > s));
            recoveries.push(json!({"name": name, "modified": d["modified"], "saved_modified": saved, "newer_than_saved": newer,
                                   "path": recovery_path(&root, &name).display().to_string()}));
        }
    }
    Ok(json!({"root": root.display().to_string(), "projects": projects, "recoveries": recoveries}))
}

pub fn discard(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let name = text(input, "name");
    check_name(name, true)?;
    let slot = recovery_path(&root, name);
    let removed = slot.is_file();
    if removed {
        std::fs::remove_file(&slot).map_err(io_err(&slot))?;
    }
    Ok(json!({"name": name, "removed": removed, "path": slot.display().to_string()}))
}

pub fn portable_doc(doc: &Value) -> Value {
    let mut out = doc.clone();
    let refs: Map<String, Value> = refs_of(doc).iter().map(|(k, r)| (k.clone(), res::portable(r))).collect();
    out["resources"] = Value::Object(refs);
    if let Some(o) = out.as_object_mut() {
        o.remove("store");
        o.remove("recovery");
    }
    out["options"] = Value::Object(plain_options(doc.get("options")));
    out
}

pub fn export(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = root(ctx, input);
    let name = text(input, "name");
    check_name(name, false)?;
    let path = project_path(&root, name);
    let doc = read_doc(&path)?;
    let mut refs = refs_of(&doc);
    refs.retain(|k, _| k == STORE);
    let spec: Spec = serde_json::from_value(doc.pointer("/characters/0/spec").cloned().unwrap_or(Value::Null)).map_err(|e| ApiError::invalid(format!("character 0: {e}")))?;
    let mut states = Vec::new();
    let resolved = {
        let mut check = base_checker(&spec.base, doc.get("base"), false, &mut states);
        res::resolve_all(&refs, input, &root, false, &mut check)?
    };
    let store = open_store(&resolved.paths[STORE])?;
    let characters = doc["characters"].as_array().cloned().unwrap_or_default();
    let mut blobs = BTreeMap::new();
    for r in residual_refs(&characters) {
        let hex = hash::parse_reference(&r).map_err(ApiError::invalid)?;
        let b = store.get_ref(&r).map_err(|e| ApiError::not_found(format!("a character blob is missing or altered: {e}")))?;
        blobs.insert(hex, b);
    }
    let portable = portable_doc(&doc);
    let mut unnamed = Vec::new();
    for (role, r) in portable["resources"].as_object().into_iter().flatten() {
        let f = r["file"].as_str().unwrap_or("");
        if !f.is_empty()
            && let Some(p) = res::bare_name_problem(f)
        {
            unnamed.push(format!("{role}: {f:?} {p}"));
        }
    }
    if !unnamed.is_empty() {
        return Err(ApiError::invalid(format!("resources are exported by file name and these names cannot be: {}", unnamed.join("; "))).hint("rename or move the file or folder, save, then export"));
    }
    let index = json!({"format": PORTABLE_FORMAT, "project": ZIP_PROJECT, "base": doc["base"], "resources": portable["resources"],
                       "blobs": blobs.iter().map(|(h, b)| (h.clone(), json!(b.len()))).collect::<serde_json::Map<String, Value>>(),
                       "note": "no local paths: every outside resource is named by role, identity and file name, and project.load finds it from its options; the base (pack, catalog, game base) is not bundled and must be in the store the project is loaded with"});
    let scan = res::scanner(&root);
    let mut hits = res::hits_with(&portable, &scan);
    hits.extend(res::hits_with(&index, &scan).into_iter().map(|h| res::Hit { pointer: format!("{ZIP_INDEX}#{}", h.pointer), ..h }));
    let notes_kept = flag(input, "allow_paths_in_notes") && hits.iter().any(|h| h.pointer == "/notes");
    if flag(input, "allow_paths_in_notes") {
        hits.retain(|h| h.pointer != "/notes");
    }
    if !hits.is_empty() {
        let list: Vec<String> = hits.iter().map(|h| format!("{} ({}): \"{}\"", h.pointer, h.why, h.fragment)).collect();
        let only_notes = hits.iter().all(|h| h.pointer == "/notes");
        return Err(ApiError::invalid(format!("the portable file would carry local paths or user details at {}: {}", hits.len(), list.join("; ")))
            .hint(if only_notes {
                "edit the notes, or pass allow_paths_in_notes: true to export them as they are"
            } else {
                "change the fields named (only the notes can be kept, with allow_paths_in_notes); resources are exported by identity and file name only"
            })
            .details(json!({"found": hits.iter().map(|h| json!({"pointer": h.pointer, "reason": h.why, "fragment": h.fragment})).collect::<Vec<_>>()})));
    }
    let project_bytes = serde_json::to_vec_pretty(&portable).map_err(|e| ApiError::internal(e.to_string()))?;
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buffer);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let zerr = |e: zip::result::ZipError| ApiError::internal(format!("zip: {e}"));
        let ierr = |e: std::io::Error| ApiError::internal(format!("zip: {e}"));
        z.start_file(ZIP_PROJECT, opts).map_err(zerr)?;
        z.write_all(&project_bytes).map_err(ierr)?;
        z.start_file(ZIP_INDEX, opts).map_err(zerr)?;
        z.write_all(&serde_json::to_vec_pretty(&index).map_err(|e| ApiError::internal(e.to_string()))?).map_err(ierr)?;
        for (hex, b) in &blobs {
            z.start_file(format!("blobs/{hex}"), opts).map_err(zerr)?;
            z.write_all(b).map_err(ierr)?;
        }
        z.finish().map_err(zerr)?;
    }
    let out = match text(input, "out") {
        "" => root.join(format!("{name}.gfproj.zip")),
        o => PathBuf::from(o),
    };
    let zipped = buffer.into_inner();
    write_atomic(&out, &zipped)?;
    Ok(json!({"out": out.display().to_string(), "bytes": zipped.len(), "project_bytes": project_bytes.len(), "character_blobs": blobs.len(), "notes_kept_with_paths": notes_kept,
              "base": doc["base"], "resources": portable["resources"]}))
}

fn root_prop() -> Value {
    json!({"type": "string", "minLength": 1, "description": "the projects folder (default: GAZEFORGE_PROJECTS, or serve --projects, or GazeForge/projects under the user's data folder)"})
}

fn name_prop() -> Value {
    json!({"type": "string", "minLength": 1, "description": "project name: 1 to 64 letters, digits, spaces, '_', '-', '.'; the file is <root>/<name>.gfproj.json"})
}

fn path_prop(what: &str) -> Value {
    json!({"type": "string", "minLength": 1, "description": what})
}

pub fn new_input() -> Value {
    obj(
        json!({"root": root_prop(), "name": name_prop(), "store": {"type": "string", "minLength": 1, "description": "path of the store directory"},
               "spec": {"type": "string", "minLength": 1, "description": "a spec file to start from"}, "character": {"type": "object", "description": "a spec object to start from"},
               "macro": {"type": "string", "minLength": 1, "description": "the macro definition file (kept as the project's macro resource)"},
               "options": {"type": "object", "description": "designer options kept with the project: macro, eyes, face_morphs (paths, kept as resources by identity) and plain values such as face_prefix"},
               "panels": {"type": "object", "description": "panel id -> the panel's getState(); a state's resources object {name: path} is kept as resources by identity"},
               "notes": {"type": "string"}, "overwrite": {"type": "boolean"}, "evaluate": {"type": "boolean", "description": "report each character's evaluated dense sha256"}}),
        &["name", "store"],
    )
}

pub fn save_input() -> Value {
    obj(
        json!({"root": root_prop(), "name": name_prop(),
               "store": {"type": "string", "minLength": 1, "description": "path of the store directory (default: the saved project's)"},
               "characters": {"type": "array", "items": {"type": "object"}, "description": "[{id?, spec: spec object} or {id?, designer: the designer's character input}]; default: the saved project's"},
               "active": {"type": "integer", "minimum": 0}, "panels": {"type": "object", "description": "panel id -> the panel's getState(); a state's resources object {name: path} is kept as resources by identity"},
               "viewer": {"type": "object", "description": "the designer's viewer state"}, "notes": {"type": "string"},
               "options": {"type": "object", "description": "designer options: macro, eyes, face_morphs (paths, kept as resources by identity) and plain values such as face_prefix"},
               "macro": {"type": "string", "minLength": 1, "description": "the macro definition file, for characters with macro sliders"},
               "recovery": {"type": "boolean", "description": "write the recovery slot (autosave) instead of the project file"},
               "evaluate": {"type": "boolean", "description": "report each character's evaluated dense sha256"}}),
        &["name"],
    )
}

pub fn load_input() -> Value {
    obj(
        json!({"root": root_prop(), "name": name_prop(), "zip": {"type": "string", "minLength": 1, "description": "a portable .gfproj.zip to import into root (name renames it)"},
               "store": path_prop("the store for the store role (and any panel store role)"),
               "macro": path_prop("the macro definition file for the macro role"),
               "eyes": path_prop("the eyes file for the eyes role"),
               "face_morphs": path_prop("the face morphs folder for the face_morphs role"),
               "body": path_prop("the body pack folder for a panel body role"),
               "resources": {"type": "object", "additionalProperties": {"type": "string"}, "description": "role -> path for any role, e.g. {\"panels.stitch.body\": \"...\"}"},
               "search": {"type": "array", "items": {"type": "string"}, "description": "folders where a resource is looked up by its file name (before GAZEFORGE_RESOURCES, the projects folder and the saved hint)"},
               "skip": {"type": "array", "items": {"type": "string"}, "description": "roles that may stay unresolved (the store cannot)"},
               "recovery": {"type": "boolean", "description": "load the recovery slot (autosave) instead of the project file"},
               "verify": {"enum": ["quick", "full"], "description": "quick (default): manifests hashed and blob sizes fingerprinted; full: every base blob hashed"},
               "overwrite": {"type": "boolean", "description": "with zip: replace a different project of the same name"},
               "max_entry_bytes": {"type": "integer", "minimum": 1, "description": "with zip: the cap on one entry's decompressed bytes, counted while reading (default 268435456)"},
               "max_total_bytes": {"type": "integer", "minimum": 1, "description": "with zip: the cap on all entries' decompressed bytes (default 2147483648)"},
               "max_project_bytes": {"type": "integer", "minimum": 1, "description": "with zip: the cap on the project entry, counted while reading and before parsing (default 16777216)"},
               "evaluate": {"type": "boolean", "description": "report each character's evaluated dense sha256"}}),
        &[],
    )
}

pub fn list_input() -> Value {
    obj(json!({"root": root_prop()}), &[])
}

pub fn discard_input() -> Value {
    obj(json!({"root": root_prop(), "name": name_prop()}), &["name"])
}

pub fn export_input() -> Value {
    obj(
        json!({"root": root_prop(), "name": name_prop(), "out": {"type": "string", "minLength": 1, "description": "the .gfproj.zip to write (default <root>/<name>.gfproj.zip)"},
               "store": path_prop("the store holding the character blobs (default: the project's)"),
               "allow_paths_in_notes": {"type": "boolean", "description": "export the notes even when they hold local paths or user details (every other field is always refused)"}}),
        &["name"],
    )
}

pub fn save_output() -> Value {
    shape(json!({"path": {"type": "string"}, "name": {"type": "string"}, "modified": {"type": "string"}, "characters": {"type": "array"}, "unresolved": {"type": "array"}}))
}

pub fn load_output() -> Value {
    shape(json!({"path": {"type": "string"}, "project": {"type": "object"}, "checked": {"type": "object"}, "characters": {"type": "array"}}))
}

pub fn list_output() -> Value {
    shape(json!({"root": {"type": "string"}, "projects": {"type": "array"}, "recoveries": {"type": "array"}}))
}

pub fn discard_output() -> Value {
    shape(json!({"name": {"type": "string"}, "removed": {"type": "boolean"}}))
}

pub fn export_output() -> Value {
    shape(json!({"out": {"type": "string"}, "bytes": {"type": "integer"}, "character_blobs": {"type": "integer"}, "resources": {"type": "object"}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_confined_to_the_root() {
        for bad in ["", "../x", "a/b", "a\\b", ".hidden", "CON", "com1.x", "x.", "untitled"] {
            assert!(check_name(bad, false).is_err(), "{bad}");
        }
        for good in ["face 01", "my_project-2.v3"] {
            assert!(check_name(good, false).is_ok(), "{good}");
        }
        assert!(check_name(UNTITLED, true).is_ok());
    }

    #[test]
    fn timestamps_are_utc_iso() {
        let t = now_utc();
        assert_eq!(t.len(), 24, "{t}");
        assert!(t.ends_with('Z') && &t[4..5] == "-" && &t[10..11] == "T", "{t}");
    }

    #[test]
    fn portable_documents_hold_no_hints() {
        let doc = json!({"format": FORMAT, "store": "C:/a/store", "options": {"eyes": "C:/a/eyes.json", "face_prefix": "faceunits/"},
                         "resources": {"store": {"kind": "store", "identity": "sha256:00", "hint": "C:/a/store"}, "eyes": {"kind": "file", "identity": "sha256:11", "hint": "assets/eyes.json"},
                                       "panels.stitch.body": {"kind": "dir", "identity": "sha256:22", "by": "content", "hint": "/home/q/body/"}}});
        let p = portable_doc(&doc);
        assert_eq!(p["resources"]["eyes"], json!({"kind": "file", "identity": "sha256:11", "file": "eyes.json"}));
        assert_eq!(p["resources"]["panels.stitch.body"]["file"], "body");
        assert_eq!(p["options"], json!({"face_prefix": "faceunits/"}));
        assert!(p.get("store").is_none());
        let root = std::env::temp_dir().join("gf_portable_test_root");
        let hits: Vec<String> = res::private_hits(&p, &root).into_iter().map(|h| h.pointer).collect();
        assert!(hits.is_empty(), "{hits:?}");
        assert!(!res::private_hits(&doc, &root).is_empty());
    }

    #[test]
    fn concurrent_atomic_writes_never_leave_a_torn_file() {
        let dir = std::env::temp_dir().join(format!("gf_project_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.gfproj.json");
        let bodies: Vec<Vec<u8>> = (0..8).map(|w| serde_json::to_vec(&json!({"writer": w, "pad": "x".repeat(200_000 + w * 1000)})).unwrap()).collect();
        write_atomic(&path, &bodies[0]).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (path, stop, bodies) = (path.clone(), stop.clone(), bodies.clone());
            std::thread::spawn(move || {
                let mut reads = 0;
                while !stop.load(Ordering::Relaxed) {
                    let b = read_retry(&path).unwrap();
                    assert!(bodies.contains(&b), "a reader saw a torn or foreign file of {} bytes", b.len());
                    reads += 1;
                }
                reads
            })
        };
        let writers: Vec<_> = (0..8)
            .map(|w| {
                let (path, body) = (path.clone(), bodies[w].clone());
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        write_atomic(&path, &body).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        let reads = reader.join().unwrap();
        assert!(reads > 0);
        assert!(bodies.contains(&std::fs::read(&path).unwrap()));
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "temporary files left: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
