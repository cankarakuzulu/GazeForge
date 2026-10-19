use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use gf_api::designer::{self, MacroSamples, MacroSlope, TargetDeltas};
use gf_api::{ApiError, Ctx};
use serde_json::{Value, json};

use crate::session::{EdgePairs, Session, SettleOut, run_settle};
use crate::shape::Values;

pub struct Opened {
    pub session: Session,
    pub sliders: Value,
    pub deltas: TargetDeltas,
    pub deltas_ms: f64,
}

pub enum Job {
    Load { store: String, spec: Option<String>, macro_file: Option<String> },
    Slope { name: String, at: f64, input: Value, prefetch: bool },
    Samples { name: String, key: String, input: Value },
    Reference { input: Value },
    Save { input: Value },
}

pub enum Done {
    Loaded { epoch: u64, result: Result<Box<Opened>, String>, ms: f64 },
    Settled { epoch: u64, generation: u64, sent: Values, result: Result<SettleOut, String>, queued_ms: f64 },
    Slope { epoch: u64, name: String, at: f64, result: Result<MacroSlope, String>, ms: f64 },
    Samples { epoch: u64, name: String, key: String, result: Result<MacroSamples, String>, ms: f64 },
    Reference { epoch: u64, result: Result<Vec<f32>, String> },
    Saved { result: Result<Value, String> },
}

pub struct SettleRequest {
    pub epoch: u64,
    pub generation: u64,
    pub sent: Values,
    pub input: Value,
    pub at: Instant,
    pub edges: Option<Arc<EdgePairs>>,
}

#[derive(Default)]
struct Slot {
    next: Option<SettleRequest>,
    running: bool,
    cancelled: u64,
    dragging: bool,
    stop: Arc<AtomicBool>,
    aborted: u64,
    requeued: u64,
}

#[derive(Default, Clone, Copy)]
pub struct SettleState {
    pub queued: bool,
    pub running: bool,
    pub cancelled: u64,
    pub aborted: u64,
    pub requeued: u64,
}

type Wake = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

pub struct Jobs {
    tx: Sender<(u64, Job)>,
    rx: Receiver<Done>,
    slot: Arc<(Mutex<Slot>, Condvar)>,
    wake: Wake,
}

