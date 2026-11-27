//! Vulkan commands recorded on the guest's side of the bridge.
//!
//! Every Vulkan call the guest makes crosses the bridge with two GS switches,
//! half a microsecond for the pair. The renderer makes about eight thousand
//! calls a frame, and three quarters of them are `vkCmd*`: commands recorded
//! into a command buffer, which return nothing and take effect only when the
//! buffer is executed (docs/PERFORMANCE_HOST_CHANGES_2026-11-27.md).
//!
//! For those, the guest is given a function that does not cross at all. It runs
//! in the guest's domain and writes the command, with a copy of everything its
//! arguments point at, into a stream kept for the command buffer. The stream is
//! *flushed*, replayed in order into the host's driver, in one crossing: when
//! the guest ends the command buffer, when the stream is full, and before any
//! command on the same buffer that is not recorded. So the driver sees the same
//! commands in the same order, later, and before anything could tell the
//! difference: Vulkan asks nothing of a command buffer between its begin and
//! its end.
//!
//! The recording functions are neutral, as [`crate::clock`] says: no thread
//! data, no lock, no allocation, no panic. Streams are made in the host domain,
//! when the guest begins a command buffer, and found by neutral code in a table
//! of atomics. A command that cannot be recorded as it is (a structure with a
//! `pNext` chain, a command buffer without a stream, a command larger than a
//! stream) is passed to the driver directly, after a flush: slower, never wrong.
//!
//! `CS2METAL_NO_VULKAN_RECORD=1` gives the guest the bridged functions, as
//! before.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

/// Command buffers that can have a stream at once: a power of two.
const SLOTS: usize = 4096;
/// Slots tried from a handle's own before giving up.
const PROBES: usize = 64;
/// 64-bit words in a stream: 64 KiB, about two thousand commands.
const CAPACITY: usize = 8192;
/// The words before a command's payload: its length in words, the function that
/// replays it, and the driver's function.
const HEADER: usize = 3;
/// The most elements of an array a command is recorded with; a longer one goes
/// to the driver directly.
const MOST: usize = 256;

/// Replays one recorded command: `payload` is its `words` words, `real` the
/// driver's function for it.
type Replay = unsafe extern "C" fn(cb: u64, payload: *mut u64, words: usize, real: u64);

/// The commands of one command buffer, not yet given to the driver.
struct Stream {
    /// Words in use. Only the thread that records the command buffer touches a
    /// stream, as Vulkan requires of a command buffer.
    len: AtomicUsize,
    words: *mut u64,
}

static KEYS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static STREAMS: [AtomicPtr<Stream>; SLOTS] =
    [const { AtomicPtr::new(std::ptr::null_mut()) }; SLOTS];
/// Serializes the making of streams; finding one takes no lock.
static MAKING: Mutex<()> = Mutex::new(());
/// The key of a slot whose command buffer was freed. A search goes past it,
/// and the next command buffer that needs a slot takes it, stream and all. No
/// handle has this value: handles are addresses.
const FREED: u64 = 1;
/// Command buffers that have a stream now, for the trace.
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// The entry guest code calls to have a stream flushed: [`flush_host`] behind a
/// host thunk. Zero until [`start`].
static FLUSH: AtomicU64 = AtomicU64::new(0);

/// Counts for the trace: commands recorded, flushes, and commands that went to
/// the driver directly.
static COUNTING: AtomicBool = AtomicBool::new(false);
static RECORDED: AtomicU64 = AtomicU64::new(0);
static FLUSHES: AtomicU64 = AtomicU64::new(0);
static DIRECT: AtomicU64 = AtomicU64::new(0);

/// Whether commands are recorded: not with `CS2METAL_NO_VULKAN_RECORD`.
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_NO_VULKAN_RECORD").is_none())
}

/// Gives the recorder the entry that flushes from guest code, and says whether
/// to count for the trace. Host domain.
pub(crate) fn start(flush_entry: u64, counting: bool) {
    FLUSH.store(flush_entry, Ordering::Release);
    COUNTING.store(counting, Ordering::Relaxed);
}

fn count(counter: &AtomicU64) {
    if COUNTING.load(Ordering::Relaxed) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// The counts since the last call: recorded, flushes, direct.
pub(crate) fn take_counts() -> (u64, u64, u64) {
    (
        RECORDED.swap(0, Ordering::Relaxed),
        FLUSHES.swap(0, Ordering::Relaxed),
        DIRECT.swap(0, Ordering::Relaxed),
    )
}

/// Where a command buffer's slot search starts. Handles are pointers: their low
/// bits say nothing.
fn first_slot(cb: u64) -> usize {
    ((cb >> 4).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 52) as usize & (SLOTS - 1)
}

/// The stream of a command buffer, if it has one. Neutral.
fn find(cb: u64) -> Option<&'static Stream> {
    if cb <= FREED {
        return None;
    }
    let mut slot = first_slot(cb);
    for _ in 0..PROBES {
        let key = KEYS.get(slot)?.load(Ordering::Acquire);
        if key == cb {
            let stream = STREAMS.get(slot)?.load(Ordering::Acquire);
            // SAFETY: a stream is leaked when it is made and its pointer stored
            // before its key, so a key found names a stream that lives on.
            return unsafe { stream.as_ref() };
        }
        if key == 0 {
            return None;
        }
        slot = (slot + 1) & (SLOTS - 1);
    }
    None
}

/// Makes sure `cb` has a stream, and empties it: the guest begins the command
/// buffer. Without a free slot the buffer has none, and its commands go to the
/// driver directly. Host domain.
pub(crate) fn begin(cb: u64) {
    if cb == 0 {
        return;
    }
    if let Some(stream) = find(cb) {
        stream.len.store(0, Ordering::Relaxed);
        return;
    }
    let Ok(_making) = MAKING.lock() else {
        return;
    };
    // The first slot a freed command buffer left, or else the first empty one.
    let mut slot = first_slot(cb);
    let mut freed = None;
    let mut empty = None;
    for _ in 0..PROBES {
        match KEYS.get(slot).map(|key| key.load(Ordering::Acquire)) {
            Some(key) if key == cb => return,
            Some(FREED) if freed.is_none() => freed = Some(slot),
            Some(0) => {
                empty = Some(slot);
                break;
            }
            _ => {}
        }
        slot = (slot + 1) & (SLOTS - 1);
    }
    let Some(slot) = freed.or(empty) else {
        return;
    };
    let (Some(key), Some(place)) = (KEYS.get(slot), STREAMS.get(slot)) else {
        return;
    };
    if place.load(Ordering::Acquire).is_null() {
        let words = Box::leak(vec![0u64; CAPACITY].into_boxed_slice()).as_mut_ptr();
        let stream = Box::leak(Box::new(Stream {
            len: AtomicUsize::new(0),
            words,
        }));
        place.store(stream, Ordering::Release);
    }
    // SAFETY: the slot's stream was stored above or when the slot was first
    // used, and streams are never freed.
    if let Some(stream) = unsafe { place.load(Ordering::Acquire).as_ref() } {
        stream.len.store(0, Ordering::Relaxed);
    }
    key.store(cb, Ordering::Release);
    LIVE.fetch_add(1, Ordering::Relaxed);
}

