//! Synthesized Linux kernel files: the `/proc` and `/sys` entries the engine reads.
//!
//! macOS has neither tree. When the guest reads one of the files below, the host
//! writes its contents, as the Linux kernel would report them for this process
//! and machine, into a private directory, and the guest reads that file. Each read
//! writes the file anew, so the values are current. Anything else under `/proc`
//! or `/sys` does not exist, as on a kernel without it.
//!
//! The processor's frequency is the rate of its time stamp counter as Rosetta 2
//! presents it, 1,000 MHz, so that time measured with `rdtsc` and that frequency
//! agrees with the clock.

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::Mutex;

/// The guest's command line, NUL-separated, as `/proc/self/cmdline` reports it.
pub(crate) static CMDLINE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// The rate of the time stamp counter under Rosetta 2, in kHz.
const TSC_KHZ: u64 = 1_000_000;

fn root() -> PathBuf {
    std::env::temp_dir().join(format!("cs2metal-{}-kernel", std::process::id()))
}

fn sysctl_string(name: &std::ffi::CStr) -> Option<String> {
    let mut buf = vec![0u8; 1024];
    let mut size = buf.len();
    // SAFETY: sysctlbyname writes at most `size` bytes.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return None;
    }
    buf.truncate(size);
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

fn sysctl_u32(name: &std::ffi::CStr) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>();
    // SAFETY: sysctlbyname writes a u32.
    let status = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::from_mut(&mut value).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0).then_some(value)
}

fn cpus() -> u32 {
    sysctl_u32(c"hw.logicalcpu").unwrap_or(1).max(1)
}

