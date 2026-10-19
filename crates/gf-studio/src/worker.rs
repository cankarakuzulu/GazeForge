use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

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
    Load { store: String, spec: Option<String>, macro_file: Option<String>, eyes: Option<String> },
    Slope { name: String, key: String, input: Value, urgent: bool },
    Samples { id: u64, name: String, key: String, input: Value },
    Reference { input: Value },
    Save { input: Value },
}

pub enum Done {
    Loaded { epoch: u64, result: Result<Box<Opened>, String>, ms: f64 },
    Settled { epoch: u64, generation: u64, sent: Values, result: Result<SettleOut, String>, queued_ms: f64 },
    Slope { epoch: u64, name: String, key: String, result: Result<MacroSlope, String>, ms: f64 },
    Samples { epoch: u64, id: u64, name: String, key: String, result: Result<Option<MacroSamples>, String>, ms: f64 },
    Reference { epoch: u64, result: Result<Vec<f32>, String> },
    Saved { result: Result<Value, String> },
    Trace(Trace),
}

pub struct Trace {
    pub kind: &'static str,
    pub name: String,
    pub sent: f64,
    pub start: f64,
    pub run: f64,
    pub end: f64,
}

static T0: OnceLock<Instant> = OnceLock::new();

pub fn stamp(t: Instant) -> f64 {
    let t0 = *T0.get_or_init(Instant::now);
    t.checked_duration_since(t0).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
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
type Queued = (u64, Instant, Job);
type SlopeQueue = Arc<(Mutex<VecDeque<Queued>>, Condvar)>;

const SLOPE_THREADS: usize = 2;

pub struct Jobs {
    tx: Sender<Queued>,
    slopes: SlopeQueue,
    samples: Sender<Queued>,
    samples_stop: Arc<Mutex<Arc<AtomicBool>>>,
    samples_drop: Arc<AtomicU64>,
    slopes_pending: Arc<AtomicUsize>,
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

fn settle_pending(slot: &Arc<(Mutex<Slot>, Condvar)>) -> bool {
    slot.0.lock().map(|s| s.running || (s.next.is_some() && !s.dragging)).unwrap_or(false)
}

fn notify(wake: &Wake) {
    if let Ok(w) = wake.lock()
        && let Some(f) = w.as_ref()
    {
        f();
    }
}

fn reply(done: &Sender<Done>, wake: &Wake, msg: Done, trace: Trace) -> bool {
    let ok = done.send(msg).is_ok() && done.send(Done::Trace(trace)).is_ok();
    if ok {
        notify(wake);
    }
    ok
}

fn open(store: &str, spec: Option<&str>, macro_file: Option<&str>, eyes: Option<&str>) -> Result<Box<Opened>, String> {
    let (session, sliders) = Session::open(store, spec, macro_file, eyes)?;
    let t = Instant::now();
    let deltas = designer::preview_deltas(&json!({"store": store, "pack": session.pack, "catalog": session.catalog, "game": session.game})).map_err(err)?;
    if deltas.vertices != session.head.n {
        return Err(format!("the preview deltas have {} vertices, the game base {}", deltas.vertices, session.head.n));
    }
    Ok(Box::new(Opened { session, sliders, deltas, deltas_ms: ms(t) }))
}

fn next_slope(q: &SlopeQueue) -> Option<Queued> {
    let (lock, cv) = &**q;
    let mut items = lock.lock().ok()?;
    loop {
        let urgent = items.iter().position(|(_, _, j)| matches!(j, Job::Slope { urgent: true, .. }));
        if let Some(job) = urgent.and_then(|i| items.remove(i)).or_else(|| items.pop_front()) {
            return Some(job);
        }
        items = cv.wait(items).ok()?;
    }
}

impl Jobs {
    pub fn start() -> Jobs {
        stamp(Instant::now());
        let (tx, jobs) = channel::<Queued>();
        let (samples_tx, samples_rx) = channel::<Queued>();
        let slopes: SlopeQueue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let (done, rx) = channel::<Done>();
        let wake: Wake = Arc::new(Mutex::new(None));
        let slot: Arc<(Mutex<Slot>, Condvar)> = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let samples_stop = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
        let samples_drop = Arc::new(AtomicU64::new(0));
        let slopes_pending = Arc::new(AtomicUsize::new(0));
        let cores = thread::available_parallelism().map_or(4, |n| n.get());
        {
            let (done, wake) = (done.clone(), wake.clone());
            thread::Builder::new()
                .name("studio-jobs".into())
                .spawn(move || {
                    for (epoch, sent, job) in jobs {
                        let t = Instant::now();
                        let (kind, msg) = match job {
                            Job::Load { store, spec, macro_file, eyes } => {
                                let result = open(&store, spec.as_deref(), macro_file.as_deref(), eyes.as_deref());
                                ("load", Done::Loaded { epoch, result, ms: ms(t) })
                            }
                            Job::Reference { input } => (
                                "reference",
                                Done::Reference { epoch, result: designer::settle_mesh(&input).map(|s| s.positions.iter().flatten().map(|&x| x as f32).collect()).map_err(err) },
                            ),
                            Job::Save { input } => ("save", Done::Saved { result: designer::save(&Ctx::from_env(), &input).map_err(err) }),
                            Job::Slope { .. } | Job::Samples { .. } => continue,
                        };
                        let trace = Trace { kind, name: String::new(), sent: stamp(sent), start: stamp(t), run: stamp(t), end: stamp(Instant::now()) };
                        if !reply(&done, &wake, msg, trace) {
                            break;
                        }
                    }
                })
                .expect("spawn the job thread");
        }
        for i in 0..SLOPE_THREADS {
            let (done, wake, queue, gate, pending) = (done.clone(), wake.clone(), slopes.clone(), slot.clone(), slopes_pending.clone());
            thread::Builder::new()
                .name(format!("studio-slope-{i}"))
                .spawn(move || {
                    while let Some((epoch, sent, job)) = next_slope(&queue) {
                        let Job::Slope { name, key, input, .. } = job else {
                            pending.fetch_sub(1, Ordering::Relaxed);
                            continue;
                        };
                        let t = Instant::now();
                        while t.elapsed().as_secs_f64() < 10.0 && settle_pending(&gate) {
                            thread::sleep(Duration::from_millis(2));
                        }
                        let run = Instant::now();
                        let result = designer::macro_slope(&input).map_err(err);
                        pending.fetch_sub(1, Ordering::Relaxed);
                        let trace = Trace { kind: "slope", name: name.clone(), sent: stamp(sent), start: stamp(t), run: stamp(run), end: stamp(Instant::now()) };
                        if !reply(&done, &wake, Done::Slope { epoch, name, key, result, ms: ms(run) }, trace) {
                            break;
                        }
                    }
                })
                .expect("spawn a slope thread");
        }
        {
            let (done, wake, gate, stop_cell, dropped, pending) = (done.clone(), wake.clone(), slot.clone(), samples_stop.clone(), samples_drop.clone(), slopes_pending.clone());
            let samples_pool = rayon::ThreadPoolBuilder::new().num_threads((cores / 2).max(1)).thread_name(|i| format!("samples-{i}")).build().ok();
            thread::Builder::new()
                .name("studio-samples".into())
                .spawn(move || {
                    for (epoch, sent, job) in samples_rx {
                        let Job::Samples { id, name, key, input } = job else { continue };
                        let t = Instant::now();
                        let (result, run) = loop {
                            if dropped.load(Ordering::Relaxed) == id {
                                break (Ok(None), Instant::now());
                            }
                            if slot_busy(&gate) || pending.load(Ordering::Relaxed) > 0 {
                                thread::sleep(Duration::from_millis(5));
                                continue;
                            }
                            let stop = Arc::new(AtomicBool::new(false));
                            if let Ok(mut s) = stop_cell.lock() {
                                *s = stop.clone();
                            }
                            if slot_busy(&gate) || dropped.load(Ordering::Relaxed) == id {
                                continue;
                            }
                            let run = Instant::now();
                            let go = || gf_geom::cancel::scope(&stop, || designer::macro_samples(&input));
                            let ran = match &samples_pool {
                                Some(p) => p.install(go),
                                None => go(),
                            };
                            match ran {
                                Ok(r) => break (r.map(Some).map_err(err), run),
                                Err(gf_geom::cancel::Cancelled) => continue,
                            }
                        };
                        let trace = Trace { kind: "samples", name: name.clone(), sent: stamp(sent), start: stamp(t), run: stamp(run), end: stamp(Instant::now()) };
                        if !reply(&done, &wake, Done::Samples { epoch, id, name, key, result, ms: ms(run) }, trace) {
                            break;
                        }
                    }
                })
                .expect("spawn the samples thread");
        }
        {
            let (slot, wake) = (slot.clone(), wake.clone());
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
        Jobs { tx, slopes, samples: samples_tx, samples_stop, samples_drop, slopes_pending, rx, slot, wake }
    }

    pub fn set_wake(&self, f: Box<dyn Fn() + Send>) {
        if let Ok(mut w) = self.wake.lock() {
            *w = Some(f);
        }
    }

    pub fn send(&self, epoch: u64, job: Job) {
        let item = (epoch, Instant::now(), job);
        match &item.2 {
            Job::Slope { .. } => {
                let (lock, cv) = &*self.slopes;
                if let Ok(mut q) = lock.lock() {
                    self.slopes_pending.fetch_add(1, Ordering::Relaxed);
                    q.push_back(item);
                    cv.notify_one();
                }
            }
            Job::Samples { .. } => {
                let _ = self.samples.send(item);
            }
            _ => {
                let _ = self.tx.send(item);
            }
        }
    }

    fn stop_samples(&self) {
        if let Ok(s) = self.samples_stop.lock() {
            s.store(true, Ordering::Relaxed);
        }
    }

    pub fn drop_samples(&self, id: u64) {
        self.samples_drop.store(id, Ordering::Relaxed);
        self.stop_samples();
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
        self.stop_samples();
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