/// Gives up the stream of `cb`: the guest frees the command buffer, after which
/// no thread may name it. Its slot, with the stream, goes to the next command
/// buffer that needs one. Host domain.
pub(crate) fn free(cb: u64) {
    let Ok(_making) = MAKING.lock() else {
        return;
    };
    let mut slot = first_slot(cb);
    for _ in 0..PROBES {
        let Some(key) = KEYS.get(slot) else {
            return;
        };
        match key.load(Ordering::Acquire) {
            k if k == cb && cb != 0 => {
                key.store(FREED, Ordering::Release);
                LIVE.fetch_sub(1, Ordering::Relaxed);
                return;
            }
            0 => return,
            _ => slot = (slot + 1) & (SLOTS - 1),
        }
    }
}

/// How many command buffers have a stream.
pub(crate) fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// Drops what is recorded for `cb` without replaying it: the guest resets the
/// command buffer. Either domain.
pub(crate) fn discard(cb: u64) {
    if let Some(stream) = find(cb) {
        stream.len.store(0, Ordering::Relaxed);
    }
}

/// Replays the stream of `cb` into the driver, in order, and empties it. Host
/// domain: guest code reaches it through the host thunk.
pub(crate) extern "C" fn flush_host(cb: u64) {
    let Some(stream) = find(cb) else {
        return;
    };
    let len = stream.len.load(Ordering::Relaxed).min(CAPACITY);
    let mut at = 0;
    while at + HEADER <= len {
        // SAFETY: `at` is where a command starts: every command is written whole
        // by `room` and its caller, header first, and `len` covers only whole
        // commands. The header's second word is a `Replay` this module stored.
        unsafe {
            let command = stream.words.add(at);
            let words = *command as usize;
            if words < HEADER || at + words > len {
                break;
            }
            let replay: Replay = std::mem::transmute(*command.add(1) as usize);
            replay(cb, command.add(HEADER), words - HEADER, *command.add(2));
            at += words;
        }
    }
    stream.len.store(0, Ordering::Relaxed);
    if len != 0 {
        count(&FLUSHES);
    }
}

/// Flushes `cb` from the guest's domain, if anything is recorded for it. This is
/// what runs before a command that is not recorded. Neutral until it has
/// something to flush; the flush itself crosses the bridge.
pub(crate) extern "C" fn flush_pending(cb: u64) {
    let Some(stream) = find(cb) else {
        return;
    };
    if stream.len.load(Ordering::Relaxed) == 0 {
        return;
    }
    let flush = FLUSH.load(Ordering::Acquire);
    if flush == 0 {
        return;
    }
    // SAFETY: `start` stored the bridged entry of `flush_host`.
    let flush: extern "C" fn(u64) = unsafe { std::mem::transmute(flush as usize) };
    flush(cb);
}

/// Room for a command of `payload` words in the stream of `cb`, with its header
/// written: the place of its payload. `None` where the command cannot be
/// recorded; what was recorded before it is then flushed, so the caller may give
/// the command to the driver at once. Neutral unless it flushes.
fn room(cb: u64, payload: usize, replay: Replay, real: u64) -> Option<*mut u64> {
    let stream = find(cb)?;
    let words = HEADER + payload;
    if real == 0 || words > CAPACITY {
        flush_pending(cb);
        return None;
    }
    if stream.len.load(Ordering::Relaxed) + words > CAPACITY {
        flush_pending(cb);
    }
    let at = stream.len.load(Ordering::Relaxed);
    if at + words > CAPACITY {
        return None;
    }
    // SAFETY: the stream holds CAPACITY words, and at + words is within them.
    unsafe {
        let command = stream.words.add(at);
        *command = words as u64;
        *command.add(1) = replay as usize as u64;
        *command.add(2) = real;
        stream.len.store(at + words, Ordering::Relaxed);
        count(&RECORDED);
        Some(command.add(HEADER))
    }
}

/// What the recorder keeps for one Vulkan function: the driver's function, to
/// replay with, and the bridged entry the guest would have been given, to call
/// when a command cannot be recorded.
pub(crate) struct Slot {
    real: AtomicU64,
    direct: AtomicU64,
}

impl Slot {
    const fn new() -> Self {
        Self {
            real: AtomicU64::new(0),
            direct: AtomicU64::new(0),
        }
    }

    fn real(&self) -> u64 {
        self.real.load(Ordering::Relaxed)
    }

    /// The bridged entry, for a command that goes to the driver directly.
    fn direct(&self) -> usize {
        count(&DIRECT);
        self.direct.load(Ordering::Relaxed) as usize
    }
}

// --- Commands whose arguments are all integers -------------------------------------

type Plain = extern "C" fn(u64, u64, u64, u64, u64, u64);

/// Records a command that takes the command buffer and `n` more integers, none
/// of them a pointer. The registers are stored as they came; the driver reads of
/// each what its type holds.
fn plain(slot: &Slot, n: usize, cb: u64, args: [u64; 5]) {
    match room(cb, n, replay_plain, slot.real()) {
        // SAFETY: room gave n words, and n is at most 5.
        Some(payload) => unsafe {
            std::ptr::copy_nonoverlapping(args.as_ptr(), payload, n.min(5));
        },
        None => {
            // SAFETY: the bridged entry of a function of these arguments.
            let direct: Plain = unsafe { std::mem::transmute(slot.direct()) };
            direct(cb, args[0], args[1], args[2], args[3], args[4]);
        }
    }
}

unsafe extern "C" fn replay_plain(cb: u64, payload: *mut u64, words: usize, real: u64) {
    let mut args = [0u64; 5];
    // SAFETY: the payload is `words` words, at most 5, as `plain` wrote it; `real`
    // is the driver's function, which takes these integers.
    unsafe {
        std::ptr::copy_nonoverlapping(payload, args.as_mut_ptr(), words.min(5));
        let real: Plain = std::mem::transmute(real as usize);
        real(cb, args[0], args[1], args[2], args[3], args[4]);
    }
}

/// The commands recorded with [`plain`]: a module each, holding the function
/// the guest calls and its slot, and the lookup by name.
macro_rules! plain_commands {
    ($($module:ident $name:literal $n:literal,)*) => {
        $(
            mod $module {
                pub(super) static SLOT: super::Slot = super::Slot::new();
                pub(super) extern "C" fn record(cb: u64, a: u64, b: u64, c: u64, d: u64, e: u64) {
                    super::plain(&SLOT, $n, cb, [a, b, c, d, e]);
                }
            }
        )*

        fn plain_command(name: &[u8]) -> Option<(u64, &'static Slot)> {
            match name {
                $($name => Some(($module::record as *const () as u64, &$module::SLOT)),)*
                _ => None,
            }
        }
    };
}

