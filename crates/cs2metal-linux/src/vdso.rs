//! The vDSO: the time functions the guest's C library calls without a system call.
//!
//! On Linux the kernel maps a small shared object into every process and names it
//! in the auxiliary vector (`AT_SYSINFO_EHDR`). glibc looks up
//! `__vdso_clock_gettime`, `__vdso_gettimeofday`, and `__vdso_time` in it, version
//! `LINUX_2.6`, and calls them in place of the system calls. Without one, every
//! read of a clock is a system call, which under cs2metal costs two GS switches:
//! in a match the game's main thread spent an eighth of its processor time there
//! (docs/PERFORMANCE_AB_2026-11-27.md).
//!
//! The host makes that object itself, in memory: an ELF image of one page with
//! the three symbols, each a jump to a host function. Those functions are neutral
//! ([`crate::clock`]): they run in the guest domain, as the guest's own code does,
//! so the call crosses no bridge. Nothing of the guest's code or data is changed;
//! this is part of starting the process, as the kernel does it.
//!
//! A clock the host does not answer neutrally returns `-ENOSYS`, on which glibc
//! makes the system call. `CS2METAL_NO_VDSO=1` starts the guest without the
//! object, as before.

use crate::clock;

/// The auxiliary vector entry that names the vDSO.
pub(crate) const AT_SYSINFO_EHDR: u64 = 33;

const PAGE: usize = 4096;
const SONAME: &str = "linux-vdso.so.1";
const VERSION: &str = "LINUX_2.6";
const ENOSYS: i64 = -38;

/// `__vdso_clock_gettime(clock, timespec)`. Neutral.
extern "C" fn clock_gettime(id: i32, out: u64) -> i64 {
    match clock::now_ns(u64::from(id as u32)) {
        Some(ns) if out != 0 => {
            // SAFETY: the guest passes a struct timespec.
            unsafe { clock::write_timespec(out, ns) };
            0
        }
        // The processor-time clocks, and a null pointer: the system call answers.
        _ => ENOSYS,
    }
}

/// `__vdso_gettimeofday(timeval, timezone)`. Neutral.
extern "C" fn gettimeofday(tv: u64, tz: u64) -> i64 {
    let ns = clock::now_ns(clock::REALTIME).unwrap_or(0);
    if tv != 0 {
        // SAFETY: the guest passes a Linux struct timeval, two 64-bit words.
        unsafe {
            std::ptr::write_unaligned(
                tv as *mut [i64; 2],
                [
                    (ns / clock::NANOS) as i64,
                    (ns % clock::NANOS / 1000) as i64,
                ],
            );
        }
    }
    if tz != 0 {
        // SAFETY: the guest passes a struct timezone, two ints; UTC, as Linux answers.
        unsafe { std::ptr::write_unaligned(tz as *mut [i32; 2], [0, 0]) };
    }
    0
}

/// `__vdso_time(time_t *)`. Neutral.
extern "C" fn time(out: u64) -> i64 {
    let seconds = (clock::now_ns(clock::REALTIME).unwrap_or(0) / clock::NANOS) as i64;
    if out != 0 {
        // SAFETY: the guest passes a time_t.
        unsafe { std::ptr::write_unaligned(out as *mut i64, seconds) };
    }
    seconds
}

