use std::ops::RangeInclusive;

use eframe::egui;
use gf_core::eyerig::{self as er, EyeAdjust, EyeballSpec};

use crate::shape::{Edit, ShapeEvent, ShapeModel, Slider, pair_edits};

const EYE_FIELDS: [(&str, f64, f64); 7] = [
    ("out mm", -er::OFFSET_MM_RANGE, er::OFFSET_MM_RANGE),
    ("up mm", -er::OFFSET_MM_RANGE, er::OFFSET_MM_RANGE),
    ("forward mm", -er::OFFSET_MM_RANGE, er::OFFSET_MM_RANGE),
    ("pitch up deg", -er::ROTATION_DEG_RANGE, er::ROTATION_DEG_RANGE),
    ("yaw out deg", -er::ROTATION_DEG_RANGE, er::ROTATION_DEG_RANGE),
    ("roll deg", -er::ROTATION_DEG_RANGE, er::ROTATION_DEG_RANGE),
    ("scale", er::SCALE_RANGE[0], er::SCALE_RANGE[1]),
];

fn eye_field(a: &EyeAdjust, f: usize) -> f64 {
    match f {
        0..=2 => a.offset_mm[f],
        3..=5 => a.rotation_deg[f - 3],
        _ => a.scale,
    }
}

fn set_eye_field(a: &mut EyeAdjust, f: usize, x: f64) {
    match f {
        0..=2 => a.offset_mm[f] = x,
        3..=5 => a.rotation_deg[f - 3] = x,
        _ => a.scale = x,
    }
}

fn no_eyeballs() -> EyeballSpec {
    EyeballSpec { follow: er::FOLLOW_NONE.into(), rig: None, model: None, left: EyeAdjust::default(), right: EyeAdjust::default() }
}

pub trait ShapeInput {
    fn name(&self) -> &'static str;
    fn show(&mut self, ui: &mut egui::Ui, shape: &ShapeModel) -> Vec<ShapeEvent>;
}

pub const INPUTS: &[&str] = &["sliders"];

