//! `CS2METAL_THREAD_TRACE=1`: which of the process's threads are busy, by name.
//!
//! A sample of the process from outside names what its threads run, but it stops
//! every thread to walk its stack, and it is a second process looking into this
//! one. This trace is the process reading its own threads' processor time from
//! the kernel, every ten seconds, which disturbs nothing and can stay on through
//! a whole match. A thread is named as the game named it
//! ([`crate::threads::prctl`]); the host's own threads and macOS's carry their own
//! names.
//!
//! Each period is one line: the busiest threads with their share of one
//! processor, the rest summed, and the processors busy in all.

use std::collections::HashMap;
use std::time::Duration;

/// How often the trace reports.
const PERIOD: Duration = Duration::from_secs(10);
/// How many threads a line names, and the least share of a processor named.
const NAMED: usize = 12;
const LEAST_SHARE: f64 = 0.01;

/// Mach's `thread_info` flavors, and room for the largest answer of the three,
/// `thread_extended_info`: 28 words.
const THREAD_BASIC_INFO: u32 = 3;
const THREAD_IDENTIFIER_INFO: u32 = 4;
const THREAD_EXTENDED_INFO: u32 = 5;
const INFO_WORDS: usize = 32;
/// Where `thread_extended_info` holds the name, and its length in words.
const NAME_AT: usize = 12;
const NAME_WORDS: usize = 16;

unsafe extern "C" {
    static mach_task_self_: u32;
    fn task_threads(task: u32, threads: *mut *mut u32, count: *mut u32) -> i32;
    fn thread_info(thread: u32, flavor: u32, info: *mut i32, count: *mut u32) -> i32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
    fn vm_deallocate(task: u32, address: usize, size: usize) -> i32;
}

/// One thread of this process: the kernel's number for it, its name, and the
/// processor time it has used since it started.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Thread {
    id: u64,
    name: String,
    used: Duration,
}

/// One flavor of `thread_info` for `thread`, or `None` if the thread is gone.
fn info(thread: u32, flavor: u32) -> Option<[i32; INFO_WORDS]> {
    let mut words = [0i32; INFO_WORDS];
    let mut count = INFO_WORDS as u32;
    // SAFETY: the buffer holds `count` words, more than any flavor asked here.
    let status = unsafe { thread_info(thread, flavor, words.as_mut_ptr(), &mut count) };
    (status == 0).then_some(words)
}

/// Every thread of this process, as the kernel reports them now.
pub(crate) fn snapshot() -> Vec<Thread> {
    let mut list: *mut u32 = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: task_threads fills both with an array it allocates in this task.
    let task = unsafe { mach_task_self_ };
    // SAFETY: as above.
    if unsafe { task_threads(task, &mut list, &mut count) } != 0 || list.is_null() {
        return Vec::new();
    }
    let mut threads = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        // SAFETY: `list` holds `count` thread ports.
        let port = unsafe { *list.add(i) };
        if let (Some(basic), Some(identity)) = (
            info(port, THREAD_BASIC_INFO),
            info(port, THREAD_IDENTIFIER_INFO),
        ) {
            // Two `time_value_t`, seconds and microseconds: user, then system.
            let micros = |at: usize| {
                u64::try_from(basic[at]).unwrap_or(0) * 1_000_000
                    + u64::try_from(basic[at + 1]).unwrap_or(0)
            };
            let id = u64::from(identity[0] as u32) | (u64::from(identity[1] as u32) << 32);
            let name = info(port, THREAD_EXTENDED_INFO).map_or_else(String::new, |extended| {
                let bytes: Vec<u8> = extended[NAME_AT..NAME_AT + NAME_WORDS]
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .take_while(|&byte| byte != 0)
                    .collect();
                String::from_utf8_lossy(&bytes).into_owned()
            });
            threads.push(Thread {
                id,
                name,
                used: Duration::from_micros(micros(0) + micros(2)),
            });
        }
        // SAFETY: task_threads gave this task a right to each port.
        unsafe { mach_port_deallocate(task, port) };
    }
    // SAFETY: the array task_threads allocated, `count` ports long.
    unsafe { vm_deallocate(task, list as usize, count as usize * size_of::<u32>()) };
    threads
}

