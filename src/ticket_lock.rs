#[cfg(loom)]
use loom::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(loom)]
use loom::hint::spin_loop;
#[cfg(not(loom))]
use std::hint::spin_loop;

pub struct TicketLock {
    next: AtomicUsize,
    curr: AtomicUsize,
}

impl TicketLock {
    pub fn new() -> Self {
        TicketLock {
            next: AtomicUsize::new(0),
            curr: AtomicUsize::new(0),
        }
    }

    pub fn lock(&self) -> usize {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        while self.curr.load(Ordering::Acquire) != ticket {
            spin_loop();
        }
        ticket
    }

    pub fn unlock(&self, ticket: usize) {
        self.curr.store(ticket.wrapping_add(1), Ordering::Release);
    }
}

impl Default for TicketLock {
    fn default() -> Self {
        Self::new()
    }
}