pub fn by_name(name: &str) -> Option<Box<dyn ShapeInput>> {
    match name {
        "sliders" => Some(Box::new(SliderPanel::default())),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Tab {
    #[default]
    Macros,
    Eyes,
    All,
    Category(String),
}

#[derive(Default)]
pub struct SliderPanel {
    search: String,
    pub tab: Tab,
    pub unlink_eyes: bool,
}

enum Row {
    Header(usize),
    Slider(usize),
}

struct RowSpec<'a> {
    label: &'a str,
    value: f64,
    start: f64,
    range: RangeInclusive<f64>,
    ends: Option<(&'static str, &'static str)>,
    hover: String,
}

fn slider_row(ui: &mut egui::Ui, r: RowSpec, edits: impl Fn(f64) -> Vec<Edit>, reset: Vec<Edit>, out: &mut Vec<ShapeEvent>) {
    let (label, value) = (r.label, r.value);
    let mut v = value;
    let text = if value != r.start { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
    let ends = r.ends;
    let slider = egui::Slider::new(&mut v, r.range).text(text).custom_formatter(move |x, _| match ends {
        Some((neg, _)) if x < 0.0 => format!("{neg} {:.3}", -x),
        Some((_, pos)) if x > 0.0 => format!("{pos} {x:.3}"),
        _ => format!("{x:.3}"),
    });
    let resp = ui.add(slider).on_hover_text(r.hover);
    if resp.drag_started() {
        out.push(ShapeEvent::Begin(vec![label.to_string()]));
    }
    if resp.secondary_clicked() {
        out.push(ShapeEvent::Preview(reset));
        out.push(ShapeEvent::Commit(format!("{label} back to {:.3}", r.start)));
        return;
    }
    if v != value {
        out.push(ShapeEvent::Preview(edits(v)));
        if !resp.dragged() {
            out.push(ShapeEvent::Commit(format!("{label} {value:.3} to {v:.3}")));
        }
    }
    if resp.drag_stopped() {
        out.push(ShapeEvent::Commit(format!("{label} to {v:.3}")));
    }
}

fn target_row(ui: &mut egui::Ui, shape: &ShapeModel, s: &Slider, out: &mut Vec<ShapeEvent>) {
    let (live, start) = (&shape.live, &shape.start);
    match s {
        Slider::One(i) => {
            let t = &shape.targets[*i];
            let id = t.id.clone();
            let spec = RowSpec { label: &t.label, value: live.weight(&t.id), start: start.weight(&t.id), range: 0.0..=1.0, ends: None, hover: format!("{}\nright click: back to the start value", t.id) };
            let reset = vec![Edit::Weight(t.id.clone(), start.weight(&t.id))];
            slider_row(ui, spec, |x| vec![Edit::Weight(id.clone(), x)], reset, out);
        }
        Slider::Pair { label, neg, pos, ends } => {
            let (n, p) = (&shape.targets[*neg].id, &shape.targets[*pos].id);
            let (wn, wp) = (live.weight(n), live.weight(p));
            let mut hover = format!("left: {n}\nright: {p}\nright click: back to the start values");
            if wn > 0.0 && wp > 0.0 {
                hover.push_str(&format!("\nboth are set ({} {wn:.3}, {} {wp:.3}); a drag keeps only one side", ends.0, ends.1));
            }
            let spec = RowSpec { label, value: wp - wn, start: start.weight(p) - start.weight(n), range: -1.0..=1.0, ends: Some(*ends), hover };
            let reset = vec![Edit::Weight(n.clone(), start.weight(n)), Edit::Weight(p.clone(), start.weight(p))];
            slider_row(ui, spec, |x| pair_edits(n, p, x), reset, out);
        }
    }
}

impl SliderPanel {
    fn tabs(&mut self, ui: &mut egui::Ui, shape: &ShapeModel) {
        ui.horizontal_wrapped(|ui| {
            let mut pick = |ui: &mut egui::Ui, tab: Tab, text: String| {
                if ui.selectable_label(self.tab == tab, text).clicked() {
                    self.tab = tab;
                }
            };
            pick(ui, Tab::Macros, format!("Macros ({})", shape.macros.len()));
            pick(ui, Tab::Eyes, "Eyes".into());
            pick(ui, Tab::All, format!("All ({})", shape.sliders.len()));
            for (c, _) in &shape.categories {
                let n = shape.sliders.iter().filter(|s| shape.targets[s.first()].category == *c).count();
                pick(ui, Tab::Category(c.clone()), format!("{c} ({n})"));
            }
        });
    }

    fn macros(&self, ui: &mut egui::Ui, shape: &ShapeModel, out: &mut Vec<ShapeEvent>) {
        if !shape.macros_available {
            ui.label(format!("unavailable: {}", shape.macros_reason.as_deref().unwrap_or("no macro definition")));
            return;
        }
        for m in &shape.macros {
            let value = shape.live.macros.get(&m.name).copied().unwrap_or(m.start);
            let name = m.name.clone();
            let spec = RowSpec { label: &m.name, value, start: m.start, range: 0.0..=1.0, ends: None, hover: "right click: back to the start value".into() };
            slider_row(ui, spec, |x| vec![Edit::Macro(name.clone(), x)], vec![Edit::Macro(m.name.clone(), m.start)], out);
        }
    }

    fn eyes(&mut self, ui: &mut egui::Ui, shape: &ShapeModel, out: &mut Vec<ShapeEvent>) {
        let live = shape.live.eyeballs.clone().unwrap_or_else(no_eyeballs);
        let start = shape.start.eyeballs.clone().unwrap_or_else(no_eyeballs);
        let rig = live.rig.clone().or_else(|| shape.eyerig.clone());
        if let Some(rig) = &rig {
            let models = gf_core::eye_inventory::models();
            let current = live.model.as_ref().map(|m| m.id.clone());
            let shown = current.as_ref().and_then(|id| models.iter().find(|m| &m.id == id)).map_or("eyes file / none".to_string(), |m| m.name.clone());
            let mut picked = current.clone();
            egui::ComboBox::from_label("eye model").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut picked, None, "eyes file / none");
                let mut family = String::new();
                for m in &models {
                    if m.family != family {
                        family = m.family.clone();
                        ui.label(egui::RichText::new(&family).weak());
                    }
                    ui.selectable_value(&mut picked, Some(m.id.clone()), &m.name);
                }
            });
            if picked != current {
                let radius_mm = live.model.as_ref().map_or(gf_api::eyes::DEFAULT_RADIUS_MM, |m| m.radius_mm);
                let s = EyeballSpec { model: picked.clone().map(|id| er::EyeModelChoice { id, radius_mm }), rig: Some(rig.clone()), ..live.clone() };
                out.push(ShapeEvent::Preview(vec![Edit::Eyes(Some(s))]));
                out.push(ShapeEvent::Commit(picked.map_or("no eye model".into(), |id| format!("eye model {id}"))));
            }
            if let Some(m) = &live.model {
                let start_r = start.model.as_ref().map_or(gf_api::eyes::DEFAULT_RADIUS_MM, |m| m.radius_mm);
                let with = |x: f64| vec![Edit::Eyes(Some(EyeballSpec { model: Some(er::EyeModelChoice { id: m.id.clone(), radius_mm: x }), ..live.clone() }))];
                let spec = RowSpec { label: "eye radius mm", value: m.radius_mm, start: start_r, range: 5.0..=20.0, ends: None, hover: "eyeball radius in mm\nright click: back to the start value".into() };
                slider_row(ui, spec, with, with(start_r), out);
            }
            ui.separator();
        }
        let mut follow = live.follows();
        let toggle = ui
            .add_enabled(rig.is_some() || follow, egui::Checkbox::new(&mut follow, "follow the sockets"))
            .on_hover_text("move and scale each eye with its socket, the way MakeHuman moves its helper eyeball; the eyelid clearance uses the moved eyes");
        if toggle.changed() {
            let s = EyeballSpec { follow: if follow { er::FOLLOW_HELPER } else { er::FOLLOW_NONE }.into(), rig: if follow { rig.clone() } else { live.rig.clone() }, ..live.clone() };
            out.push(ShapeEvent::Preview(vec![Edit::Eyes(Some(s))]));
            out.push(ShapeEvent::Commit(format!("eyes {}", if follow { "follow the sockets" } else { "stay where the eyes file puts them" })));
        }
        if rig.is_none() {
            ui.label("this catalog has no eye rig (store import-eyerig makes one)");
        }
        let mut linked = !self.unlink_eyes;
        ui.checkbox(&mut linked, "link left and right").on_hover_text("one set of sliders for both eyes; out and yaw mirror between the sides");
        self.unlink_eyes = !linked;
        ui.separator();
        let sides: &[(&str, &[usize])] = if linked { &[("both eyes", &[0, 1])] } else { &[("left eye", &[0]), ("right eye", &[1])] };
        for &(title, which) in sides {
            ui.label(egui::RichText::new(title).strong());
            let adjust = |s: &EyeballSpec| if which[0] == 0 { s.left } else { s.right };
            for (f, &(name, lo, hi)) in EYE_FIELDS.iter().enumerate() {
                let label = format!("{} {name}", if linked { "eyes" } else { title.trim_end_matches(" eye") });
                let value = eye_field(&adjust(&live), f);
                let first = eye_field(&adjust(&start), f);
                let with = |x: f64| {
                    let mut s = live.clone();
                    for &side in which {
                        set_eye_field(if side == 0 { &mut s.left } else { &mut s.right }, f, x);
                    }
                    vec![Edit::Eyes(Some(s))]
                };
                let reset = with(first);
                let spec = RowSpec { label: &label, value, start: first, range: lo..=hi, ends: None, hover: format!("{name} relative to the socket\nright click: back to the start value") };
                slider_row(ui, spec, with, reset, out);
            }
        }
    }
}

