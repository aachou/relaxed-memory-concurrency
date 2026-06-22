use loom::sync::atomic::{fence, AtomicUsize, Ordering};
use loom::sync::Mutex;
use std::collections::HashSet;

const EPOCHS: usize = 3;
const SENTINEL: usize = usize::MAX;

pub struct Collector {
    global_epoch: AtomicUsize,
    max_threads: usize,
    local_epoch: Vec<AtomicUsize>,
    retire_lists: Mutex<[Vec<usize>; EPOCHS]>,
    freed: Mutex<HashSet<usize>>,
}

impl Collector {
    pub fn new(max_threads: usize) -> Self {
        let mut local_epoch = Vec::with_capacity(max_threads);
        for _ in 0..max_threads {
            local_epoch.push(AtomicUsize::new(SENTINEL));
        }
        Collector {
            global_epoch: AtomicUsize::new(0),
            max_threads,
            local_epoch,
            retire_lists: Mutex::new([Vec::new(), Vec::new(), Vec::new()]),
            freed: Mutex::new(HashSet::new()),
        }
    }

    pub fn pin(&self, tid: usize) -> Guard {
        let e = self.global_epoch.load(Ordering::Relaxed);
        self.local_epoch[tid].store(e, Ordering::Relaxed);
        fence(Ordering::SeqCst);
        Guard { tid }
    }

    pub fn unpin(&self, guard: Guard) {
        self.local_epoch[guard.tid].store(SENTINEL, Ordering::Release);
    }

    pub fn retire(&self, _tid: usize, obj: usize) {
        fence(Ordering::SeqCst);
        let epoch = self.global_epoch.load(Ordering::Relaxed);
        self.retire_lists.lock().unwrap()[epoch].push(obj);
    }

    pub fn is_freed(&self, obj: usize) -> bool {
        self.freed.lock().unwrap().contains(&obj)
    }

    pub fn try_advance(&self) -> bool {
        let g = self.global_epoch.load(Ordering::Relaxed);
        fence(Ordering::SeqCst);

        for tid in 0..self.max_threads {
            let e = self.local_epoch[tid].load(Ordering::Relaxed);
            if e != SENTINEL && e != g {
                return false;
            }
        }

        fence(Ordering::Acquire);

        let next = (g + 1) % EPOCHS;
        self.global_epoch.store(next, Ordering::Release);

        let old_epoch = (g + 2) % EPOCHS;
        let mut lists = self.retire_lists.lock().unwrap();
        let mut freed = self.freed.lock().unwrap();
        for obj in lists[old_epoch].drain(..) {
            freed.insert(obj);
        }

        true
    }
}

pub struct Guard {
    tid: usize,
}
