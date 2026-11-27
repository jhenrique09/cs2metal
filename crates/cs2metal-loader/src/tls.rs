//! Thread-local storage in glibc's variant II layout, and the guest threads that
//! carry it.
//!
//! Every module with a `PT_TLS` segment gets a module id (its DTV index) and a
//! static offset: its block sits at `tp - offset`, below the thread pointer, in
//! load order, as glibc places the modules loaded at startup. Because every block
//! is static, `__tls_get_addr` is a DTV lookup that never allocates, which keeps
//! it neutral. Modules loaded after a thread was created have no block in that
//! thread yet.

use std::marker::PhantomData;

use crate::bridge::{self, HostBlock, tcb};
use crate::error::{Error, Result};
use crate::memory::{Region, round_up};

/// Where a module's TLS block lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsSlot {
    /// The module's DTV index, from 1.
    pub id: u64,
    /// The block is at `tp - offset`.
    pub offset: u64,
}

/// One module's TLS template and slot.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TlsModule {
    pub slot: TlsSlot,
    /// Address of the initialization image in the loaded module.
    pub template: u64,
    /// Bytes of initialized data; the rest of the block is `.tbss`, left zero.
    pub file_size: u64,
}

/// The TLS layout of a namespace.
#[derive(Debug, Default)]
pub(crate) struct TlsLayout {
    pub modules: Vec<TlsModule>,
    /// Bytes of static TLS used below the thread pointer.
    pub used: u64,
    /// The largest alignment of any block; the thread pointer is aligned to it.
    pub align: u64,
}

impl TlsLayout {
    /// Places a block of `memory_size` bytes aligned to `align`.
    pub fn place(&mut self, memory_size: u64, align: u64) -> TlsSlot {
        let align = align.max(1);
        let offset = round_up(self.used + memory_size, align);
        self.used = offset;
        self.align = self.align.max(align);
        TlsSlot {
            id: self.modules.len() as u64 + 1,
            offset,
        }
    }
}

fn random_word() -> u64 {
    let mut bytes = [0u8; 8];
    // SAFETY: getentropy writes at most the given length into the buffer.
    let status = unsafe { libc::getentropy(bytes.as_mut_ptr().cast(), bytes.len()) };
    if status != 0 {
        return 0x00ff_0a0d_0000_0000;
    }
    u64::from_le_bytes(bytes)
}

/// A guest thread control block with its TLS blocks and DTV, bound to the host
/// thread that created it.
#[derive(Debug)]
pub struct GuestThread {
    region: Region,
    /// Kept alive as long as the control block points at it.
    _host: HostBlock,
    tp: u64,
    /// The block holds this thread's host TSD base, so it must stay on it.
    _not_send: PhantomData<*const ()>,
}

impl GuestThread {
    pub(crate) fn new(layout: &TlsLayout) -> Result<Self> {
        let page = 4096;
        let below = round_up(layout.used, page.max(layout.align));
        let dtv_size = 16 * (layout.modules.len() as u64 + 2);
        let size = below + tcb::SIZE + round_up(dtv_size, page);
        let region = Region::reserve(size)?;
        region.map_zero(0, size)?;
        let tp = region.address() + below;
        let host = HostBlock::new(tp)?;
        let at = |address: u64| address - region.address();

        for m in &layout.modules {
            let block = tp - m.slot.offset;
            // SAFETY: the template lies in a loaded, readable segment of a module
            // the namespace keeps mapped, file_size bytes long.
            let image = unsafe {
                std::slice::from_raw_parts(m.template as *const u8, m.file_size as usize)
            };
            region.write_bytes(at(block), image)?;
            // The rest, up to memory_size, is .tbss and already zero.
        }

        // The DTV: a length word, a generation word, then {block, to_free} per module.
        let dtv_base = tp + tcb::SIZE;
        region.write_u64(at(dtv_base), layout.modules.len() as u64)?;
        let dtv = dtv_base + 16;
        region.write_u64(at(dtv), 1)?;
        for m in &layout.modules {
            region.write_u64(at(dtv + 16 * m.slot.id), tp - m.slot.offset)?;
        }

        // glibc zeroes the canary's low byte, so string overflows stop on it.
        let guard = random_word() & !0xff;
        for (offset, value) in [
            (tcb::SELF, tp),
            (tcb::DTV, dtv),
            (tcb::SELF_AGAIN, tp),
            (tcb::STACK_GUARD, guard),
            (tcb::POINTER_GUARD, random_word()),
            (tcb::SYSINFO, bridge::syscall_entry()),
            (tcb::HOST_BLOCK, host.address()),
        ] {
            region.write_u64(at(tp + offset), value)?;
        }
        Ok(Self {
            region,
            _host: host,
            tp,
            _not_send: PhantomData,
        })
    }

    /// The thread pointer: the address of the control block.
    pub fn thread_pointer(&self) -> u64 {
        self.tp
    }

    /// The address of this thread's block for a module's TLS slot.
    pub fn block(&self, slot: TlsSlot) -> u64 {
        self.tp - slot.offset
    }

    /// Reads a word of the control block, such as the stack guard at 0x28.
    pub fn control_word(&self, offset: u64) -> Result<u64> {
        self.region
            .read_u64(self.tp - self.region.address() + offset)
    }

    /// Calls guest code at `f` with up to six integer arguments, in the guest
    /// domain of this thread, and returns `rax`.
    ///
    /// # Safety
    ///
    /// `f` must be guest code of the namespace this thread was made for, taking the
    /// given integer arguments, whose imports are bound to code that honours their
    /// contracts.
    pub unsafe fn call(&self, f: u64, args: &[u64]) -> Result<u64> {
        if !cfg!(target_arch = "x86_64") {
            return Err(Error::ForeignHost);
        }
        let mut regs = [0u64; 6];
        if args.len() > regs.len() {
            return Err(Error::TooManyArguments(args.len()));
        }
        regs[..args.len()].copy_from_slice(args);
        // SAFETY: the block belongs to this thread (the type is not Send) and holds
        // its host TSD base; the caller vouches for f.
        Ok(unsafe { bridge::call_guest(self.tp, f, &regs) })
    }
}