fn err(e: ApiError) -> String {
    serde_json::to_string(&e.to_json()).unwrap_or_else(|_| "gf-api error".into())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn slot_busy(slot: &Arc<(Mutex<Slot>, Condvar)>) -> bool {
    slot.0.lock().map(|s| s.running || s.next.is_some()).unwrap_or(false)
}

fn open(store: &str, spec: Option<&str>, macro_file: Option<&str>) -> Result<Box<Opened>, String> {
    let (session, sliders) = Session::open(store, spec, macro_file)?;
    let t = Instant::now();
    let deltas = designer::preview_deltas(&json!({"store": store, "pack": session.pack, "catalog": session.catalog, "game": session.game})).map_err(err)?;
    if deltas.vertices != session.head.n {
        return Err(format!("the preview deltas have {} vertices, the game base {}", deltas.vertices, session.head.n));
    }
    Ok(Box::new(Opened { session, sliders, deltas, deltas_ms: ms(t) }))
}

impl Jobs {
    pub fn start() -> Jobs {
        let (tx, jobs) = channel::<(u64, Job)>();
        let (done, rx) = channel::<Done>();
        let wake: Wake = Arc::new(Mutex::new(None));
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let notify = |wake: &Wake| {
            if let Ok(w) = wake.lock()
                && let Some(f) = w.as_ref()
            {
                f();
            }
        };
        {
            let (done, wake, gate) = (done.clone(), wake.clone(), slot.clone());
            let cores = thread::available_parallelism().map_or(4, |n| n.get());
            let samples_pool = rayon::ThreadPoolBuilder::new().num_threads((cores / 2).max(1)).thread_name(|i| format!("samples-{i}")).build().ok();
            thread::Builder::new()
                .name("studio-jobs".into())
                .spawn(move || {
                    for (epoch, job) in jobs {
                        let t = Instant::now();
                        let msg = match job {
                            Job::Load { store, spec, macro_file } => Done::Loaded { epoch, result: open(&store, spec.as_deref(), macro_file.as_deref()), ms: 0.0 },
                            Job::Slope { name, at, input, prefetch } => {
                                while prefetch && t.elapsed().as_secs_f64() < 10.0 && slot_busy(&gate) {
                                    thread::sleep(std::time::Duration::from_millis(5));
                                }
                                Done::Slope { epoch, name, at, result: designer::macro_slope(&input).map_err(err), ms: 0.0 }
                            }
                            Job::Samples { name, key, input } => {
                                while t.elapsed().as_secs_f64() < 10.0 && slot_busy(&gate) {
                                    thread::sleep(std::time::Duration::from_millis(5));
                                }
                                let t = Instant::now();
                                let result = match &samples_pool {
                                    Some(p) => p.install(|| designer::macro_samples(&input)),
                                    None => designer::macro_samples(&input),
                                };
                                Done::Samples { epoch, name, key, result: result.map_err(err), ms: ms(t) }
                            }
                            Job::Reference { input } => {
                                Done::Reference { epoch, result: designer::settle_mesh(&input).map(|s| s.positions.iter().flatten().map(|&x| x as f32).collect()).map_err(err) }
                            }
                            Job::Save { input } => Done::Saved { result: designer::save(&Ctx::from_env(), &input).map_err(err) },
                        };
                        let msg = match msg {
                            Done::Loaded { epoch, result, .. } => Done::Loaded { epoch, result, ms: ms(t) },
                            Done::Slope { epoch, name, at, result, .. } => Done::Slope { epoch, name, at, result, ms: ms(t) },
                            other => other,
                        };
                        if done.send(msg).is_err() {
                            break;
                        }
                        notify(&wake);
                    }
                })
                .expect("spawn the job thread");
        }
        {
            let (slot, wake) = (slot.clone(), wake.clone());
            let cores = thread::available_parallelism().map_or(4, |n| n.get());
            let threads = std::env::var("GAZEFORGE_SETTLE_THREADS").ok().and_then(|t| t.parse::<usize>().ok()).unwrap_or(cores).max(1);
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("settle-{i}")).build().ok();
            thread::Builder::new()
                .name("studio-settle".into())
                .spawn(move || {
                    loop {
                        let req = {
                            let (lock, cv) = &*slot;
                            let Ok(mut s) = lock.lock() else { return };
                            while s.next.is_none() || s.dragging {
                                s = match cv.wait(s) {
                                    Ok(s) => s,
                                    Err(_) => return,
                                };
                            }
                            s.running = true;
                            s.stop = Arc::new(AtomicBool::new(false));
                            (s.next.take().expect("checked above"), s.stop.clone())
                        };
                        let (req, stop) = req;
                        let queued_ms = ms(req.at);
                        let run = || gf_geom::cancel::scope(&stop, || run_settle(&req.input));
                        let ran = match &pool {
                            Some(p) => p.install(run),
                            None => run(),
                        };
                        let mut result = match ran {
                            Ok(r) => r,
                            Err(gf_geom::cancel::Cancelled) => {
                                if let Ok(mut s) = slot.0.lock() {
                                    s.running = false;
                                    s.aborted += 1;
                                    if s.next.is_none() {
                                        s.requeued += 1;
                                        s.next = Some(req);
                                    }
                                }
                                continue;
                            }
                        };
                        if let (Ok(out), Some(idx)) = (result.as_mut(), req.edges.as_ref()) {
                            let t = Instant::now();
                            out.flags = out.uvs.as_ref().map(|u| idx.flags(u));
                            out.server_ms["edge_flags"] = json!(ms(t));
                        }
                        let sent = Done::Settled { epoch: req.epoch, generation: req.generation, sent: req.sent, result, queued_ms };
                        let gone = done.send(sent).is_err();
                        if let Ok(mut s) = slot.0.lock() {
                            s.running = false;
                        }
                        if gone {
                            return;
                        }
                        notify(&wake);
                    }
                })
                .expect("spawn the settle thread");
        }
        Jobs { tx, rx, slot, wake }
    }

    pub fn set_wake(&self, f: Box<dyn Fn() + Send>) {
        if let Ok(mut w) = self.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn send(&self, epoch: u64, job: Job) {
        let _ = self.tx.send((epoch, job));
    }

    pub fn settle(&self, req: SettleRequest) {
        let (lock, cv) = &*self.slot;
        if let Ok(mut s) = lock.lock() {
            if s.next.is_some() {
                s.cancelled += 1;
            }
            s.next = Some(req);
            s.stop.store(true, Ordering::Relaxed);
            cv.notify_one();
        }
    }

    pub fn set_dragging(&self, on: bool) {
        let (lock, cv) = &*self.slot;
        if let Ok(mut s) = lock.lock()
            && s.dragging != on
        {
            s.dragging = on;
            if on {
                s.stop.store(true, Ordering::Relaxed);
            }
            cv.notify_one();
        }
    }

    pub fn settle_state(&self) -> SettleState {
        self.slot.0.lock().map(|s| SettleState { queued: s.next.is_some(), running: s.running, cancelled: s.cancelled, aborted: s.aborted, requeued: s.requeued }).unwrap_or_default()
    }

    pub fn try_recv(&self) -> Option<Done> {
        self.rx.try_recv().ok()
    }
}
