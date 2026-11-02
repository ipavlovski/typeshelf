//! A small priority worker pool for rendering. Visible rows are submitted with
//! higher priority; rows scrolled away cancel their jobs before they start.

use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

type Work = Box<dyn FnOnce() + Send>;

/// Set to cancel every job submitted with it that has not started yet.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

struct Job {
    priority: i64,
    /// Submission order, so equal priorities run first-in first-out.
    seq: u64,
    cancel: CancelToken,
    work: Work,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        (self.priority, self.seq) == (other.priority, other.seq)
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&other.priority).then(other.seq.cmp(&self.seq))
    }
}

#[derive(Default)]
struct Queue {
    jobs: BinaryHeap<Job>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
    seq: AtomicU64,
}

pub struct Pool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    pub fn new(threads: usize) -> Pool {
        let shared =
            Arc::new(Shared { queue: Mutex::new(Queue::default()), ready: Condvar::new(), seq: AtomicU64::new(0) });
        let workers = (0..threads.max(1))
            .map(|i| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("render-{i}"))
                    .spawn(move || worker(&shared))
                    .expect("spawning render thread")
            })
            .collect();
        Pool { shared, workers }
    }

    /// Queues `work`; higher `priority` runs first.
    pub fn submit(&self, priority: i64, cancel: &CancelToken, work: impl FnOnce() + Send + 'static) {
        let seq = self.shared.seq.fetch_add(1, Ordering::Relaxed);
        let job = Job { priority, seq, cancel: cancel.clone(), work: Box::new(work) };
        self.shared.queue.lock().unwrap().jobs.push(job);
        self.shared.ready.notify_one();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().closed = true;
        self.shared.ready.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker(shared: &Shared) {
    loop {
        let job = {
            let mut q = shared.queue.lock().unwrap();
            loop {
                if q.closed {
                    return;
                }
                if let Some(job) = q.jobs.pop() {
                    break job;
                }
                q = shared.ready.wait(q).unwrap();
            }
        };
        if !job.cancel.is_cancelled() {
            (job.work)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn runs_by_priority_and_skips_cancelled() {
        let pool = Pool::new(1);
        let (tx, rx) = mpsc::channel();
        // Block the only worker so the rest queue up.
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        pool.submit(100, &CancelToken::default(), move || gate_rx.recv().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));

        let stale = CancelToken::default();
        for (p, name, token) in
            [(1, "low", CancelToken::default()), (5, "high", CancelToken::default()), (9, "stale", stale.clone())]
        {
            let tx = tx.clone();
            pool.submit(p, &token, move || tx.send(name).unwrap());
        }
        stale.cancel();
        gate_tx.send(()).unwrap();
        drop(tx);
        let order: Vec<_> = rx.iter().take(2).collect();
        assert_eq!(order, ["high", "low"]);
    }
}
