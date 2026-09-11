//! Bounded latest-request worker. The owner never waits for CPU preparation.
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

struct Mailbox<I, O> {
    pending: Option<(u64, I)>,
    ready: Option<(u64, O)>,
}

struct Shared<I, O> {
    mailbox: Mutex<Mailbox<I, O>>,
    wake: Condvar,
    generation: AtomicU64,
    stopped: AtomicBool,
    closed: Arc<AtomicBool>,
}

/// Cooperative cancellation between expensive preparation phases. A running
/// platform call may finish, but its output is never delivered after invalidation.
pub(super) struct Cancellation {
    generation: u64,
    current: Box<dyn Fn() -> bool + Send>,
}

impl Cancellation {
    pub(super) fn is_cancelled(&self) -> bool {
        !(self.current)()
    }
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }
}

pub(super) struct LatestWorker<I, O> {
    shared: Arc<Shared<I, O>>,
}

impl<I: Send + 'static, O: Send + 'static> LatestWorker<I, O> {
    pub(super) fn new(
        mut build: impl FnMut(I, &Cancellation) -> O + Send + 'static,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            mailbox: Mutex::new(Mailbox {
                pending: None,
                ready: None,
            }),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            closed: Arc::new(AtomicBool::new(false)),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("yu-frame-build".into())
            .spawn(move || {
                struct Closed(Arc<AtomicBool>);
                impl Drop for Closed {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _closed = Closed(Arc::clone(&worker.closed));
                loop {
                    let (generation, input) = {
                        let Ok(mut mailbox) = worker.mailbox.lock() else {
                            return;
                        };
                        while mailbox.pending.is_none() && !worker.stopped.load(Ordering::Acquire) {
                            let Ok(next) = worker.wake.wait(mailbox) else {
                                return;
                            };
                            mailbox = next;
                        }
                        if worker.stopped.load(Ordering::Acquire) {
                            return;
                        }
                        mailbox.pending.take().expect("pending job")
                    };
                    let current = Arc::clone(&worker);
                    let cancellation = Cancellation {
                        generation,
                        current: Box::new(move || {
                            !current.stopped.load(Ordering::Acquire)
                                && current.generation.load(Ordering::Acquire) == generation
                        }),
                    };
                    if cancellation.is_cancelled() {
                        continue;
                    }
                    let output = build(input, &cancellation);
                    let previous = {
                        let Ok(mut mailbox) = worker.mailbox.lock() else {
                            return;
                        };
                        if cancellation.is_cancelled() {
                            drop(mailbox);
                            if std::env::var_os("YU_RENDER_TIMING").is_some() {
                                println!("yu-render-metric event=stale_publication");
                            }
                            continue;
                        }
                        mailbox.ready.replace((generation, output))
                    };
                    drop(previous);
                }
            })?;
        Ok(Self { shared })
    }

    pub(super) fn submit(&self, generation: u64, input: I) -> Result<(), ()> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(());
        }
        let (pending, ready) = {
            let mut mailbox = self.shared.mailbox.lock().map_err(|_| ())?;
            self.shared.generation.store(generation, Ordering::Release);
            (
                mailbox.pending.replace((generation, input)),
                mailbox.ready.take(),
            )
        };
        self.shared.wake.notify_one();
        drop((pending, ready));
        Ok(())
    }

    pub(super) fn try_take(&self) -> Result<Option<(u64, O)>, ()> {
        if self.shared.closed.load(Ordering::Acquire) {
            return Err(());
        }
        Ok(self.shared.mailbox.lock().map_err(|_| ())?.ready.take())
    }

    /// Also used for detach/rebind: retain the thread, invalidate its work.
    pub(super) fn invalidate(&self) {
        self.shared.generation.store(0, Ordering::Release);
        if let Ok(mut mailbox) = self.shared.mailbox.lock() {
            let old = (mailbox.pending.take(), mailbox.ready.take());
            drop(mailbox);
            drop(old);
        }
    }
}

impl<I, O> Drop for LatestWorker<I, O> {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.generation.store(0, Ordering::Release);
        if let Ok(mut mailbox) = self.shared.mailbox.lock() {
            let old = (mailbox.pending.take(), mailbox.ready.take());
            drop(mailbox);
            drop(old);
        }
        // Notify after synchronizing with the wait mutex, avoiding a lost
        // wake if the worker was about to enter Condvar::wait.
        self.shared.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn blocked_build_keeps_only_latest_request_on_one_thread() {
        let (entered, entered_rx) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        let worker = LatestWorker::new(move |job, cancel| {
            entered.send((job, std::thread::current().id())).unwrap();
            if job == 1 {
                resume_rx.recv().unwrap();
                assert!(cancel.is_cancelled());
            }
            job
        })
        .unwrap();
        worker.submit(1, 1).unwrap();
        let (_, thread) = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        for job in 2..=1000 {
            worker.submit(job, job).unwrap();
        }
        assert!(entered_rx.try_recv().is_err(), "no parallel builds");
        resume.send(()).unwrap();
        let (job, next_thread) = entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(job, 1000);
        assert_eq!(thread, next_thread);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some((generation, result)) = worker.try_take().unwrap() {
                assert_eq!((generation, result), (1000, 1000));
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(entered_rx.try_recv().is_err());
    }

    #[test]
    fn detach_discards_running_and_ready_work_without_joining() {
        let (entered, entered_rx) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        let worker = LatestWorker::new(move |job, cancel| {
            entered.send(job).unwrap();
            if job == 1 {
                resume_rx.recv().unwrap();
                assert!(cancel.is_cancelled());
            }
            job
        })
        .unwrap();
        worker.submit(1, 1).unwrap();
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
        worker.submit(2, 2).unwrap();
        worker.invalidate();
        worker.submit(3, 3).unwrap(); // a new binding uses a new ticket
        resume.send(()).unwrap();
        assert_eq!(entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 3);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        // Wait for a delivered result without consuming it: detach must clear
        // the ready slot as well as the queued/running old-binding work.
        while worker.shared.mailbox.lock().unwrap().ready.is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        worker.invalidate();
        assert!(worker.try_take().unwrap().is_none());
        assert!(entered_rx.try_recv().is_err());
    }

    #[test]
    fn dropping_owner_does_not_wait_for_blocked_build() {
        let (entered, entered_rx) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        let (finished, finished_rx) = mpsc::channel();
        let worker = LatestWorker::new(move |(), cancel| {
            entered.send(()).unwrap();
            resume_rx.recv().unwrap();
            assert!(cancel.is_cancelled());
            finished.send(()).unwrap();
        })
        .unwrap();
        worker.submit(1, ()).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (dropped, dropped_rx) = mpsc::channel();
        std::thread::spawn(move || {
            drop(worker);
            dropped.send(()).unwrap();
        });
        let result = dropped_rx.recv_timeout(Duration::from_secs(2));
        resume.send(()).unwrap();
        assert!(result.is_ok(), "owner waited for preparation");
        finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