plain_commands! {
    bind_pipeline b"vkCmdBindPipeline" 2,
    bind_index_buffer b"vkCmdBindIndexBuffer" 3,
    draw b"vkCmdDraw" 4,
    draw_indexed b"vkCmdDrawIndexed" 5,
    draw_indirect b"vkCmdDrawIndirect" 4,
    draw_indexed_indirect b"vkCmdDrawIndexedIndirect" 4,
    dispatch b"vkCmdDispatch" 3,
    dispatch_indirect b"vkCmdDispatchIndirect" 2,
    end_rendering b"vkCmdEndRendering" 0,
    end_rendering_khr b"vkCmdEndRenderingKHR" 0,
    stencil_reference b"vkCmdSetStencilReference" 2,
    stencil_compare_mask b"vkCmdSetStencilCompareMask" 2,
    stencil_write_mask b"vkCmdSetStencilWriteMask" 2,
    stencil_op b"vkCmdSetStencilOp" 5,
    stencil_op_ext b"vkCmdSetStencilOpEXT" 5,
    stencil_test b"vkCmdSetStencilTestEnable" 1,
    stencil_test_ext b"vkCmdSetStencilTestEnableEXT" 1,
    cull_mode b"vkCmdSetCullMode" 1,
    cull_mode_ext b"vkCmdSetCullModeEXT" 1,
    front_face b"vkCmdSetFrontFace" 1,
    front_face_ext b"vkCmdSetFrontFaceEXT" 1,
    topology b"vkCmdSetPrimitiveTopology" 1,
    topology_ext b"vkCmdSetPrimitiveTopologyEXT" 1,
    depth_test b"vkCmdSetDepthTestEnable" 1,
    depth_test_ext b"vkCmdSetDepthTestEnableEXT" 1,
    depth_write b"vkCmdSetDepthWriteEnable" 1,
    depth_write_ext b"vkCmdSetDepthWriteEnableEXT" 1,
    depth_compare b"vkCmdSetDepthCompareOp" 1,
    depth_compare_ext b"vkCmdSetDepthCompareOpEXT" 1,
    depth_bounds_test b"vkCmdSetDepthBoundsTestEnable" 1,
    depth_bounds_test_ext b"vkCmdSetDepthBoundsTestEnableEXT" 1,
    depth_bias_enable b"vkCmdSetDepthBiasEnable" 1,
    depth_bias_enable_ext b"vkCmdSetDepthBiasEnableEXT" 1,
    depth_clamp_ext b"vkCmdSetDepthClampEnableEXT" 1,
    polygon_mode_ext b"vkCmdSetPolygonModeEXT" 1,
    rasterizer_discard b"vkCmdSetRasterizerDiscardEnable" 1,
    rasterizer_discard_ext b"vkCmdSetRasterizerDiscardEnableEXT" 1,
    primitive_restart b"vkCmdSetPrimitiveRestartEnable" 1,
    primitive_restart_ext b"vkCmdSetPrimitiveRestartEnableEXT" 1,
    begin_query b"vkCmdBeginQuery" 3,
    end_query b"vkCmdEndQuery" 2,
    reset_query_pool b"vkCmdResetQueryPool" 3,
    fill_buffer b"vkCmdFillBuffer" 4,
}

// --- Commands with arrays and structures -------------------------------------------

/// Copies `words` 64-bit words of guest memory into a payload.
///
/// # Safety
///
/// `from` must be readable for `words` words and `to` writable for as many.
unsafe fn copy(from: u64, to: *mut u64, words: usize) {
    // SAFETY: the caller's contract. Guest structures hold 64-bit members, so the
    // arrays copied here are aligned; unaligned reads would be as correct.
    unsafe { std::ptr::copy_nonoverlapping(from as *const u8, to.cast::<u8>(), words * 8) };
}

mod depth_bias {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, f32, f32, f32);

    /// `vkCmdSetDepthBias(cb, constant, clamp, slope)`: three floats, which travel
    /// in vector registers.
    pub(super) extern "C" fn record(cb: u64, constant: f32, clamp: f32, slope: f32) {
        match super::room(cb, 2, replay, SLOT.real()) {
            // SAFETY: room gave two words.
            Some(payload) => unsafe {
                *payload = u64::from(constant.to_bits()) | u64::from(clamp.to_bits()) << 32;
                *payload.add(1) = u64::from(slope.to_bits());
            },
            None => {
                // SAFETY: the bridged entry of vkCmdSetDepthBias.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(cb, constant, clamp, slope);
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: two words, as `record` wrote them; `real` is the driver's function.
        unsafe {
            let (first, second) = (*payload, *payload.add(1));
            let real: Real = std::mem::transmute(real as usize);
            real(
                cb,
                f32::from_bits(first as u32),
                f32::from_bits((first >> 32) as u32),
                f32::from_bits(second as u32),
            );
        }
    }
}

mod blend_constants {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u64);

    /// `vkCmdSetBlendConstants(cb, const float[4])`.
    pub(super) extern "C" fn record(cb: u64, constants: u64) {
        let payload = (constants != 0)
            .then(|| super::room(cb, 2, replay, SLOT.real()))
            .flatten();
        match payload {
            // SAFETY: the guest passes four floats; room gave two words.
            Some(payload) => unsafe { super::copy(constants, payload, 2) },
            None => {
                super::flush_pending(cb);
                // SAFETY: the bridged entry of vkCmdSetBlendConstants.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(cb, constants);
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: `real` is the driver's function; the payload holds the floats.
        let real: Real = unsafe { std::mem::transmute(real as usize) };
        real(cb, payload as u64);
    }
}

/// `vkCmdSetViewportWithCount(cb, count, viewports)` and its like: a count and
/// an array of `$words`-word elements.
macro_rules! counted_array {
    ($module:ident, $words:literal) => {
        mod $module {
            pub(super) static SLOT: super::Slot = super::Slot::new();
            type Real = extern "C" fn(u64, u32, u64);

            pub(super) extern "C" fn record(cb: u64, count: u32, elements: u64) {
                let n = count as usize;
                let payload = (elements != 0 && n <= super::MOST)
                    .then(|| super::room(cb, 1 + $words * n, replay, SLOT.real()))
                    .flatten();
                match payload {
                    // SAFETY: the guest passes `count` elements; room gave a word
                    // and theirs.
                    Some(payload) => unsafe {
                        *payload = u64::from(count);
                        super::copy(elements, payload.add(1), $words * n);
                    },
                    None => {
                        super::flush_pending(cb);
                        // SAFETY: the bridged entry of a function of these arguments.
                        let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                        direct(cb, count, elements);
                    }
                }
            }

            unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
                // SAFETY: the count and the elements, as `record` wrote them.
                unsafe {
                    let real: Real = std::mem::transmute(real as usize);
                    real(cb, *payload as u32, payload.add(1) as u64);
                }
            }
        }
    };
}

// A VkViewport is six floats, a VkRect2D four 32-bit integers.
counted_array!(viewport_with_count, 3);
counted_array!(viewport_with_count_ext, 3);
counted_array!(scissor_with_count, 2);
counted_array!(scissor_with_count_ext, 2);

/// `vkCmdSetViewport(cb, first, count, viewports)` and `vkCmdSetScissor`.
macro_rules! ranged_array {
    ($module:ident, $words:literal) => {
        mod $module {
            pub(super) static SLOT: super::Slot = super::Slot::new();
            type Real = extern "C" fn(u64, u32, u32, u64);

            pub(super) extern "C" fn record(cb: u64, first: u32, count: u32, elements: u64) {
                let n = count as usize;
                let payload = (elements != 0 && n <= super::MOST)
                    .then(|| super::room(cb, 1 + $words * n, replay, SLOT.real()))
                    .flatten();
                match payload {
                    // SAFETY: as for a counted array.
                    Some(payload) => unsafe {
                        *payload = u64::from(first) | u64::from(count) << 32;
                        super::copy(elements, payload.add(1), $words * n);
                    },
                    None => {
                        super::flush_pending(cb);
                        // SAFETY: the bridged entry of a function of these arguments.
                        let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                        direct(cb, first, count, elements);
                    }
                }
            }

            unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
                // SAFETY: the range and the elements, as `record` wrote them.
                unsafe {
                    let real: Real = std::mem::transmute(real as usize);
                    let range = *payload;
                    real(
                        cb,
                        range as u32,
                        (range >> 32) as u32,
                        payload.add(1) as u64,
                    );
                }
            }
        }
    };
}

