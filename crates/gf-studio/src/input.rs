use eframe::egui;

use crate::shape::{Edit, ShapeEvent, ShapeModel};

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

#[derive(Default)]
pub struct SliderPanel {
    search: String,
    category: Option<String>,
}

enum Row {
    Header(usize),
    Target(usize),
}

fn slider_row(ui: &mut egui::Ui, label: &str, value: f64, start: f64, edit: impl Fn(f64) -> Edit, out: &mut Vec<ShapeEvent>) {
    let mut v = value;
    let changed_from_start = value != start;
    let text = if changed_from_start { egui::RichText::new(label).strong() } else { egui::RichText::new(label) };
    let slider = egui::Slider::new(&mut v, 0.0..=1.0).custom_formatter(|x, _| format!("{x:.3}")).text(text);
    let resp = ui.add(slider).on_hover_text("right click: back to the start value");
    if resp.drag_started() {
        out.push(ShapeEvent::Begin(vec![label.to_string()]));
    }
    if resp.secondary_clicked() {
        v = start;
    }
    if v != value {
        out.push(ShapeEvent::Preview(vec![edit(v)]));
        if !resp.dragged() {
            out.push(ShapeEvent::Commit(format!("{label} {value:.3} to {v:.3}")));
        }
    }
    if resp.drag_stopped() {
        out.push(ShapeEvent::Commit(format!("{label} to {v:.3}")));
    }
}

impl ShapeInput for SliderPanel {
    fn name(&self) -> &'static str {
        "sliders"
    }

    fn show(&mut self, ui: &mut egui::Ui, shape: &ShapeModel) -> Vec<ShapeEvent> {
        let mut out = Vec::new();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("search targets").desired_width(150.0));
            let label = self.category.clone().unwrap_or_else(|| "all categories".into());
            egui::ComboBox::from_id_salt("category").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.category, None, "all categories");
                for (c, n) in &shape.categories {
                    ui.selectable_value(&mut self.category, Some(c.clone()), format!("{c} ({n})"));
                }
            });
            if ui.button("reset").on_hover_text("every slider back to the start values").clicked() {
                out.push(ShapeEvent::Reset);
            }
        });
        egui::CollapsingHeader::new("macros").default_open(true).show(ui, |ui| {
            if !shape.macros_available {
                ui.label(format!("unavailable: {}", shape.macros_reason.as_deref().unwrap_or("no macro definition")));
                return;
            }
            for m in &shape.macros {
                let value = shape.live.macros.get(&m.name).copied().unwrap_or(m.start);
                let name = m.name.clone();
                slider_row(ui, &m.name, value, m.start, |x| Edit::Macro(name.clone(), x), &mut out);
            }
        });
        let q = self.search.to_lowercase();
        let mut rows = Vec::new();
        let mut current: Option<&str> = None;
        for (i, t) in shape.targets.iter().enumerate() {
            if self.category.as_deref().is_some_and(|c| c != t.category) || (!q.is_empty() && !t.id.to_lowercase().contains(&q)) {
                continue;
            }
            if current != Some(t.category.as_str()) {
                current = Some(t.category.as_str());
                let c = shape.categories.iter().position(|(c, _)| *c == t.category).unwrap_or(0);
                rows.push(Row::Header(c));
            }
            rows.push(Row::Target(i));
        }
        let h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        egui::ScrollArea::vertical().id_salt("targets").auto_shrink([false, false]).show_rows(ui, h, rows.len(), |ui, range| {
            for r in &rows[range] {
                match *r {
                    Row::Header(c) => {
                        let (name, n) = &shape.categories[c];
                        ui.add_sized([ui.available_width(), h - ui.spacing().item_spacing.y], egui::Label::new(egui::RichText::new(format!("{name} ({n})")).strong()));
                    }
                    Row::Target(i) => {
                        let t = &shape.targets[i];
                        let id = t.id.clone();
                        slider_row(ui, &t.label, shape.live.weight(&t.id), shape.start.weight(&t.id), |x| Edit::Weight(id.clone(), x), &mut out);
                    }
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
        let Some(font) = crate::app::system_font() else { return };
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert("ui".into(), std::sync::Arc::new(egui::FontData::from_owned(font)));
        fonts.families.insert(egui::FontFamily::Proportional, vec!["ui".into()]);
        fonts.families.insert(egui::FontFamily::Monospace, vec!["ui".into()]);
        ctx.set_fonts(fonts);
        let shape = ShapeModel::from_sliders(&json!({
            "targets": [{"id": "nose/a", "category": "nose"}, {"id": "chin/b", "category": "chin"}],
            "macros": {"available": true, "sliders": [{"name": "age", "base": 0.5, "value": 0.123456789}]},
            "start": {"weights": {"nose/a": 0.987654321, "chin/b": 0.0000123}, "macros": null}
        }));
        let mut panel = SliderPanel::default();
        let mut events = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                events.extend(panel.show(ui, &shape));
            });
        }
        assert!(events.is_empty(), "{events:?}");
    }
}
