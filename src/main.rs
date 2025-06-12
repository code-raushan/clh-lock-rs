use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use crossbeam_utils::CachePadded;
use crossbeam_utils::Backoff;


struct Node {
    locked: AtomicBool
}

struct Token(*const CachePadded<Node>);

struct CLHLock {
    tail: AtomicPtr<CachePadded<Node>>,
}

impl Node {
    fn new(locked: bool) -> *mut CachePadded<Self> {
        Box::into_raw(Box::new(CachePadded::new(Self {
            locked: AtomicBool::new(locked),
        })))
    }
}

impl CLHLock {
    fn new() -> Self {
        Self {
            tail: AtomicPtr::new(Node::new(false)),
        }
    }

    fn lock(&self) -> Token {
        let backoff = Backoff::new();
        let node = Node::new(true);
        let prev = self.tail.swap(node, Ordering::AcqRel);

        while unsafe { (*prev).locked.load(Ordering::Acquire)} {
            backoff.snooze();
        }

        drop(unsafe { Box::from_raw(prev) });

        Token(node)
    }

    fn unlock(&self, token: Token) {
        let node = unsafe { &*token.0 };
        node.locked.store(false, Ordering::Release);
    }
}

fn main() {
    let lock = Arc::new(CLHLock::new());
    let counter = Arc::new(AtomicUsize::new(0));
    let num_threads = 4;
    let num_iterations = 1000;

    let mut handles = vec![];

    for _ in 0..num_threads {
        let lock_clone = Arc::clone(&lock);
        let counter_clone = Arc::clone(&counter);
        
        let handle = thread::spawn(move || {
            for _ in 0..num_iterations {
                let token = lock_clone.lock();
                // Critical section
                counter_clone.fetch_add(1, Ordering::Relaxed);
                lock_clone.unlock(token);
            }
        });
        
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    println!("Final counter value: {}", counter.load(Ordering::Relaxed));
    println!("Expected value: {}", num_threads * num_iterations);
}
