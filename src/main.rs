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
unsafe impl Send for Token {}

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

impl Drop for CLHLock {
    fn drop(&mut self) {
        // Drop the node made by the last thread that `lock()`ed.
        let node = *self.tail.get_mut();

        // SAFETY: Since this is the tail node, no other thread has access to it.
        drop(unsafe { Box::from_raw(node) });
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


#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    // Helper function to test mutual exclusion
    fn test_mutual_exclusion(num_threads: usize, num_iterations: usize) {
        let lock = Arc::new(CLHLock::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..num_threads {
            let lock_clone = Arc::clone(&lock);
            let counter_clone = Arc::clone(&counter);
            
            let handle = thread::spawn(move || {
                for _ in 0..num_iterations {
                    let token = lock_clone.lock();
                    // Critical section
                    let current = counter_clone.load(Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(1)); // Simulate some work
                    counter_clone.store(current + 1, Ordering::Relaxed);
                    lock_clone.unlock(token);
                }
            });
            
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::Relaxed), num_threads * num_iterations);
    }

    // Test for fairness (FIFO ordering)
    fn test_fairness(num_threads: usize) {
        let lock = Arc::new(CLHLock::new());
        let execution_order = Arc::new(Mutex::new(VecDeque::new()));
        let mut handles = vec![];

        for i in 0..num_threads {
            let lock_clone = Arc::clone(&lock);
            let execution_order_clone = Arc::clone(&execution_order);
            
            let handle = thread::spawn(move || {
                let token = lock_clone.lock();
                // Record the order of execution
                execution_order_clone.lock().unwrap().push_back(i);
                thread::sleep(Duration::from_millis(10)); // Simulate some work
                lock_clone.unlock(token);
            });
            
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify that threads executed in roughly the order they requested the lock
        let order = execution_order.lock().unwrap();
        let mut out_of_order_count = 0;
        for i in 0..order.len() - 1 {
            if order[i] > order[i + 1] {
                out_of_order_count += 1;
            }
        }
        // Allow some small number of out-of-order executions due to scheduling
        assert!(out_of_order_count < num_threads / 2);
    }

    // Stress test with different operations
    fn stress_test(num_threads: usize, num_iterations: usize) {
        let lock = Arc::new(CLHLock::new());
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..num_threads {
            let lock_clone = Arc::clone(&lock);
            let counter_clone = Arc::clone(&counter);
            
            let handle = thread::spawn(move || {
                for _ in 0..num_iterations {
                    let token = lock_clone.lock();
                    // Perform different operations in critical section
                    match counter_clone.load(Ordering::Relaxed) % 3 {
                        0 => counter_clone.fetch_add(1, Ordering::Relaxed),
                        1 => counter_clone.fetch_sub(1, Ordering::Relaxed),
                        _ => counter_clone.fetch_add(2, Ordering::Relaxed),
                    };
                    thread::sleep(Duration::from_micros(100)); // Simulate some work
                    lock_clone.unlock(token);
                }
            });
            
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_basic_mutual_exclusion() {
        test_mutual_exclusion(4, 1000);
    }

    #[test]
    fn test_fairness_small() {
        test_fairness(4);
    }

    #[test]
    fn test_fairness_large() {
        test_fairness(8);
    }

    #[test]
    fn test_stress_small() {
        stress_test(4, 1000);
    }

    #[test]
    fn test_stress_large() {
        stress_test(8, 5000);
    }

    #[test]
    fn test_concurrent_operations() {
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
                    // Perform multiple operations in critical section
                    let current = counter_clone.load(Ordering::Relaxed);
                    thread::sleep(Duration::from_micros(50));
                    counter_clone.store(current + 1, Ordering::Relaxed);
                    thread::sleep(Duration::from_micros(50));
                    let new_value = counter_clone.load(Ordering::Relaxed);
                    assert_eq!(new_value, current + 1);
                    lock_clone.unlock(token);
                }
            });
            
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::Relaxed), num_threads * num_iterations);
    }
}
