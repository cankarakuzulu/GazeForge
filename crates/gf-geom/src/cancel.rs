use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Cancelled;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static FLAG: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

struct Restore(Option<Arc<AtomicBool>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let previous = self.0.take();
        FLAG.with(|f| *f.borrow_mut() = previous);
    }
}

pub fn scope<R>(flag: &Arc<AtomicBool>, f: impl FnOnce() -> R) -> Result<R, Cancelled> {
    let _restore = Restore(FLAG.with(|cell| cell.borrow_mut().replace(flag.clone())));
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Ok(r),
        Err(payload) if payload.is::<Cancelled>() => Err(Cancelled),
        Err(payload) => resume_unwind(payload),
    }
}

pub fn point() {
    if FLAG.with(|f| f.borrow().as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed))) {
        resume_unwind(Box::new(Cancelled));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_flag_stops_at_the_next_point_and_an_unset_one_never_does() {
        let flag = Arc::new(AtomicBool::new(false));
        let mut reached = 0;
        let done = scope(&flag, || {
            for i in 0..10 {
                point();
                reached = i;
                if i == 4 {
                    flag.store(true, Ordering::Relaxed);
                }
            }
            7
        });
        assert!(done.is_err());
        assert_eq!(reached, 4);
        flag.store(false, Ordering::Relaxed);
        assert_eq!(scope(&flag, || 7).ok(), Some(7));
        point();
    }

    #[test]
    fn outside_a_scope_a_point_is_free_and_other_panics_pass_through() {
        point();
        let flag = Arc::new(AtomicBool::new(true));
        let other = catch_unwind(|| scope(&flag, || panic!("not a cancel")));
        assert!(other.is_err());
        point();
    }
}
