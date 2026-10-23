use eframe::egui;
use serde_json::Value;

use crate::i18n::{tr, tr_or, trf};

pub fn rows(measurements: &serde_json::Map<String, Value>) -> Vec<(String, f64, f64, String)> {
    measurements
        .iter()
        .filter_map(|(k, x)| Some((k.clone(), x["mm"].as_f64()?, x["delta_mm"].as_f64()?, x["what"].as_str().unwrap_or_default().to_string())))
        .collect()
}

pub fn box_mm(lo: [f64; 3], hi: [f64; 3]) -> String {
    (0..3).map(|k| format!("{:.1}", (hi[k] - lo[k]) * 1000.0)).collect::<Vec<_>>().join(" x ")
}

impl crate::studio::Studio<'_> {
    pub fn measure_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("panel.measurements")).id_salt("measurements").default_open(false).show(ui, |ui| {
            let Some(s) = &self.world.session else {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            };
            let table = s.settled.as_ref().map(|st| rows(&st.measurements)).unwrap_or_default();
            let lods = s.head.lods.iter().map(|l| (l.indices.len() / 3).to_string()).collect::<Vec<_>>().join(" / ");
            egui::Grid::new("measure_grid").num_columns(3).striped(true).show(ui, |ui| {
                ui.label(egui::RichText::new(tr("measure.col.measure")).strong());
                ui.label(egui::RichText::new(tr("unit.mm")).strong());
                ui.label(egui::RichText::new(tr("measure.col.change")).strong());
                ui.end_row();
                for (k, mm, delta, what) in &table {
                    ui.label(tr_or(&format!("measure.{k}"), k)).on_hover_text(tr_or(&format!("measure.{k}.what"), what));
                    ui.label(format!("{mm:.2}"));
                    ui.label(format!("{}{delta:.2}", if *delta >= 0.0 { "+" } else { "" }));
                    ui.end_row();
                }
            });
            ui.checkbox(&mut self.world.measure_box, tr("measure.bbox"));
            if let Some((lo, hi)) = self.world.state.head_box {
                ui.label(egui::RichText::new(trf("measure.box", &[("size", &box_mm(lo, hi))])).weak());
            }
            ui.label(egui::RichText::new(trf("measure.lods", &[("list", &lods)])).weak());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_skip_entries_without_numbers_and_box_reads_in_mm() {
        let m = json!({"nose_width": {"mm": 35.5, "delta_mm": -1.25, "what": "w"}, "broken": {"mm": "x"}});
        let r = rows(m.as_object().unwrap());
        assert_eq!(r, vec![("nose_width".to_string(), 35.5, -1.25, "w".to_string())]);
        assert_eq!(box_mm([0.0, -0.1, 0.0], [0.15, 0.1, 0.2]), "150.0 x 200.0 x 200.0");
    }
}