ranged_array!(viewport, 3);
ranged_array!(scissor, 2);

mod bind_vertex_buffers {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u32, u32, u64, u64);

    /// `vkCmdBindVertexBuffers(cb, first, count, buffers, offsets)`: the call the
    /// renderer makes most.
    pub(super) extern "C" fn record(cb: u64, first: u32, count: u32, buffers: u64, offsets: u64) {
        let n = count as usize;
        let payload = (buffers != 0 && offsets != 0 && n <= super::MOST)
            .then(|| super::room(cb, 1 + 2 * n, replay, SLOT.real()))
            .flatten();
        match payload {
            // SAFETY: the guest passes `count` handles and as many offsets; room
            // gave a word and both arrays.
            Some(payload) => unsafe {
                *payload = u64::from(first) | u64::from(count) << 32;
                super::copy(buffers, payload.add(1), n);
                super::copy(offsets, payload.add(1 + n), n);
            },
            None => {
                super::flush_pending(cb);
                // SAFETY: the bridged entry of vkCmdBindVertexBuffers.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(cb, first, count, buffers, offsets);
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the range and the two arrays, as `record` wrote them.
        unsafe {
            let real: Real = std::mem::transmute(real as usize);
            let range = *payload;
            let count = (range >> 32) as u32;
            real(
                cb,
                range as u32,
                count,
                payload.add(1) as u64,
                payload.add(1 + count as usize) as u64,
            );
        }
    }
}

mod bind_descriptor_sets {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u32, u64, u32, u32, u64, u32, u64);

    /// `vkCmdBindDescriptorSets(cb, bindPoint, layout, firstSet, count, sets,
    /// dynamicOffsetCount, dynamicOffsets)`. It is replayed through the bridge's
    /// own wrapper, which leaves null sets out.
    #[allow(clippy::too_many_arguments)]
    pub(super) extern "C" fn record(
        cb: u64,
        bind_point: u32,
        layout: u64,
        first: u32,
        count: u32,
        sets: u64,
        dynamic_count: u32,
        dynamic: u64,
    ) {
        let (n, d) = (count as usize, dynamic_count as usize);
        let recordable = (sets != 0 || n == 0)
            && (dynamic != 0 || d == 0)
            && n <= super::MOST
            && d <= super::MOST;
        let payload = recordable
            .then(|| super::room(cb, 3 + n + d.div_ceil(2), replay, SLOT.real()))
            .flatten();
        match payload {
            // SAFETY: the guest passes `count` handles and `dynamic_count` 32-bit
            // offsets; room gave three words, the handles, and the offsets
            // rounded up to whole words.
            Some(payload) => unsafe {
                *payload = u64::from(bind_point) | u64::from(first) << 32;
                *payload.add(1) = layout;
                *payload.add(2) = u64::from(count) | u64::from(dynamic_count) << 32;
                super::copy(sets, payload.add(3), n);
                std::ptr::copy_nonoverlapping(
                    dynamic as *const u8,
                    payload.add(3 + n).cast::<u8>(),
                    4 * d,
                );
            },
            None => {
                super::flush_pending(cb);
                // SAFETY: the bridged entry of vkCmdBindDescriptorSets.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(
                    cb,
                    bind_point,
                    layout,
                    first,
                    count,
                    sets,
                    dynamic_count,
                    dynamic,
                );
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the three words, the handles, and the offsets, as `record`
        // wrote them.
        unsafe {
            let real: Real = std::mem::transmute(real as usize);
            let (points, counts) = (*payload, *payload.add(2));
            let count = counts as u32;
            real(
                cb,
                points as u32,
                *payload.add(1),
                (points >> 32) as u32,
                count,
                payload.add(3) as u64,
                (counts >> 32) as u32,
                payload.add(3 + count as usize) as u64,
            );
        }
    }
}

mod push_constants {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u64, u32, u32, u32, u64);

    /// `vkCmdPushConstants(cb, layout, stages, offset, size, values)`.
    pub(super) extern "C" fn record(
        cb: u64,
        layout: u64,
        stages: u32,
        offset: u32,
        size: u32,
        values: u64,
    ) {
        let bytes = size as usize;
        let payload = (values != 0 && bytes <= 8 * super::MOST)
            .then(|| super::room(cb, 3 + bytes.div_ceil(8), replay, SLOT.real()))
            .flatten();
        match payload {
            // SAFETY: the guest passes `size` bytes; room gave three words and
            // the bytes rounded up to whole words.
            Some(payload) => unsafe {
                *payload = layout;
                *payload.add(1) = u64::from(stages) | u64::from(offset) << 32;
                *payload.add(2) = u64::from(size);
                std::ptr::copy_nonoverlapping(
                    values as *const u8,
                    payload.add(3).cast::<u8>(),
                    bytes,
                );
            },
            None => {
                super::flush_pending(cb);
                // SAFETY: the bridged entry of vkCmdPushConstants.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(cb, layout, stages, offset, size, values);
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the three words and the bytes, as `record` wrote them.
        unsafe {
            let real: Real = std::mem::transmute(real as usize);
            let range = *payload.add(1);
            real(
                cb,
                *payload,
                range as u32,
                (range >> 32) as u32,
                *payload.add(2) as u32,
                payload.add(3) as u64,
            );
        }
    }
}

mod copy_buffer {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u64, u64, u32, u64);

    /// `vkCmdCopyBuffer(cb, source, destination, count, regions)`; a
    /// VkBufferCopy is three 64-bit words.
    pub(super) extern "C" fn record(
        cb: u64,
        source: u64,
        destination: u64,
        count: u32,
        regions: u64,
    ) {
        let n = count as usize;
        let payload = (regions != 0 && n <= super::MOST)
            .then(|| super::room(cb, 3 + 3 * n, replay, SLOT.real()))
            .flatten();
        match payload {
            // SAFETY: the guest passes `count` regions; room gave three words
            // and theirs.
            Some(payload) => unsafe {
                *payload = source;
                *payload.add(1) = destination;
                *payload.add(2) = u64::from(count);
                super::copy(regions, payload.add(3), 3 * n);
            },
            None => {
                super::flush_pending(cb);
                // SAFETY: the bridged entry of vkCmdCopyBuffer.
                let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
                direct(cb, source, destination, count, regions);
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the three words and the regions, as `record` wrote them.
        unsafe {
            let real: Real = std::mem::transmute(real as usize);
            real(
                cb,
                *payload,
                *payload.add(1),
                *payload.add(2) as u32,
                payload.add(3) as u64,
            );
        }
    }
}

/// A word of a guest structure.
///
/// # Safety
///
/// `structure` must be readable for `index + 1` words.
unsafe fn word(structure: u64, index: usize) -> u64 {
    // SAFETY: the caller's contract.
    unsafe { std::ptr::read_unaligned((structure as *const u64).add(index)) }
}

/// Whether each of the `count` structures of `words` words at `array` has a null
/// `pNext`, its second word. A chain is not copied, so a structure with one is
/// not recorded.
///
/// # Safety
///
/// `array` must be null or hold `count` structures.
unsafe fn unchained(array: u64, count: usize, words: usize) -> bool {
    if array == 0 {
        return count == 0;
    }
    // SAFETY: the caller's contract.
    (0..count).all(|i| unsafe { word(array + (8 * words * i) as u64, 1) } == 0)
}

mod begin_rendering {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u64);

    /// Words in a VkRenderingInfo and in a VkRenderingAttachmentInfo.
    const INFO: usize = 9;
    const ATTACHMENT: usize = 9;

    /// `vkCmdBeginRendering(cb, info)`. The info names an array of color
    /// attachments and, by pointer, a depth and a stencil attachment. All are
    /// copied behind the info, and its three pointers are kept as offsets until
    /// the replay turns them back into addresses.
    pub(super) extern "C" fn record(cb: u64, info: u64) {
        if let Some(payload) = plan(cb, info) {
            // SAFETY: `plan` checked the structures and reserved their words.
            unsafe { write(info, payload) };
            return;
        }
        super::flush_pending(cb);
        // SAFETY: the bridged entry of vkCmdBeginRendering.
        let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
        direct(cb, info);
    }

    fn plan(cb: u64, info: u64) -> Option<*mut u64> {
        if info == 0 {
            return None;
        }
        // SAFETY: the guest passes a VkRenderingInfo, nine words; each pointer in
        // it is null or names what the specification says.
        unsafe {
            let colors = (super::word(info, 5) >> 32) as usize;
            let (color, depth, stencil) = (
                super::word(info, 6),
                super::word(info, 7),
                super::word(info, 8),
            );
            let plain = super::word(info, 1) == 0
                && colors <= super::MOST
                && super::unchained(color, colors, ATTACHMENT)
                && super::unchained(depth, usize::from(depth != 0), ATTACHMENT)
                && super::unchained(stencil, usize::from(stencil != 0), ATTACHMENT);
            if !plain {
                return None;
            }
            let attachments = colors + usize::from(depth != 0) + usize::from(stencil != 0);
            super::room(cb, INFO + ATTACHMENT * attachments, replay, SLOT.real())
        }
    }

    /// # Safety
    ///
    /// `info` must be a VkRenderingInfo that `plan` accepted, and `payload` the
    /// room it reserved.
    unsafe fn write(info: u64, payload: *mut u64) {
        // SAFETY: the caller's contract.
        unsafe {
            super::copy(info, payload, INFO);
            let colors = (*payload.add(5) >> 32) as usize;
            let mut at = INFO;
            super::copy(*payload.add(6), payload.add(at), ATTACHMENT * colors);
            *payload.add(6) = at as u64;
            at += ATTACHMENT * colors;
            for pointer in [7, 8] {
                let attachment = *payload.add(pointer);
                if attachment != 0 {
                    super::copy(attachment, payload.add(at), ATTACHMENT);
                    *payload.add(pointer) = at as u64;
                    at += ATTACHMENT;
                }
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the info and its attachments, as `write` laid them out; an
        // offset of zero stands for a null pointer, since the info itself is
        // at zero.
        unsafe {
            let colors = (*payload.add(5) >> 32) as usize;
            for pointer in [6, 7, 8] {
                let offset = *payload.add(pointer) as usize;
                let none = offset == 0 || (pointer == 6 && colors == 0);
                *payload.add(pointer) = if none { 0 } else { payload.add(offset) as u64 };
            }
            let real: Real = std::mem::transmute(real as usize);
            real(cb, payload as u64);
        }
    }
}

mod pipeline_barrier2 {
    pub(super) static SLOT: super::Slot = super::Slot::new();
    type Real = extern "C" fn(u64, u64);

    /// Words in a VkDependencyInfo, and in a VkMemoryBarrier2, a
    /// VkBufferMemoryBarrier2, and a VkImageMemoryBarrier2; the info's words
    /// that hold each array's count and pointer.
    const INFO: usize = 8;
    const BARRIERS: [(usize, usize); 3] = [(3, 6), (5, 10), (7, 12)];

    /// The three counts of a VkDependencyInfo.
    ///
    /// # Safety
    ///
    /// `info` must be readable for eight words.
    unsafe fn counts(info: *const u64) -> [usize; 3] {
        // SAFETY: the caller's contract. The counts are 32-bit: the first shares
        // its word with the flags, the others have theirs to themselves.
        unsafe {
            [
                (*info.add(2) >> 32) as usize,
                *info.add(4) as u32 as usize,
                *info.add(6) as u32 as usize,
            ]
        }
    }

    /// `vkCmdPipelineBarrier2(cb, info)`: the info names three arrays of
    /// barriers, copied behind it.
    pub(super) extern "C" fn record(cb: u64, info: u64) {
        if let Some(payload) = plan(cb, info) {
            // SAFETY: `plan` checked the structures and reserved their words.
            unsafe { write(info, payload) };
            return;
        }
        super::flush_pending(cb);
        // SAFETY: the bridged entry of vkCmdPipelineBarrier2.
        let direct: Real = unsafe { std::mem::transmute(SLOT.direct()) };
        direct(cb, info);
    }

    fn plan(cb: u64, info: u64) -> Option<*mut u64> {
        if info == 0 {
            return None;
        }
        // SAFETY: the guest passes a VkDependencyInfo, eight words; each pointer
        // in it names as many barriers as the count beside it.
        unsafe {
            let counts = counts(info as *const u64);
            let mut words = INFO;
            let mut plain = super::word(info, 1) == 0;
            for ((pointer, size), count) in BARRIERS.into_iter().zip(counts) {
                plain = plain
                    && count <= super::MOST
                    && super::unchained(super::word(info, pointer), count, size);
                words += size * count;
            }
            if !plain {
                return None;
            }
            super::room(cb, words, replay, SLOT.real())
        }
    }

    /// # Safety
    ///
    /// `info` must be a VkDependencyInfo that `plan` accepted, and `payload` the
    /// room it reserved.
    unsafe fn write(info: u64, payload: *mut u64) {
        // SAFETY: the caller's contract.
        unsafe {
            super::copy(info, payload, INFO);
            let counts = counts(payload);
            let mut at = INFO;
            for ((pointer, size), count) in BARRIERS.into_iter().zip(counts) {
                super::copy(*payload.add(pointer), payload.add(at), size * count);
                *payload.add(pointer) = at as u64;
                at += size * count;
            }
        }
    }

    unsafe extern "C" fn replay(cb: u64, payload: *mut u64, _words: usize, real: u64) {
        // SAFETY: the info and its barriers, as `write` laid them out.
        unsafe {
            let counts = counts(payload);
            for ((pointer, _), count) in BARRIERS.into_iter().zip(counts) {
                let offset = *payload.add(pointer) as usize;
                *payload.add(pointer) = if count == 0 {
                    0
                } else {
                    payload.add(offset) as u64
                };
            }
            let real: Real = std::mem::transmute(real as usize);
            real(cb, payload as u64);
        }
    }
}

/// The recording function for the Vulkan function `name`, if it is one that is
/// recorded: `real` is the function to replay with, in the host domain, and
/// `direct` the bridged entry the guest would otherwise be given. Host domain.
pub(crate) fn command(name: &[u8], real: u64, direct: u64) -> Option<u64> {
    let (record, slot): (u64, &'static Slot) = match name {
        b"vkCmdSetDepthBias" => (depth_bias::record as *const () as u64, &depth_bias::SLOT),
        b"vkCmdSetBlendConstants" => (
            blend_constants::record as *const () as u64,
            &blend_constants::SLOT,
        ),
        b"vkCmdSetViewportWithCount" => (
            viewport_with_count::record as *const () as u64,
            &viewport_with_count::SLOT,
        ),
        b"vkCmdSetViewportWithCountEXT" => (
            viewport_with_count_ext::record as *const () as u64,
            &viewport_with_count_ext::SLOT,
        ),
        b"vkCmdSetScissorWithCount" => (
            scissor_with_count::record as *const () as u64,
            &scissor_with_count::SLOT,
        ),
        b"vkCmdSetScissorWithCountEXT" => (
            scissor_with_count_ext::record as *const () as u64,
            &scissor_with_count_ext::SLOT,
        ),
        b"vkCmdSetViewport" => (viewport::record as *const () as u64, &viewport::SLOT),
        b"vkCmdSetScissor" => (scissor::record as *const () as u64, &scissor::SLOT),
        b"vkCmdBindVertexBuffers" => (
            bind_vertex_buffers::record as *const () as u64,
            &bind_vertex_buffers::SLOT,
        ),
        b"vkCmdBindDescriptorSets" => (
            bind_descriptor_sets::record as *const () as u64,
            &bind_descriptor_sets::SLOT,
        ),
        b"vkCmdPushConstants" => (
            push_constants::record as *const () as u64,
            &push_constants::SLOT,
        ),
        b"vkCmdCopyBuffer" => (copy_buffer::record as *const () as u64, &copy_buffer::SLOT),
        b"vkCmdBeginRendering" | b"vkCmdBeginRenderingKHR" => (
            begin_rendering::record as *const () as u64,
            &begin_rendering::SLOT,
        ),
        b"vkCmdPipelineBarrier2" | b"vkCmdPipelineBarrier2KHR" => (
            pipeline_barrier2::record as *const () as u64,
            &pipeline_barrier2::SLOT,
        ),
        _ => plain_command(name)?,
    };
    if real == 0 || direct == 0 {
        return None;
    }
    slot.real.store(real, Ordering::Relaxed);
    slot.direct.store(direct, Ordering::Relaxed);
    Some(record)
}

/// Whether `name` is a command recorded into a command buffer, by its name: the
/// ones not recorded here are given an entry that flushes first.
pub(crate) fn is_command(name: &[u8]) -> bool {
    name.starts_with(b"vkCmd")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder's slots and counters are the process's: one test at a time.
    static ALONE: Mutex<()> = Mutex::new(());
    /// What the stand-ins for the driver were called with, in order.
    static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn note(call: String) {
        CALLS.lock().expect("calls").push(call);
    }

    fn calls() -> Vec<String> {
        std::mem::take(&mut *CALLS.lock().expect("calls"))
    }

    extern "C" fn real_draw(cb: u64, a: u64, b: u64, c: u64, d: u64, e: u64) {
        note(format!(
            "draw {cb:#x} {} {} {} {} {e}",
            a as u32, b as u32, c as u32, d as u32
        ));
    }

    extern "C" fn direct_draw(cb: u64, a: u64, _: u64, _: u64, _: u64, _: u64) {
        note(format!("direct draw {cb:#x} {}", a as u32));
    }

    extern "C" fn real_vertex_buffers(cb: u64, first: u32, count: u32, buffers: u64, offsets: u64) {
        // SAFETY: the replay passes `count` handles and as many offsets.
        let (buffers, offsets) = unsafe {
            (
                std::slice::from_raw_parts(buffers as *const u64, count as usize),
                std::slice::from_raw_parts(offsets as *const u64, count as usize),
            )
        };
        note(format!(
            "vertex buffers {cb:#x} {first} {buffers:?} {offsets:?}"
        ));
    }

    #[allow(clippy::too_many_arguments)]
    extern "C" fn real_descriptor_sets(
        cb: u64,
        bind_point: u32,
        layout: u64,
        first: u32,
        count: u32,
        sets: u64,
        dynamic_count: u32,
        dynamic: u64,
    ) {
        // SAFETY: the replay passes `count` handles and `dynamic_count` offsets.
        let (sets, dynamic) = unsafe {
            (
                std::slice::from_raw_parts(sets as *const u64, count as usize),
                std::slice::from_raw_parts(dynamic as *const u32, dynamic_count as usize),
            )
        };
        note(format!(
            "descriptor sets {cb:#x} {bind_point} {layout:#x} {first} {sets:?} {dynamic:?}"
        ));
    }

    extern "C" fn real_push_constants(
        cb: u64,
        layout: u64,
        stages: u32,
        offset: u32,
        size: u32,
        values: u64,
    ) {
        // SAFETY: the replay passes `size` bytes.
        let values = unsafe { std::slice::from_raw_parts(values as *const u8, size as usize) };
        note(format!(
            "push constants {cb:#x} {layout:#x} {stages} {offset} {values:?}"
        ));
    }

    extern "C" fn real_depth_bias(cb: u64, constant: f32, clamp: f32, slope: f32) {
        note(format!("depth bias {cb:#x} {constant} {clamp} {slope}"));
    }

    extern "C" fn real_viewports(cb: u64, count: u32, viewports: u64) {
        // SAFETY: the replay passes `count` viewports of six floats.
        let floats =
            unsafe { std::slice::from_raw_parts(viewports as *const f32, 6 * count as usize) };
        note(format!("viewports {cb:#x} {floats:?}"));
    }

    /// The image view of the VkRenderingAttachmentInfo at `attachment`.
    fn view(attachment: u64) -> u64 {
        // SAFETY: a VkRenderingAttachmentInfo, whose third word is its image view.
        unsafe { word(attachment, 2) }
    }

    extern "C" fn real_begin_rendering(cb: u64, info: u64) {
        // SAFETY: the replay passes a VkRenderingInfo with its pointers restored.
        let (colors, color, depth, stencil) = unsafe {
            (
                (word(info, 5) >> 32) as usize,
                word(info, 6),
                word(info, 7),
                word(info, 8),
            )
        };
        let views: Vec<u64> = (0..colors).map(|i| view(color + 72 * i as u64)).collect();
        note(format!(
            "begin rendering {cb:#x} colors {views:x?} depth {:x?} stencil {:x?}",
            (depth != 0).then(|| view(depth)),
            (stencil != 0).then(|| view(stencil)),
        ));
    }

    extern "C" fn direct_begin_rendering(cb: u64, info: u64) {
        note(format!(
            "direct begin rendering {cb:#x} {}",
            u8::from(info != 0)
        ));
    }

    extern "C" fn real_barrier(cb: u64, info: u64) {
        // SAFETY: the replay passes a VkDependencyInfo with its pointers restored;
        // an image barrier's ninth word is its image.
        let line = unsafe {
            let (memory, buffers, images) = (
                (word(info, 2) >> 32) as u32,
                word(info, 4) as u32,
                word(info, 6) as u32,
            );
            let image = if images == 0 {
                0
            } else {
                word(word(info, 7), 8)
            };
            format!(
                "barrier {cb:#x} {memory} {buffers} {images} image {image:#x} pointers {} {}",
                u8::from(word(info, 3) != 0),
                u8::from(word(info, 5) != 0)
            )
        };
        note(line);
    }

    /// Registers the stand-ins and returns the recording functions, typed.
    struct Recording {
        draw: Plain,
        vertex_buffers: extern "C" fn(u64, u32, u32, u64, u64),
        descriptor_sets: extern "C" fn(u64, u32, u64, u32, u32, u64, u32, u64),
        push_constants: extern "C" fn(u64, u64, u32, u32, u32, u64),
        depth_bias: extern "C" fn(u64, f32, f32, f32),
        viewports: extern "C" fn(u64, u32, u64),
        begin_rendering: extern "C" fn(u64, u64),
        barrier: extern "C" fn(u64, u64),
    }

    fn recording() -> Recording {
        start(flush_host as *const () as u64, true);
        take_counts();
        calls();
        let entry = |name: &[u8], real: usize, direct: usize| {
            command(name, real as u64, direct as u64).expect("recorded") as usize
        };
        let unused = direct_draw as *const () as usize;
        // SAFETY: each entry is the recording function of the command named,
        // which has the type given here.
        unsafe {
            Recording {
                draw: std::mem::transmute::<usize, Plain>(entry(
                    b"vkCmdDraw",
                    real_draw as *const () as usize,
                    unused,
                )),
                vertex_buffers: std::mem::transmute::<usize, extern "C" fn(u64, u32, u32, u64, u64)>(
                    entry(
                        b"vkCmdBindVertexBuffers",
                        real_vertex_buffers as *const () as usize,
                        unused,
                    ),
                ),
                descriptor_sets: std::mem::transmute::<
                    usize,
                    extern "C" fn(u64, u32, u64, u32, u32, u64, u32, u64),
                >(entry(
                    b"vkCmdBindDescriptorSets",
                    real_descriptor_sets as *const () as usize,
                    unused,
                )),
                push_constants: std::mem::transmute::<
                    usize,
                    extern "C" fn(u64, u64, u32, u32, u32, u64),
                >(entry(
                    b"vkCmdPushConstants",
                    real_push_constants as *const () as usize,
                    unused,
                )),
                depth_bias: std::mem::transmute::<usize, extern "C" fn(u64, f32, f32, f32)>(entry(
                    b"vkCmdSetDepthBias",
                    real_depth_bias as *const () as usize,
                    unused,
                )),
                viewports: std::mem::transmute::<usize, extern "C" fn(u64, u32, u64)>(entry(
                    b"vkCmdSetViewportWithCountEXT",
                    real_viewports as *const () as usize,
                    unused,
                )),
                begin_rendering: std::mem::transmute::<usize, extern "C" fn(u64, u64)>(entry(
                    b"vkCmdBeginRenderingKHR",
                    real_begin_rendering as *const () as usize,
                    direct_begin_rendering as *const () as usize,
                )),
                barrier: std::mem::transmute::<usize, extern "C" fn(u64, u64)>(entry(
                    b"vkCmdPipelineBarrier2KHR",
                    real_barrier as *const () as usize,
                    unused,
                )),
            }
        }
    }

    /// A VkRenderingAttachmentInfo for `view`, with `next` as its pNext.
    fn attachment(view: u64, next: u64) -> [u64; 9] {
        let mut words = [0u64; 9];
        (words[0], words[1], words[2]) = (1_000_044_001, next, view);
        words
    }

    /// A VkRenderingInfo naming `colors`, a depth attachment, and no stencil.
    fn rendering_info(colors: &[[u64; 9]], depth: &[u64; 9]) -> [u64; 9] {
        let mut words = [0u64; 9];
        words[0] = 1_000_044_000;
        words[5] = (colors.len() as u64) << 32;
        words[6] = colors.as_ptr() as u64;
        words[7] = depth.as_ptr() as u64;
        words
    }

    #[test]
    fn commands_reach_the_driver_at_the_flush_in_order_with_their_own_copies() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        let cb = 0x7000_0000_1000u64;
        begin(cb);

        (r.draw)(cb, 3, 1, 0, 7, 0);
        let (mut buffers, mut offsets) = ([0xb1u64, 0xb2], [16u64, 32]);
        (r.vertex_buffers)(cb, 1, 2, buffers.as_ptr() as u64, offsets.as_ptr() as u64);
        let (mut sets, mut dynamic) = ([0x51u64, 0, 0x53], [4u32, 8, 12]);
        (r.descriptor_sets)(
            cb,
            0,
            0x1a,
            2,
            3,
            sets.as_ptr() as u64,
            3,
            dynamic.as_ptr() as u64,
        );
        let mut constants = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        (r.push_constants)(cb, 0x1b, 17, 4, 11, constants.as_ptr() as u64);
        (r.depth_bias)(cb, 1.5, -2.0, 0.25);
        let mut viewport = [0.0f32, 0.0, 2560.0, 1440.0, 0.0, 1.0];
        (r.viewports)(cb, 1, viewport.as_ptr() as u64);
        let mut colors = [attachment(0xc1, 0), attachment(0xc2, 0)];
        let mut depth = attachment(0xd1, 0);
        let mut info = rendering_info(&colors, &depth);
        (r.begin_rendering)(cb, info.as_ptr() as u64);
        // One image barrier: its ninth word is the image.
        let mut image_barrier = [0u64; 12];
        image_barrier[8] = 0x1e;
        let mut dependency = [0u64; 8];
        (dependency[6], dependency[7]) = (1, image_barrier.as_ptr() as u64);
        (r.barrier)(cb, dependency.as_ptr() as u64);
        (r.draw)(cb, 6, 2, 1, 0, 0);

        // Nothing has crossed, and what the guest passed may now change.
        assert_eq!(calls(), Vec::<String>::new());
        (buffers, offsets, sets, dynamic) = ([0; 2], [0; 2], [0; 3], [0; 3]);
        (constants, viewport) = ([0; 11], [0.0; 6]);
        (colors, depth, info) = ([[0; 9]; 2], [0; 9], [0; 9]);
        (image_barrier, dependency) = ([0; 12], [0; 8]);
        let _ = (
            buffers,
            offsets,
            sets,
            dynamic,
            constants,
            viewport,
            colors,
            depth,
            info,
            image_barrier,
            dependency,
        );

        flush_host(cb);
        assert_eq!(
            calls(),
            [
                "draw 0x700000001000 3 1 0 7 0",
                "vertex buffers 0x700000001000 1 [177, 178] [16, 32]",
                "descriptor sets 0x700000001000 0 0x1a 2 [81, 0, 83] [4, 8, 12]",
                "push constants 0x700000001000 0x1b 17 4 [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]",
                "depth bias 0x700000001000 1.5 -2 0.25",
                "viewports 0x700000001000 [0.0, 0.0, 2560.0, 1440.0, 0.0, 1.0]",
                "begin rendering 0x700000001000 colors [c1, c2] depth Some(d1) stencil None",
                "barrier 0x700000001000 0 0 1 image 0x1e pointers 0 0",
                "draw 0x700000001000 6 2 1 0 0",
            ]
        );
        assert_eq!(take_counts(), (9, 1, 0), "recorded, flushes, direct");
        // A second flush has nothing to replay.
        flush_host(cb);
        assert_eq!(calls(), Vec::<String>::new());
    }

    #[test]
    fn a_full_stream_is_flushed_and_recording_goes_on() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        let cb = 0x7000_0000_2000u64;
        begin(cb);
        // Seven words a draw: three thousand of them fill the stream twice over.
        for i in 0..3000u64 {
            (r.draw)(cb, i, 0, 0, 0, 0);
        }
        flush_host(cb);
        let seen = calls();
        assert_eq!(seen.len(), 3000);
        for (i, call) in seen.iter().enumerate() {
            assert_eq!(*call, format!("draw 0x700000002000 {i} 0 0 0 0"));
        }
        let (recorded, flushes, direct) = take_counts();
        assert_eq!((recorded, direct), (3000, 0));
        assert_eq!(flushes, 3, "two when the stream filled, and the last");
    }

    #[test]
    fn a_command_that_cannot_be_recorded_follows_a_flush() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        let cb = 0x7000_0000_3000u64;
        begin(cb);
        (r.draw)(cb, 1, 0, 0, 0, 0);
        // An attachment with a pNext chain: the chain is not copied.
        let colors = [attachment(0xc1, 0x1234)];
        let depth = attachment(0xd1, 0);
        let info = rendering_info(&colors, &depth);
        (r.begin_rendering)(cb, info.as_ptr() as u64);
        (r.draw)(cb, 2, 0, 0, 0, 0);
        assert_eq!(
            calls(),
            [
                "draw 0x700000003000 1 0 0 0 0",
                "direct begin rendering 0x700000003000 1"
            ],
            "the draw before it reached the driver first"
        );
        flush_host(cb);
        assert_eq!(calls(), ["draw 0x700000003000 2 0 0 0 0"]);
        assert_eq!(take_counts(), (2, 2, 1));
    }

    #[test]
    fn a_command_buffer_without_a_stream_goes_to_the_driver() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        // Never begun: the guest was given the function before recording knew it.
        (r.draw)(0x7000_0000_4000, 9, 0, 0, 0, 0);
        assert_eq!(calls(), ["direct draw 0x700000004000 9"]);
        assert_eq!(take_counts(), (0, 0, 1));
    }

    #[test]
    fn beginning_or_resetting_a_command_buffer_drops_what_was_recorded() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        let cb = 0x7000_0000_5000u64;
        begin(cb);
        (r.draw)(cb, 1, 0, 0, 0, 0);
        discard(cb);
        (r.draw)(cb, 2, 0, 0, 0, 0);
        begin(cb);
        (r.draw)(cb, 3, 0, 0, 0, 0);
        flush_host(cb);
        assert_eq!(calls(), ["draw 0x700000005000 3 0 0 0 0"]);
        // What runs before a command that is not recorded: a flush, if there is
        // anything to flush.
        flush_pending(cb);
        (r.draw)(cb, 4, 0, 0, 0, 0);
        flush_pending(cb);
        assert_eq!(calls(), ["draw 0x700000005000 4 0 0 0 0"]);
        take_counts();
    }

    #[test]
    fn a_freed_command_buffer_gives_its_slot_to_the_next() {
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        // Two handles that start their search at the same slot or not: either
        // way the second finds room, and the first has no stream once freed.
        let (first, second) = (0x7000_0000_7000u64, 0x7000_0000_8000u64);
        let before = live();
        begin(first);
        assert_eq!(live(), before + 1);
        (r.draw)(first, 1, 0, 0, 0, 0);
        free(first);
        assert_eq!(live(), before);
        // Freed: its command goes to the driver, and nothing is left to replay.
        (r.draw)(first, 2, 0, 0, 0, 0);
        assert_eq!(calls(), ["direct draw 0x700000007000 2"]);
        begin(second);
        (r.draw)(second, 3, 0, 0, 0, 0);
        flush_host(second);
        assert_eq!(calls(), ["draw 0x700000008000 3 0 0 0 0"]);
        // Begun again, the first handle has a stream once more.
        begin(first);
        (r.draw)(first, 4, 0, 0, 0, 0);
        flush_host(first);
        assert_eq!(calls(), ["draw 0x700000007000 4 0 0 0 0"]);
        free(first);
        free(second);
        free(0x7000_0000_9000);
        assert_eq!(live(), before);
        take_counts();
    }

    #[test]
    fn only_commands_are_recorded() {
        assert!(is_command(b"vkCmdCopyImage"));
        assert!(!is_command(b"vkQueueSubmit"));
        let stand_in = direct_draw as *const () as u64;
        assert!(command(b"vkCmdCopyImage", stand_in, stand_in).is_none());
        assert!(command(b"vkAllocateDescriptorSets", stand_in, stand_in).is_none());
        // A function the host lacks is not recorded: there is nothing to replay.
        assert!(command(b"vkCmdSetLineWidth", 0, stand_in).is_none());
    }

    /// Recording touches no thread data: a hundred draws, a binding with arrays,
    /// and a rendering info with its attachments are recorded with GS at a block
    /// of poison words. Only the flush crosses the bridge, and here there is
    /// none: the stream has room.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn recording_is_neutral() {
        static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);
        const CB: u64 = 0x7000_0000_6000;
        extern "C" fn record(draw: u64) -> u64 {
            // SAFETY: `draw` is the recording function of vkCmdDraw.
            let draw: Plain = unsafe { std::mem::transmute(draw as usize) };
            for i in 0..100 {
                draw(CB, i, 0, 0, 0, 0);
            }
            1
        }
        extern "C" fn record_structures(begin_rendering: u64) -> u64 {
            // SAFETY: the recording function of vkCmdBeginRenderingKHR.
            let begin_rendering: extern "C" fn(u64, u64) =
                unsafe { std::mem::transmute(begin_rendering as usize) };
            let colors = [attachment(0xc1, 0)];
            let depth = attachment(0xd1, 0);
            let info = rendering_info(&colors, &depth);
            begin_rendering(CB, info.as_ptr() as u64);
            1
        }
        let _alone = ALONE.lock().expect("alone");
        let r = recording();
        // Counting writes two words the probe does not look at; without it the
        // path is the one a session without a trace takes.
        start(flush_host as *const () as u64, false);
        begin(CB);
        let (draw, begin_rendering) = (r.draw as usize as u64, r.begin_rendering as usize as u64);
        *RECORDING.lock().expect("recording") = Some(r);
        assert_eq!(
            cs2metal_loader::probe_neutral(record, draw).expect("neutral"),
            1
        );
        assert_eq!(
            cs2metal_loader::probe_neutral(record_structures, begin_rendering).expect("neutral"),
            1
        );
        assert_eq!(calls(), Vec::<String>::new());
        flush_host(CB);
        let seen = calls();
        assert_eq!(seen.len(), 101);
        assert_eq!(seen[99], "draw 0x700000006000 99 0 0 0 0");
        assert_eq!(
            seen[100],
            "begin rendering 0x700000006000 colors [c1] depth Some(d1) stencil None"
        );
    }
}