/// The trace's line for a period of `elapsed`: what each of `now`'s threads used
/// beyond `before`, the busiest first.
fn line(before: &HashMap<u64, Duration>, now: &[Thread], elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64().max(0.001);
    let mut shares: Vec<(f64, &Thread)> = now
        .iter()
        .map(|thread| {
            let earlier = before.get(&thread.id).copied().unwrap_or_default();
            (
                thread.used.saturating_sub(earlier).as_secs_f64() / seconds,
                thread,
            )
        })
        .collect();
    shares.sort_by(|a, b| b.0.total_cmp(&a.0));
    let all: f64 = shares.iter().map(|(share, _)| share).sum();
    let named: Vec<String> = shares
        .iter()
        .take(NAMED)
        .filter(|(share, _)| *share >= LEAST_SHARE)
        .map(|(share, thread)| {
            if thread.name.is_empty() {
                format!("thread {} {:.0}%", thread.id, share * 100.0)
            } else {
                format!("{} {:.0}%", thread.name, share * 100.0)
            }
        })
        .collect();
    let rest: f64 = shares
        .iter()
        .skip(named.len())
        .map(|(share, _)| share)
        .sum();
    format!(
        "cs2metal: threads over {seconds:.1} s, of one processor: {}; {} others {:.0}%; \
         {all:.2} processors in all",
        named.join(", "),
        shares.len() - named.len(),
        rest * 100.0
    )
}

fn used(threads: &[Thread]) -> HashMap<u64, Duration> {
    threads.iter().map(|t| (t.id, t.used)).collect()
}

/// Starts the trace if `CS2METAL_THREAD_TRACE` is set.
pub(crate) fn start() {
    if std::env::var_os("CS2METAL_THREAD_TRACE").is_none() {
        return;
    }
    let reporter = std::thread::Builder::new().name("cs2metal thread trace".to_owned());
    let started = reporter.spawn(|| {
        let mut before = used(&snapshot());
        let mut at = std::time::Instant::now();
        loop {
            std::thread::sleep(PERIOD);
            let now = snapshot();
            let elapsed = at.elapsed();
            at = std::time::Instant::now();
            eprintln!("{}", line(&before, &now, elapsed));
            before = used(&now);
        }
    });
    if started.is_err() {
        eprintln!("cs2metal: the thread trace could not start its thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(id: u64, name: &str, millis: u64) -> Thread {
        Thread {
            id,
            name: name.to_owned(),
            used: Duration::from_millis(millis),
        }
    }

    #[test]
    fn a_line_names_the_busiest_threads_and_sums_the_rest() {
        let before = HashMap::from([
            (1, Duration::from_millis(1_000)),
            (2, Duration::from_millis(500)),
        ]);
        let now = [
            thread(1, "main", 8_200),
            thread(2, "", 9_700),
            thread(3, "idle", 20),
            // Started in the period: all its time counts.
            thread(4, "new", 2_500),
        ];
        assert_eq!(
            line(&before, &now, Duration::from_secs(10)),
            "cs2metal: threads over 10.0 s, of one processor: thread 2 92%, main 72%, \
             new 25%; 1 others 0%; 1.89 processors in all"
        );
    }

    #[test]
    fn a_thread_that_lost_time_counts_as_idle() {
        let before = HashMap::from([(1, Duration::from_secs(5))]);
        let text = line(&before, &[thread(1, "a", 4_000)], Duration::from_secs(10));
        assert!(text.contains(": ; 1 others 0%; 0.00 processors"), "{text}");
    }

    /// The kernel reports this process's threads with the names given them and
    /// the processor time they use.
    #[test]
    fn a_snapshot_has_a_named_thread_and_its_time() {
        let (ready, wait) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            crate::threads::name_this_thread(b"trace-test-busy");
            ready.send(()).expect("ready");
            let mut sum = 0u64;
            while stopped.try_recv().is_err() {
                for i in 0..100_000u64 {
                    sum = std::hint::black_box(sum.wrapping_add(i));
                }
            }
        });
        wait.recv().expect("named");
        let find = || {
            snapshot()
                .into_iter()
                .find(|t| t.name == "trace-test-busy")
                .expect("the worker is listed")
        };
        let first = find();
        std::thread::sleep(Duration::from_millis(60));
        let second = find();
        stop.send(()).expect("stop");
        worker.join().expect("worker");
        assert_eq!(first.id, second.id);
        let spent = second.used.saturating_sub(first.used);
        assert!(
            spent >= Duration::from_millis(20) && spent <= Duration::from_millis(200),
            "a busy thread used {spent:?} in 60 ms"
        );
    }
}
