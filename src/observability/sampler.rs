use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Lightweight RSS / CPU sampler for the current process.
///
/// Spawns a background thread that polls `sysinfo` every `interval` and
/// publishes the latest reading via atomics. `stop()` joins the thread
/// and returns a final summary.
pub struct ResourceSampler {
    stop: Arc<AtomicBool>,
    peak_rss_kb: Arc<AtomicU64>,
    last_rss_kb: Arc<AtomicU64>,
    cpu_sum_milli: Arc<AtomicU64>,
    cpu_samples: Arc<AtomicU64>,
    handle: Option<thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceSummary {
    pub rss_mb: f64,
    pub peak_rss_mb: f64,
    pub avg_cpu_pct: f64,
}

impl ResourceSampler {
    pub fn start(interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak_rss_kb = Arc::new(AtomicU64::new(0));
        let last_rss_kb = Arc::new(AtomicU64::new(0));
        let cpu_sum_milli = Arc::new(AtomicU64::new(0));
        let cpu_samples = Arc::new(AtomicU64::new(0));

        let stop_c = Arc::clone(&stop);
        let peak_c = Arc::clone(&peak_rss_kb);
        let last_c = Arc::clone(&last_rss_kb);
        let cpu_sum_c = Arc::clone(&cpu_sum_milli);
        let cpu_n_c = Arc::clone(&cpu_samples);

        let handle = thread::Builder::new()
            .name("virgil-resource-sampler".into())
            .spawn(move || {
                let pid = Pid::from_u32(std::process::id());
                let mut sys = System::new();
                let refresh = ProcessRefreshKind::new().with_memory().with_cpu();
                while !stop_c.load(Ordering::Relaxed) {
                    sys.refresh_processes_specifics(
                        ProcessesToUpdate::Some(&[pid]),
                        true,
                        refresh,
                    );
                    if let Some(proc_) = sys.process(pid) {
                        let rss_kb = proc_.memory() / 1024;
                        last_c.store(rss_kb, Ordering::Relaxed);
                        peak_c.fetch_max(rss_kb, Ordering::Relaxed);
                        let cpu = proc_.cpu_usage();
                        cpu_sum_c.fetch_add((cpu * 1000.0) as u64, Ordering::Relaxed);
                        cpu_n_c.fetch_add(1, Ordering::Relaxed);
                    }
                    thread::sleep(interval);
                }
            })
            .expect("spawn resource sampler thread");

        Self {
            stop,
            peak_rss_kb,
            last_rss_kb,
            cpu_sum_milli,
            cpu_samples,
            handle: Some(handle),
        }
    }

    pub fn snapshot(&self) -> ResourceSummary {
        let last = self.last_rss_kb.load(Ordering::Relaxed);
        let peak = self.peak_rss_kb.load(Ordering::Relaxed);
        let cpu_sum = self.cpu_sum_milli.load(Ordering::Relaxed);
        let cpu_n = self.cpu_samples.load(Ordering::Relaxed).max(1);
        ResourceSummary {
            rss_mb: kb_to_mb(last),
            peak_rss_mb: kb_to_mb(peak),
            avg_cpu_pct: (cpu_sum as f64) / (cpu_n as f64) / 1000.0,
        }
    }

    pub fn stop(mut self) -> ResourceSummary {
        let summary = self.snapshot();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        summary
    }
}

impl Drop for ResourceSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn kb_to_mb(kb: u64) -> f64 {
    (kb as f64) / 1024.0
}