/// Linux's names for the CPUID features macOS reports.
fn linux_flags(features: &str) -> String {
    features
        .split_whitespace()
        .filter_map(|f| {
            Some(match f {
                "SSE3" => "pni".to_owned(),
                "SSE4.1" => "sse4_1".to_owned(),
                "SSE4.2" => "sse4_2".to_owned(),
                "CLFSH" => "clflush".to_owned(),
                "MON" => "monitor".to_owned(),
                "PCLMULQDQ" => "pclmulqdq".to_owned(),
                "DSCPL" => "ds_cpl".to_owned(),
                "DTSE64" => "dtes64".to_owned(),
                "SS" => "ss".to_owned(),
                "HTT" => "ht".to_owned(),
                "TM" => "tm".to_owned(),
                "SEGLIM64" | "VMX" => return None,
                other => other.to_ascii_lowercase().replace('.', "_"),
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn cpuinfo() -> String {
    let model = sysctl_string(c"machdep.cpu.brand_string").unwrap_or_default();
    let mut flags = linux_flags(&sysctl_string(c"machdep.cpu.features").unwrap_or_default());
    if let Some(leaf7) = sysctl_string(c"machdep.cpu.leaf7_features") {
        flags.push(' ');
        flags.push_str(&linux_flags(&leaf7));
    }
    let count = cpus();
    let mut out = String::new();
    for n in 0..count {
        out.push_str(&format!(
            "processor\t: {n}\nvendor_id\t: GenuineIntel\ncpu family\t: 6\nmodel\t\t: 0\n\
             model name\t: {model}\nstepping\t: 0\ncpu MHz\t\t: {:.3}\ncache size\t: 0 KB\n\
             physical id\t: 0\nsiblings\t: {count}\ncore id\t\t: {n}\ncpu cores\t: {count}\n\
             apicid\t\t: {n}\nfpu\t\t: yes\nfpu_exception\t: yes\ncpuid level\t: 13\n\
             wp\t\t: yes\nflags\t\t: {flags}\nbogomips\t: {:.2}\nclflush size\t: 64\n\
             cache_alignment\t: 64\naddress sizes\t: 47 bits physical, 47 bits virtual\n\
             power management:\n\n",
            TSC_KHZ as f64 / 1000.0,
            2.0 * TSC_KHZ as f64 / 1000.0,
        ));
    }
    out
}

/// This process's virtual and resident sizes and thread count.
fn task_sizes() -> (u64, u64, u64) {
    // SAFETY: proc_taskinfo is plain data; proc_pidinfo fills it.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    // SAFETY: PROC_PIDTASKINFO writes a proc_taskinfo for this process.
    let got = unsafe {
        libc::proc_pidinfo(
            std::process::id() as i32,
            libc::PROC_PIDTASKINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    if got != size {
        return (0, 0, 1);
    }
    (
        info.pti_virtual_size,
        info.pti_resident_size,
        u64::try_from(info.pti_threadnum).unwrap_or(1),
    )
}

fn status() -> String {
    let (virtual_size, resident, threads) = task_sizes();
    // SAFETY: plain process queries.
    let (pid, ppid, uid, gid) = unsafe {
        (
            libc::getpid(),
            libc::getppid(),
            libc::getuid(),
            libc::getgid(),
        )
    };
    format!(
        "Name:\tcs2\nUmask:\t0022\nState:\tR (running)\nTgid:\t{pid}\nNgid:\t0\nPid:\t{pid}\n\
         PPid:\t{ppid}\nTracerPid:\t0\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n\
         Gid:\t{gid}\t{gid}\t{gid}\t{gid}\nFDSize:\t256\nVmPeak:\t{v} kB\nVmSize:\t{v} kB\n\
         VmLck:\t0 kB\nVmPin:\t0 kB\nVmHWM:\t{r} kB\nVmRSS:\t{r} kB\nThreads:\t{threads}\n",
        v = virtual_size / 1024,
        r = resident / 1024,
    )
}

fn statm() -> String {
    let (virtual_size, resident, _) = task_sizes();
    format!(
        "{} {} 0 0 0 {} 0\n",
        virtual_size / 4096,
        resident / 4096,
        virtual_size / 4096
    )
}

/// What a synthesized path is.
enum Entry {
    File(Vec<u8>),
    Directory,
    /// Another host path, such as the guest's executable.
    Elsewhere(Vec<u8>),
}

/// The entry for a guest path under `/proc` or `/sys`, if the kernel would have
/// it.
fn entry(path: &str) -> Option<Entry> {
    let pid = std::process::id().to_string();
    let file = |s: String| Some(Entry::File(s.into_bytes()));
    if let Some(rest) = path.strip_prefix("/proc/") {
        let rest = match rest.split_once('/') {
            Some((who, tail)) if who == "self" || who == "thread-self" || who == pid => {
                format!("self/{tail}")
            }
            Some((who, _)) if who.bytes().all(|b| b.is_ascii_digit()) => return None,
            _ => rest.to_owned(),
        };
        return match rest.as_str() {
            "cpuinfo" => file(cpuinfo()),
            "version" => {
                file("Linux version 5.15.0-cs2metal (cs2metal) (gcc) #1 SMP PREEMPT\n".to_owned())
            }
            "sys/vm/overcommit_memory" => file("0\n".to_owned()),
            "self/status" => file(status()),
            "self/statm" => file(statm()),
            "self/cmdline" => Some(Entry::File(
                CMDLINE.lock().map(|c| c.clone()).unwrap_or_default(),
            )),
            "self/task" => Some(Entry::Directory),
            "self/exe" => crate::syscall::EXE
                .lock()
                .ok()
                .map(|e| Entry::Elsewhere(e.clone().into_bytes())),
            _ => None,
        };
    }
    let rest = path.strip_prefix("/sys/devices/system/")?;
    let count = cpus();
    match rest {
        "clocksource/clocksource0/current_clocksource" => return file("tsc\n".to_owned()),
        "cpu/online" | "cpu/possible" | "cpu/present" => {
            return file(format!("0-{}\n", count - 1));
        }
        "node/node0" => return Some(Entry::Directory),
        _ => {}
    }
    let tail = rest.strip_prefix("cpu/cpu")?;
    let (n, leaf) = tail.split_once('/')?;
    let n: u32 = n.parse().ok()?;
    if n >= count {
        return None;
    }
    match leaf {
        "cpufreq/cpuinfo_max_freq" | "cpufreq/scaling_max_freq" | "cpufreq/scaling_cur_freq" => {
            file(format!("{TSC_KHZ}\n"))
        }
        "topology/core_id" => file(format!("{n}\n")),
        "topology/physical_package_id" => file("0\n".to_owned()),
        _ => None,
    }
}

/// The host path standing for a guest path under `/proc` or `/sys`, written
/// now; `None` for any other path, and for entries the kernel would not have.
pub(crate) fn synthesize(path: &[u8]) -> Option<CString> {
    if !(path.starts_with(b"/proc/") || path.starts_with(b"/sys/")) {
        return None;
    }
    let path = std::str::from_utf8(path).ok()?.trim_end_matches('/');
    let entry = entry(path)?;
    let host = root().join(path.trim_start_matches('/'));
    match entry {
        Entry::Elsewhere(p) => return CString::new(p).ok(),
        Entry::Directory => {
            std::fs::create_dir_all(&host).ok()?;
        }
        Entry::File(contents) => {
            std::fs::create_dir_all(host.parent()?).ok()?;
            // Written beside and renamed, so a concurrent reader sees a whole file.
            let partial = host.with_extension(format!("partial-{:?}", std::thread::current().id()));
            std::fs::write(&partial, contents).ok()?;
            std::fs::rename(&partial, &host).ok()?;
        }
    }
    CString::new(host.to_string_lossy().as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str) -> String {
        let host = synthesize(path.as_bytes()).unwrap_or_else(|| panic!("{path}"));
        std::fs::read_to_string(host.to_str().expect("utf-8")).expect("read")
    }

    #[test]
    fn the_engine_reads_a_frequency_and_the_topology() {
        let cpuinfo = read("/proc/cpuinfo");
        assert!(cpuinfo.contains("cpu MHz\t\t: 1000.000"));
        assert_eq!(cpuinfo.matches("processor\t:").count() as u32, cpus());
        // An x86-64 process under Rosetta 2 sees x86 features; a native one none.
        if cfg!(target_arch = "x86_64") {
            assert!(cpuinfo.contains(" sse4_2"), "{cpuinfo}");
        }
        assert_eq!(
            read("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"),
            "1000000\n"
        );
        assert_eq!(read("/sys/devices/system/cpu/cpu1/topology/core_id"), "1\n");
        let beyond = format!("/sys/devices/system/cpu/cpu{}/topology/core_id", cpus());
        assert!(synthesize(beyond.as_bytes()).is_none(), "no such processor");
    }

    #[test]
    fn process_files_describe_this_process() {
        let status = read("/proc/self/status");
        assert!(status.contains(&format!("Pid:\t{}\n", std::process::id())));
        let by_pid = read(&format!("/proc/{}/statm", std::process::id()));
        assert_eq!(by_pid.split_whitespace().count(), 7);
        assert!(synthesize(b"/proc/1/status").is_none(), "another process");
        assert!(synthesize(b"/proc/kallsyms").is_none());
        assert!(synthesize(b"/usr/lib/libc.so.6").is_none());
        let node = synthesize(b"/sys/devices/system/node/node0").expect("node");
        assert!(std::path::Path::new(node.to_str().expect("utf-8")).is_dir());
    }
}