/// The symbols of the object and the host functions they jump to.
fn entries() -> [(&'static str, u64); 3] {
    [
        ("__vdso_clock_gettime", clock_gettime as *const () as u64),
        ("__vdso_gettimeofday", gettimeofday as *const () as u64),
        ("__vdso_time", time as *const () as u64),
    ]
}

/// The ELF hash of the System V ABI, by which symbol versions are named.
fn elf_hash(name: &[u8]) -> u32 {
    let mut hash: u32 = 0;
    for &byte in name {
        hash = (hash << 4).wrapping_add(u32::from(byte));
        let high = hash & 0xf000_0000;
        if high != 0 {
            hash ^= high >> 24;
        }
        hash &= !high;
    }
    hash
}

fn put(image: &mut [u8], at: usize, bytes: &[u8]) {
    image[at..at + bytes.len()].copy_from_slice(bytes);
}

fn align(value: usize, to: usize) -> usize {
    value.div_ceil(to) * to
}

/// Where the pieces of an image for `count` symbols lie, as file offsets, which
/// are also its addresses: the one loadable segment starts at zero.
struct Layout {
    hash: usize,
    symbols: usize,
    versym: usize,
    verdef: usize,
    strings: usize,
    dynamic: usize,
    text: usize,
    size: usize,
}

const EHDR: usize = 64;
const PHDR: usize = 56;
const SYMBOL: usize = 24;
const VERDEF: usize = 28;
const DYNAMIC_ENTRIES: usize = 10;
/// `movabs rax, target; jmp rax`, padded with `int3`.
const STUB: usize = 16;

fn layout(count: usize, strings_len: usize) -> Layout {
    let hash = EHDR + 2 * PHDR;
    let symbols = align(hash + 4 * (3 + count + 1), 8);
    let versym = symbols + SYMBOL * (count + 1);
    let verdef = align(versym + 2 * (count + 1), 4);
    let strings = verdef + 2 * VERDEF;
    let dynamic = align(strings + strings_len, 8);
    let text = align(dynamic + 16 * DYNAMIC_ENTRIES, 16);
    Layout {
        hash,
        symbols,
        versym,
        verdef,
        strings,
        dynamic,
        text,
        size: align(text + STUB * count, PAGE),
    }
}

/// An ELF shared object that defines each name of `entries`, version
/// `LINUX_2.6`, as a jump to its address. It has what glibc reads of a vDSO: one
/// loadable segment at address zero, a dynamic segment, a System V hash table,
/// the symbols and their versions, and no relocations, since ld.so applies none
/// to an object the kernel is taken to have prepared.
fn image(entries: &[(&str, u64)]) -> Vec<u8> {
    let count = entries.len();
    let mut strings = vec![0u8];
    let mut name = |text: &str| -> u32 {
        let at = strings.len() as u32;
        strings.extend_from_slice(text.as_bytes());
        strings.push(0);
        at
    };
    let soname = name(SONAME);
    let version = name(VERSION);
    let names: Vec<u32> = entries.iter().map(|(text, _)| name(text)).collect();
    let at = layout(count, strings.len());
    let mut image = vec![0u8; at.size];
    let word = |value: usize| (value as u64).to_le_bytes();

    // The ELF header: a 64-bit little-endian shared object for x86-64.
    put(&mut image, 0, &[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    put(&mut image, 16, &3u16.to_le_bytes()); // ET_DYN
    put(&mut image, 18, &62u16.to_le_bytes()); // EM_X86_64
    put(&mut image, 20, &1u32.to_le_bytes()); // EV_CURRENT
    put(&mut image, 32, &word(EHDR)); // e_phoff
    put(&mut image, 52, &(EHDR as u16).to_le_bytes());
    put(&mut image, 54, &(PHDR as u16).to_le_bytes());
    put(&mut image, 56, &2u16.to_le_bytes()); // e_phnum

    // PT_LOAD, read and execute, the whole image at address zero; PT_DYNAMIC.
    let mut segment = |index: usize, kind: u32, flags: u32, offset: usize, len: usize, a: usize| {
        let base = EHDR + PHDR * index;
        put(&mut image, base, &kind.to_le_bytes());
        put(&mut image, base + 4, &flags.to_le_bytes());
        for field in [8, 16, 24] {
            put(&mut image, base + field, &word(offset));
        }
        put(&mut image, base + 32, &word(len));
        put(&mut image, base + 40, &word(len));
        put(&mut image, base + 48, &word(a));
    };
    segment(0, 1, 5, 0, at.size, PAGE);
    segment(1, 2, 4, at.dynamic, 16 * DYNAMIC_ENTRIES, 8);

    // The hash table: one bucket, so every symbol is on one chain, in order.
    let mut hash = vec![1u32, count as u32 + 1, u32::from(count > 0), 0];
    hash.extend((1..=count as u32).map(|i| if i < count as u32 { i + 1 } else { 0 }));
    for (i, value) in hash.iter().enumerate() {
        put(&mut image, at.hash + 4 * i, &value.to_le_bytes());
    }

    for (i, ((_, target), name)) in entries.iter().zip(&names).enumerate() {
        let stub = at.text + STUB * i;
        let symbol = at.symbols + SYMBOL * (i + 1);
        put(&mut image, symbol, &name.to_le_bytes());
        image[symbol + 4] = 0x12; // STB_GLOBAL, STT_FUNC
        put(&mut image, symbol + 6, &1u16.to_le_bytes()); // defined: any section
        put(&mut image, symbol + 8, &word(stub));
        put(&mut image, symbol + 16, &word(STUB));
        // Version index 2, the second definition below.
        put(&mut image, at.versym + 2 * (i + 1), &2u16.to_le_bytes());
        put(&mut image, stub, &[0x48, 0xb8]);
        put(&mut image, stub + 2, &target.to_le_bytes());
        put(&mut image, stub + 10, &[0xff, 0xe0, 0xcc, 0xcc, 0xcc, 0xcc]);
    }

    // Two version definitions: the object's own name, flagged as the base, and
    // LINUX_2.6. Each is a Verdef followed by its one Verdaux.
    for (i, (flags, text, text_at)) in [(1u16, SONAME, soname), (0, VERSION, version)]
        .into_iter()
        .enumerate()
    {
        let base = at.verdef + VERDEF * i;
        put(&mut image, base, &1u16.to_le_bytes()); // vd_version
        put(&mut image, base + 2, &flags.to_le_bytes());
        put(&mut image, base + 4, &(i as u16 + 1).to_le_bytes()); // vd_ndx
        put(&mut image, base + 6, &1u16.to_le_bytes()); // vd_cnt
        put(
            &mut image,
            base + 8,
            &elf_hash(text.as_bytes()).to_le_bytes(),
        );
        put(&mut image, base + 12, &20u32.to_le_bytes()); // vd_aux
        let next = if i == 0 { VERDEF as u32 } else { 0 };
        put(&mut image, base + 16, &next.to_le_bytes());
        put(&mut image, base + 20, &text_at.to_le_bytes()); // vda_name
    }

    put(&mut image, at.strings, &strings);
    let dynamic: [(u64, usize); DYNAMIC_ENTRIES] = [
        (4, at.hash),             // DT_HASH
        (5, at.strings),          // DT_STRTAB
        (6, at.symbols),          // DT_SYMTAB
        (10, strings.len()),      // DT_STRSZ
        (11, SYMBOL),             // DT_SYMENT
        (14, soname as usize),    // DT_SONAME
        (0x6fff_fff0, at.versym), // DT_VERSYM
        (0x6fff_fffc, at.verdef), // DT_VERDEF
        (0x6fff_fffd, 2),         // DT_VERDEFNUM
        (0, 0),                   // DT_NULL
    ];
    for (i, (tag, value)) in dynamic.iter().enumerate() {
        put(&mut image, at.dynamic + 16 * i, &tag.to_le_bytes());
        put(&mut image, at.dynamic + 16 * i + 8, &word(*value));
    }
    image
}

/// Maps `image` for reading and execution and returns its address; it stays
/// mapped for the life of the process.
fn map(image: &[u8]) -> Option<u64> {
    // SAFETY: a fresh anonymous mapping of the image's length.
    let memory = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            image.len(),
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if memory == libc::MAP_FAILED {
        return None;
    }
    // SAFETY: the mapping just made is image.len() bytes, writable until the
    // protection change, and nothing else knows its address yet.
    let status = unsafe {
        std::ptr::copy_nonoverlapping(image.as_ptr(), memory.cast::<u8>(), image.len());
        libc::mprotect(memory, image.len(), libc::PROT_READ | libc::PROT_EXEC)
    };
    (status == 0).then_some(memory as u64)
}

/// Makes the vDSO and returns the address to put in the auxiliary vector, or
/// `None` with `CS2METAL_NO_VDSO` set, or where it cannot be mapped: the guest
/// then makes a system call for each read of a clock.
pub(crate) fn start() -> Option<u64> {
    if std::env::var_os("CS2METAL_NO_VDSO").is_some() {
        return None;
    }
    clock::init();
    map(&image(&entries()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(image: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(image[at..at + 2].try_into().expect("2"))
    }

    fn u32_at(image: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(image[at..at + 4].try_into().expect("4"))
    }

    fn u64_at(image: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(image[at..at + 8].try_into().expect("8"))
    }

    fn c_str(image: &[u8], at: usize) -> &[u8] {
        let end = image[at..].iter().position(|&b| b == 0).expect("NUL");
        &image[at..at + end]
    }

    /// Looks `name` up as glibc does in a vDSO: the dynamic segment from the
    /// program headers, the System V hash chain, the name, and the version by its
    /// hash and its string. Returns the symbol's value.
    fn lookup(image: &[u8], name: &str, version: &str) -> Option<u64> {
        assert_eq!(&image[..4], b"\x7fELF");
        let (phoff, phnum) = (u64_at(image, 32) as usize, u16_at(image, 56) as usize);
        let mut dynamic = None;
        let mut loads = 0;
        for i in 0..phnum {
            let ph = phoff + 56 * i;
            match u32_at(image, ph) {
                // The object is used where it lies: offsets must equal addresses.
                1 => {
                    loads += 1;
                    assert_eq!((u64_at(image, ph + 8), u64_at(image, ph + 16)), (0, 0));
                    assert_eq!(u64_at(image, ph + 32) as usize, image.len());
                }
                2 => dynamic = Some(u64_at(image, ph + 16) as usize),
                _ => {}
            }
        }
        assert_eq!(loads, 1, "glibc takes the first loadable segment's address");
        let mut tags = std::collections::HashMap::new();
        let mut at = dynamic?;
        while u64_at(image, at) != 0 {
            tags.insert(u64_at(image, at), u64_at(image, at + 8) as usize);
            at += 16;
        }
        let (hash, strings, symbols) = (tags[&4], tags[&5], tags[&6]);
        let (versym, verdef) = (tags[&0x6fff_fff0], tags[&0x6fff_fffc]);
        assert_eq!(tags[&11], 24);

        // The version's index, from the definitions that are not the base.
        let mut index = None;
        let mut def = verdef;
        for _ in 0..tags[&0x6fff_fffd] {
            assert_eq!(u16_at(image, def), 1);
            let aux = def + u32_at(image, def + 12) as usize;
            let text = c_str(image, strings + u32_at(image, aux) as usize);
            if u16_at(image, def + 2) & 1 == 0
                && text == version.as_bytes()
                && u32_at(image, def + 8) == elf_hash(version.as_bytes())
            {
                index = Some(u16_at(image, def + 4));
            }
            def += u32_at(image, def + 16) as usize;
        }
        let index = index?;

        let buckets = u32_at(image, hash) as usize;
        let chain = hash + 8 + 4 * buckets;
        let bucket = elf_hash(name.as_bytes()) as usize % buckets;
        let mut symbol = u32_at(image, hash + 8 + 4 * bucket) as usize;
        while symbol != 0 {
            let entry = symbols + 24 * symbol;
            let text = c_str(image, strings + u32_at(image, entry) as usize);
            let defined = u16_at(image, entry + 6) != 0 && u64_at(image, entry + 8) != 0;
            if text == name.as_bytes()
                && defined
                && u16_at(image, versym + 2 * symbol) & 0x7fff == index
            {
                return Some(u64_at(image, entry + 8));
            }
            symbol = u32_at(image, chain + 4 * symbol) as usize;
        }
        None
    }

    #[test]
    fn the_version_hash_is_the_one_glibc_asks_for() {
        // VDSO_HASH of LINUX_2.6 in glibc's sysdeps/unix/sysv/linux/x86/dl-vdso.h.
        assert_eq!(elf_hash(VERSION.as_bytes()), 61_765_110);
    }

    #[test]
    fn each_symbol_is_found_as_glibc_looks_it_up() {
        let targets = [
            ("__vdso_clock_gettime", 0x1122_3344_5566_7788u64),
            ("__vdso_gettimeofday", 0x1000),
            ("__vdso_time", 0x7fff_0000_2000),
        ];
        let image = image(&targets);
        assert_eq!(image.len(), PAGE);
        for (name, target) in targets {
            let stub = lookup(&image, name, "LINUX_2.6").expect(name) as usize;
            assert_eq!(stub % 16, 0);
            assert_eq!(image[stub..stub + 2], [0x48, 0xb8], "movabs rax");
            assert_eq!(u64_at(&image, stub + 2), target);
            assert_eq!(image[stub + 10..stub + 12], [0xff, 0xe0], "jmp rax");
        }
        assert_eq!(lookup(&image, "__vdso_getcpu", "LINUX_2.6"), None);
        assert_eq!(lookup(&image, "__vdso_time", "LINUX_2.5"), None);
    }

    #[test]
    fn an_image_without_symbols_is_still_an_object() {
        let image = image(&[]);
        assert_eq!(lookup(&image, "__vdso_time", "LINUX_2.6"), None);
    }

    #[test]
    fn the_functions_answer_as_linux_does() {
        clock::init();
        let mut ts = [0i64; 2];
        assert_eq!(clock_gettime(1, std::ptr::from_mut(&mut ts) as u64), 0);
        assert!(ts[0] > 0 && (0..1_000_000_000).contains(&ts[1]), "{ts:?}");
        // The thread's processor time, and a null pointer, go to the system call.
        assert_eq!(clock_gettime(3, std::ptr::from_mut(&mut ts) as u64), ENOSYS);
        assert_eq!(clock_gettime(1, 0), ENOSYS);
        assert_eq!(
            clock_gettime(-6, std::ptr::from_mut(&mut ts) as u64),
            ENOSYS
        );

        let (mut tv, mut tz) = ([-1i64; 2], [-1i32; 2]);
        let status = gettimeofday(
            std::ptr::from_mut(&mut tv) as u64,
            std::ptr::from_mut(&mut tz) as u64,
        );
        assert_eq!((status, tz), (0, [0, 0]));
        assert!((0..1_000_000).contains(&tv[1]), "microseconds {}", tv[1]);
        let mut seconds = 0i64;
        let now = time(std::ptr::from_mut(&mut seconds) as u64);
        assert_eq!(now, seconds);
        assert!((tv[0]..=tv[0] + 2).contains(&now));
        assert_eq!(time(0) / 10, now / 10);
    }

    /// The mapped object is called as the guest calls it: in the guest domain,
    /// through the symbol's jump, with GS at a block of poison words.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_mapped_object_runs_in_the_guest_domain() {
        let image = image(&entries());
        let base = map(&image).expect("mapped");
        let stub = base + lookup(&image, "__vdso_time", "LINUX_2.6").expect("time");
        // SAFETY: the stub jumps to `time`, which takes a pointer and returns
        // seconds; zero is a pointer it does not write through.
        let time_stub: extern "C" fn(u64) -> u64 = unsafe { std::mem::transmute(stub as usize) };
        let seconds = cs2metal_loader::probe_neutral(time_stub, 0).expect("neutral");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        assert!((now - 2..=now + 2).contains(&seconds), "{seconds} vs {now}");

        static STUB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        extern "C" fn monotonic(_: u64) -> u64 {
            let stub = STUB.load(std::sync::atomic::Ordering::Relaxed);
            // SAFETY: the stub jumps to `clock_gettime`.
            let f: extern "C" fn(i32, u64) -> i64 = unsafe { std::mem::transmute(stub as usize) };
            let mut ts = [0i64; 2];
            if f(1, std::ptr::from_mut(&mut ts) as u64) != 0 {
                return 0;
            }
            ts[0] as u64 * clock::NANOS + ts[1] as u64
        }
        let at = base + lookup(&image, "__vdso_clock_gettime", "LINUX_2.6").expect("clock");
        STUB.store(at, std::sync::atomic::Ordering::Relaxed);
        let first = cs2metal_loader::probe_neutral(monotonic, 0).expect("neutral");
        let second = cs2metal_loader::probe_neutral(monotonic, 0).expect("neutral");
        assert!(first != 0 && second >= first, "{first} then {second}");
    }
}
