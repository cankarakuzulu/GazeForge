use std::collections::VecDeque;
use std::time::Instant;

use serde_json::{Value, json};

use crate::i18n::trf;

pub struct Ring {
    cap: usize,
    v: VecDeque<f64>,
    pub total: u64,
}

impl Ring {
    pub fn new(cap: usize) -> Ring {
        Ring { cap, v: VecDeque::with_capacity(cap), total: 0 }
    }

    pub fn push(&mut self, x: f64) {
        if self.v.len() == self.cap {
            self.v.pop_front();
        }
        self.v.push_back(x);
        self.total += 1;
    }

    pub fn clear(&mut self) {
        self.v.clear();
    }

    pub fn quantile(&self, q: f64) -> Option<f64> {
        if self.v.is_empty() {
            return None;
        }
        let mut s: Vec<f64> = self.v.iter().copied().collect();
        s.sort_by(|a, b| a.total_cmp(b));
        Some(s[((s.len() - 1) as f64 * q).round() as usize])
    }

    pub fn last(&self) -> Option<f64> {
        self.v.back().copied()
    }

    pub fn json(&self) -> Value {
        if self.v.is_empty() {
            return Value::Null;
        }
        let n = self.v.len();
        json!({"count": n, "mean": self.v.iter().sum::<f64>() / n as f64, "p50": self.quantile(0.5), "p95": self.quantile(0.95),
               "max": self.v.iter().copied().fold(f64::MIN, f64::max), "last": self.last()})
    }
}

pub struct Hud {
    pub on: bool,
    pub frame_interval: Ring,
    pub frame_cpu: Ring,
    pub frame_gpu: Ring,
    pub preview_compute: Ring,
    pub preview_latency: Ring,
    pub settle_latency: Ring,
    pub settle_worker: Ring,
    pub slope: Ring,
    pub samples: Ring,
    pub presents: VecDeque<Instant>,
}

impl Default for Hud {
    fn default() -> Hud {
        Hud {
            on: false,
            frame_interval: Ring::new(240),
            frame_cpu: Ring::new(240),
            frame_gpu: Ring::new(240),
            preview_compute: Ring::new(240),
            preview_latency: Ring::new(240),
            settle_latency: Ring::new(64),
            settle_worker: Ring::new(64),
            slope: Ring::new(64),
            samples: Ring::new(64),
            presents: VecDeque::new(),
        }
    }
}

impl Hud {
    pub fn presented(&mut self, at: Instant) {
        if let Some(&prev) = self.presents.back() {
            self.frame_interval.push((at - prev).as_secs_f64() * 1000.0);
        }
        self.presents.push_back(at);
        while self.presents.front().is_some_and(|t| (at - *t).as_secs_f64() > 1.0) {
            self.presents.pop_front();
        }
    }

    pub fn present_hz(&self) -> f64 {
        match (self.presents.front(), self.presents.back()) {
            (Some(a), Some(b)) if self.presents.len() > 1 => (self.presents.len() - 1) as f64 / (*b - *a).as_secs_f64().max(1e-9),
            _ => 0.0,
        }
    }

    pub fn lines(&self) -> Vec<String> {
        let q = |r: &Ring| match (r.quantile(0.5), r.quantile(0.95)) {
            (Some(a), Some(b)) => format!("{a:.2} / {b:.2} ms"),
            _ => "-".into(),
        };
        vec![
            trf("studio.hud.present", &[("v", &format!("{:.1}", self.present_hz()))]),
            trf("studio.hud.interval", &[("v", &q(&self.frame_interval))]),
            trf("studio.hud.cpu", &[("v", &q(&self.frame_cpu))]),
            trf("studio.hud.compute", &[("v", &q(&self.preview_compute))]),
            trf("studio.hud.latency", &[("v", &q(&self.preview_latency))]),
            trf("studio.hud.settle", &[("v", &q(&self.settle_latency))]),
            trf("studio.hud.slope", &[("v", &q(&self.slope))]),
            trf("studio.hud.samples", &[("v", &q(&self.samples))]),
        ]
    }

    pub fn json(&self) -> Value {
        json!({"present_hz": self.present_hz(), "frame_interval_ms": self.frame_interval.json(), "frame_cpu_ms": self.frame_cpu.json(), "frame_gpu_complete_ms": self.frame_gpu.json(),
               "preview_compute_ms": self.preview_compute.json(), "input_to_preview_ms": self.preview_latency.json(), "settle_latency_ms": self.settle_latency.json(),
               "settle_worker_ms": self.settle_worker.json(), "macro_slope_ms": self.slope.json(), "macro_samples_ms": self.samples.json()})
    }
}