impl ShapeInput for SliderPanel {
    fn name(&self) -> &'static str {
        "sliders"
    }

    fn show(&mut self, ui: &mut egui::Ui, shape: &ShapeModel) -> Vec<ShapeEvent> {
        let mut out = Vec::new();
        self.tabs(ui, shape);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("search all targets").desired_width(150.0));
            if ui.button("reset").on_hover_text("every slider back to the start values").clicked() {
                out.push(ShapeEvent::Reset);
            }
        });
        ui.separator();
        let q = self.search.trim().to_lowercase();
        if q.is_empty() && self.tab == Tab::Macros {
            self.macros(ui, shape, &mut out);
            return out;
        }
        if q.is_empty() && self.tab == Tab::Eyes {
            self.eyes(ui, shape, &mut out);
            return out;
        }
        let only = match (&self.tab, q.is_empty()) {
            (Tab::Category(c), true) => Some(c.as_str()),
            _ => None,
        };
        let mut rows = Vec::new();
        let mut current: Option<&str> = None;
        for (k, s) in shape.sliders.iter().enumerate() {
            let category = shape.targets[s.first()].category.as_str();
            if only.is_some_and(|c| c != category) || (!q.is_empty() && !s.matches(&shape.targets, &q)) {
                continue;
            }
            if only.is_none() && current != Some(category) {
                current = Some(category);
                rows.push(Row::Header(shape.categories.iter().position(|(c, _)| c == category).unwrap_or(0)));
            }
            rows.push(Row::Slider(k));
        }
        let h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical().id_salt("targets").auto_shrink([false, false]).show_rows(ui, h, rows.len(), |ui, range| {
            for r in &rows[range] {
                match *r {
                    Row::Header(c) => {
                        let name = &shape.categories[c].0;
                        ui.add_sized([ui.available_width(), h - ui.spacing().item_spacing.y], egui::Label::new(egui::RichText::new(name.as_str()).strong()));
                    }
                    Row::Slider(k) => target_row(ui, shape, &shape.sliders[k], &mut out),
                }
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn showing_values_off_any_grid_emits_no_event() {
        let Some(font) = crate::studio::system_font() else { return };
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert("ui".into(), std::sync::Arc::new(egui::FontData::from_owned(font)));
        fonts.families.insert(egui::FontFamily::Proportional, vec!["ui".into()]);
        fonts.families.insert(egui::FontFamily::Monospace, vec!["ui".into()]);
        ctx.set_fonts(fonts);
        let shape = ShapeModel::from_sliders(&json!({
            "targets": [{"id": "nose/a", "category": "nose"}, {"id": "chin/b", "category": "chin"},
                        {"id": "chin/w-decr", "category": "chin"}, {"id": "chin/w-incr", "category": "chin"}],
            "macros": {"available": true, "sliders": [{"name": "age", "base": 0.5, "value": 0.123456789}]},
            "start": {"weights": {"nose/a": 0.987654321, "chin/b": 0.0000123, "chin/w-decr": 0.3141592653, "chin/w-incr": 0.0271828}, "macros": null}
        }));
        let mut panel = SliderPanel::default();
        let mut events = Vec::new();
        for tab in [Tab::Macros, Tab::Eyes, Tab::All, Tab::Category("chin".into())] {
            panel.tab = tab;
            for _ in 0..3 {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    events.extend(panel.show(ui, &shape));
                });
            }
        }
        assert!(events.is_empty(), "{events:?}");
    }
}
