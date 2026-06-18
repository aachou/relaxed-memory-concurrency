#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

#[cfg(loom)]
use loom::hint::spin_loop;
#[cfg(not(loom))]
use std::hint::spin_loop;

struct Node {
    locked: AtomicBool,
}

impl Node {
    fn new(locked: bool) -> *mut Node {
        Box::into_raw(Box::new(Node { locked: AtomicBool::new(locked) }))
    }
}

pub struct CLHLock {
    tail: AtomicPtr<Node>,
}

pub struct Token(*const Node);

unsafe impl Send for Token {}

impl CLHLock {
    pub fn new() -> Self {
        CLHLock { tail: AtomicPtr::new(Node::new(false)) }
    }

    pub fn lock(&self) -> Token {
        let node = Node::new(true);
        let prev = self.tail.swap(node, Ordering::AcqRel);
        unsafe {
            while (*prev).locked.load(Ordering::Acquire) {
                spin_loop();
            }
            drop(Box::from_raw(prev));
        }
        Token(node)
    }

    pub fn unlock(&self, token: Token) {
        unsafe {
            (*token.0).locked.store(false, Ordering::Release);
        }
    }
}

impl Drop for CLHLock {
    fn drop(&mut self) {
        let node = self.tail.swap(std::ptr::null_mut(), Ordering::Acquire);
        if !node.is_null() {
            unsafe { drop(Box::from_raw(node)); }
        }
    }
}

unsafe impl Send for CLHLock {}
unsafe impl Sync for CLHLock {}

impl Default for CLHLock {
    fn default() -> Self {
        Self::new()
    }
}
