use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui;

use crate::i18n::tr;
use crate::icons;

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Folder,
    Open(String),
    Save(String),
}

pub type Picker = fn(&Pick, Option<&Path>) -> Option<PathBuf>;

pub fn native(pick: &Pick, near: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new();
    if let Some(dir) = near.and_then(start_dir) {
        d = d.set_directory(dir);
    }
    match pick {
        Pick::Folder => d.pick_folder(),
        Pick::Open(ext) => filtered(d, ext).pick_file(),
        Pick::Save(ext) => filtered(d, ext).save_file(),
    }
}

fn filtered(d: rfd::FileDialog, ext: &str) -> rfd::FileDialog {
    match ext.rsplit('.').next().filter(|e| !e.is_empty()) {
        Some(e) => d.add_filter(ext.trim_start_matches('.'), &[e]),
        None => d,
    }
}

fn start_dir(p: &Path) -> Option<PathBuf> {
    if p.is_dir() { Some(p.to_path_buf()) } else { p.parent().filter(|d| d.is_dir()).map(Path::to_path_buf) }
}

pub struct Browse {
    picker: Picker,
    pending: Option<(String, Receiver<Option<PathBuf>>)>,
    done: Option<(String, PathBuf)>,
}

impl Default for Browse {
    fn default() -> Browse {
        Browse::with(native)
    }
}

impl Browse {
    pub fn with(picker: Picker) -> Browse {
        Browse { picker, pending: None, done: None }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn start(&mut self, key: &str, pick: Pick, near: &str) {
        if self.busy() {
            return;
        }
        let near = Some(PathBuf::from(near.trim())).filter(|p| !p.as_os_str().is_empty());
        if cfg!(target_os = "macos") {
            self.done = (self.picker)(&pick, near.as_deref()).map(|p| (key.to_string(), p));
            return;
        }
        let (tx, rx) = channel();
        let picker = self.picker;
        std::thread::spawn(move || {
            let _ = tx.send(picker(&pick, near.as_deref()));
        });
        self.pending = Some((key.to_string(), rx));
    }

    fn poll(&mut self) {
        let Some((key, rx)) = &self.pending else { return };
        match rx.try_recv() {
            Ok(picked) => {
                self.done = picked.map(|p| (key.clone(), p));
                self.pending = None;
            }
            Err(TryRecvError::Disconnected) => self.pending = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    pub fn take(&mut self, key: &str) -> Option<String> {
        self.poll();
        if self.done.as_ref().is_some_and(|(k, _)| k == key) {
            return self.done.take().map(|(_, p)| p.display().to_string());
        }
        None
    }

    pub fn button(&mut self, ui: &mut egui::Ui, key: &str, pick: Pick, near: &str) {
        let tip = match pick {
            Pick::Folder => tr("studio.browse_folder"),
            Pick::Open(_) => tr("studio.browse_open"),
            Pick::Save(_) => tr("studio.browse_save"),
        };
        if ui.add_enabled_ui(!self.busy(), |ui| icons::button(ui, "folder-open", false, tip)).inner.clicked() {
            self.start(key, pick, near);
        }
        if self.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub fn field(ui: &mut egui::Ui, browse: &mut Browse, key: &str, value: &mut String, hint: &str, pick: Pick) -> bool {
    let picked = browse.take(key).map(|p| *value = p).is_some();
    let typed = row(ui, |ui, v: &mut String| {
        browse.button(ui, key, pick, v);
        ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(f32::INFINITY)).changed()
    }, value);
    picked || typed
}

pub fn row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui, &mut String) -> R, value: &mut String) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui, value)).inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(pick: &Pick, near: Option<&Path>) -> Option<PathBuf> {
        match pick {
            Pick::Folder => near.map(|n| n.join("picked")),
            Pick::Open(ext) => Some(PathBuf::from(format!("a{ext}"))),
            Pick::Save(_) => None,
        }
    }

    fn wait(b: &mut Browse, key: &str) -> Option<String> {
        for _ in 0..200 {
            if let Some(p) = b.take(key) {
                return Some(p);
            }
            if !b.busy() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn a_pick_lands_in_the_field_that_asked_and_a_cancel_changes_nothing() {
        let mut b = Browse::with(fake);
        b.start("spec", Pick::Open(".gfchar.json".into()), "");
        b.start("store", Pick::Folder, "X");
        assert_eq!(b.take("store"), None, "one pick at a time: the second request is ignored");
        assert_eq!(wait(&mut b, "spec").as_deref(), Some("a.gfchar.json"));
        assert_eq!(b.take("spec"), None, "taken once");
        b.start("store", Pick::Folder, "  ");
        assert_eq!(wait(&mut b, "store"), None, "no start folder, the fake picks nothing");
        b.start("out", Pick::Save(".glb".into()), "x.glb");
        assert_eq!(wait(&mut b, "out"), None, "a cancelled dialog leaves the field alone");
        assert!(!b.busy());
    }
}
