//! Builds small, valid ELF64 objects for tests, so that no test depends on a game
//! file or on a Linux toolchain.
//!
//! The builder lays every table out in one `PT_LOAD` segment whose virtual address is
//! the file offset plus a configurable load bias, and writes the dynamic segment a
//! loader would read. Panics on inconsistent input: it is test code.

use std::collections::BTreeMap;

use crate::consts::*;

/// How the builder lets readers size the dynamic symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashStyle {
    Sysv,
    Gnu,
    /// No hash table at all, only an `SHT_DYNSYM` section header.
    SectionsOnly,
    /// No hash table and no section headers.
    None,
}

#[derive(Debug, Clone)]
enum Version {
    Needed { library: String, version: String },
    Defined { version: String },
}

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    kind: u8,
    binding: u8,
    defined: bool,
    value: u64,
    size: u64,
    version: Option<Version>,
    hidden: bool,
}

#[derive(Debug, Clone)]
struct Reloc {
    kind: u32,
    symbol: Option<String>,
}

/// Machine code, and the `(offset, length)` of each function inside it that gets an
/// FDE in `.eh_frame`.
type Code = (Vec<u8>, Vec<(u64, u64)>);

/// Fluent builder for test objects.
#[derive(Debug, Clone)]
pub struct ElfBuilder {
    object_type: u16,
    machine: u16,
    bias: u64,
    interpreter: Option<String>,
    soname: Option<String>,
    needed: Vec<String>,
    runpath: Option<String>,
    symbols: Vec<Symbol>,
    relocations: Vec<Reloc>,
    plt_relocations: Vec<Reloc>,
    relr_words: u64,
    tls: Option<(u64, u64, u64)>,
    hash: HashStyle,
    init_array_entries: u64,
    flags: u64,
    flags_1: u64,
    code: Option<Code>,
    /// Use a C++-style CIE ("zPLR") with an indirect personality pointer.
    personality: bool,
}

impl ElfBuilder {
    fn new(object_type: u16) -> Self {
        Self {
            object_type,
            machine: EM_X86_64,
            bias: 0,
            interpreter: None,
            soname: None,
            needed: Vec::new(),
            runpath: None,
            symbols: Vec::new(),
            relocations: Vec::new(),
            plt_relocations: Vec::new(),
            relr_words: 0,
            tls: None,
            hash: HashStyle::Gnu,
            init_array_entries: 0,
            flags: 0,
            flags_1: 0,
            code: None,
            personality: false,
        }
    }

    pub fn shared_object(soname: &str) -> Self {
        let mut b = Self::new(ET_DYN);
        b.soname = Some(soname.to_owned());
        b
    }

    /// A position-dependent executable linked at 0x400000.
    pub fn executable() -> Self {
        let mut b = Self::new(ET_EXEC);
        b.bias = 0x40_0000;
        b.interpreter = Some("/lib64/ld-linux-x86-64.so.2".to_owned());
        b
    }

    /// A position-independent executable.
    pub fn pie() -> Self {
        let mut b = Self::new(ET_DYN);
        b.interpreter = Some("/lib64/ld-linux-x86-64.so.2".to_owned());
        b
    }

    pub fn machine(mut self, machine: u16) -> Self {
        self.machine = machine;
        self
    }

    pub fn load_bias(mut self, bias: u64) -> Self {
        self.bias = bias;
        self
    }

    pub fn needed(mut self, library: &str) -> Self {
        self.needed.push(library.to_owned());
        self
    }

    pub fn runpath(mut self, runpath: &str) -> Self {
        self.runpath = Some(runpath.to_owned());
        self
    }

    pub fn hash_style(mut self, style: HashStyle) -> Self {
        self.hash = style;
        self
    }

    pub fn bind_now(mut self) -> Self {
        self.flags |= DF_BIND_NOW;
        self.flags_1 |= DF_1_NOW;
        self
    }

    pub fn static_tls(mut self) -> Self {
        self.flags |= DF_STATIC_TLS;
        self
    }

    pub fn tls(mut self, file_size: u64, memory_size: u64, align: u64) -> Self {
        self.tls = Some((file_size, memory_size, align));
        self
    }

    pub fn init_array(mut self, entries: u64) -> Self {
        self.init_array_entries = entries;
        self
    }

    fn symbol(mut self, symbol: Symbol) -> Self {
        self.symbols.push(symbol);
        self
    }

    /// An undefined global function without a version.
    pub fn import(self, name: &str) -> Self {
        self.import_kind(name, STT_FUNC, STB_GLOBAL)
    }

    pub fn weak_import(self, name: &str) -> Self {
        self.import_kind(name, STT_NOTYPE, STB_WEAK)
    }

    pub fn import_kind(self, name: &str, kind: u8, binding: u8) -> Self {
        self.symbol(Symbol {
            name: name.to_owned(),
            kind,
            binding,
            defined: false,
            value: 0,
            size: 0,
            version: None,
            hidden: false,
        })
    }

    /// An undefined global function bound to `version` of `library`.
    pub fn import_versioned(self, name: &str, library: &str, version: &str) -> Self {
        self.symbol(Symbol {
            name: name.to_owned(),
            kind: STT_FUNC,
            binding: STB_GLOBAL,
            defined: false,
            value: 0,
            size: 0,
            version: Some(Version::Needed {
                library: library.to_owned(),
                version: version.to_owned(),
            }),
            hidden: false,
        })
    }

    /// A defined global symbol without a version.
    pub fn export(self, name: &str, kind: u8) -> Self {
        self.symbol(Symbol {
            name: name.to_owned(),
            kind,
            binding: STB_GLOBAL,
            defined: true,
            value: 0x1000,
            size: 16,
            version: None,
            hidden: false,
        })
    }

    /// A defined global function carrying `version`; `hidden` makes it `name@version`.
    pub fn export_versioned(self, name: &str, version: &str, hidden: bool) -> Self {
        self.symbol(Symbol {
            name: name.to_owned(),
            kind: STT_FUNC,
            binding: STB_GLOBAL,
            defined: true,
            value: 0x1000,
            size: 16,
            version: Some(Version::Defined {
                version: version.to_owned(),
            }),
            hidden,
        })
    }

    pub fn relocation(mut self, kind: u32, symbol: Option<&str>) -> Self {
        self.relocations.push(Reloc {
            kind,
            symbol: symbol.map(str::to_owned),
        });
        self
    }

    pub fn plt_relocation(mut self, kind: u32, symbol: &str) -> Self {
        self.plt_relocations.push(Reloc {
            kind,
            symbol: Some(symbol.to_owned()),
        });
        self
    }

    /// Adds an executable segment holding `bytes`, with an `.eh_frame` FDE and an
    /// `.eh_frame_hdr` entry for each `(offset, length)` in `functions`. Bytes not
    /// covered by any function have no unwind information, as in hand-written
    /// assembly. The rest of the file then loads without execute permission.
    pub fn code(mut self, bytes: &[u8], functions: &[(u64, u64)]) -> Self {
        self.code = Some((bytes.to_vec(), functions.to_vec()));
        self
    }

    /// Makes the CIE of `code` look like a C++ runtime's: augmentation "zPLR",
    /// an indirect pc-relative personality pointer, and an LSDA pointer per FDE.
    pub fn cxx_unwind_tables(mut self) -> Self {
        self.personality = true;
        self
    }

    pub fn relr(mut self, words: u64) -> Self {
        self.relr_words = words;
        self
    }

    pub fn build(&self) -> Vec<u8> {
        Layout::new(self).finish()
    }
}

/// The SysV ELF hash, used in `vd_hash` and `vna_hash`.
fn elf_hash(name: &str) -> u32 {
    let mut h: u32 = 0;
    for b in name.bytes() {
        h = (h << 4).wrapping_add(u32::from(b));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// The GNU hash (Bernstein's djb2 with 33).
fn gnu_hash(name: &str) -> u32 {
    name.bytes().fold(5381u32, |h, b| {
        h.wrapping_mul(33).wrapping_add(u32::from(b))
    })
}

#[derive(Default)]
struct Strings {
    data: Vec<u8>,
    index: BTreeMap<String, u32>,
}

impl Strings {
    fn new() -> Self {
        Self {
            data: vec![0],
            index: BTreeMap::new(),
        }
    }

    fn add(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        if let Some(&at) = self.index.get(s) {
            return at;
        }
        let at = u32::try_from(self.data.len()).expect("string table fits in u32");
        self.data.extend_from_slice(s.as_bytes());
        self.data.push(0);
        self.index.insert(s.to_owned(), at);
        at
    }
}

struct Layout<'b> {
    b: &'b ElfBuilder,
    out: Vec<u8>,
}

impl<'b> Layout<'b> {
    fn new(b: &'b ElfBuilder) -> Self {
        Self { b, out: Vec::new() }
    }

    fn place(&mut self, bytes: &[u8], align: usize) -> u64 {
        while self.out.len() % align != 0 {
            self.out.push(0);
        }
        let at = self.out.len() as u64;
        self.out.extend_from_slice(bytes);
        at
    }

    fn vaddr(&self, offset: u64) -> u64 {
        offset + self.b.bias
    }

    fn finish(mut self) -> Vec<u8> {
        let b = self.b;
        let mut phnum = 3; // LOAD, DYNAMIC, GNU_STACK
        phnum += usize::from(b.interpreter.is_some()) + usize::from(b.tls.is_some());
        phnum += 2 * usize::from(b.code.is_some()); // a code LOAD and GNU_EH_FRAME
        self.out = vec![0; EHDR_SIZE as usize + phnum * PHDR_SIZE as usize];

        // Undefined symbols first: the GNU hash table only covers defined ones.
        let mut symbols: Vec<&Symbol> = b.symbols.iter().filter(|s| !s.defined).collect();
        let undefined = symbols.len();
        symbols.extend(b.symbols.iter().filter(|s| s.defined));
        let nsyms = symbols.len() + 1;
        let sym_index: BTreeMap<&str, u64> = symbols
            .iter()
            .enumerate()
            .map(|(i, s)| (s.name.as_str(), i as u64 + 1))
            .collect();

        // Version indices: 1 is the base definition, then definitions, then needs.
        let mut strings = Strings::new();
        let mut defs: Vec<String> = Vec::new();
        let mut needs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in &symbols {
            match &s.version {
                Some(Version::Defined { version }) if !defs.contains(version) => {
                    defs.push(version.clone());
                }
                Some(Version::Needed { library, version }) => {
                    let list = needs.entry(library.clone()).or_default();
                    if !list.contains(version) {
                        list.push(version.clone());
                    }
                }
                _ => {}
            }
        }
        let mut version_index: BTreeMap<(Option<String>, String), u16> = BTreeMap::new();
        let mut next_index = 2u16;
        for d in &defs {
            version_index.insert((None, d.clone()), next_index);
            next_index += 1;
        }
        for (library, versions) in &needs {
            for v in versions {
                version_index.insert((Some(library.clone()), v.clone()), next_index);
                next_index += 1;
            }
        }
        let has_versions = !version_index.is_empty();

        let interp_off = b
            .interpreter
            .as_ref()
            .map(|i| self.place(format!("{i}\0").as_bytes(), 1));

        let needed: Vec<u32> = b.needed.iter().map(|n| strings.add(n)).collect();
        let soname = b.soname.as_ref().map(|s| strings.add(s));
        let runpath = b.runpath.as_ref().map(|s| strings.add(s));
        let sym_names: Vec<u32> = symbols.iter().map(|s| strings.add(&s.name)).collect();
        let base_name = strings.add(b.soname.as_deref().unwrap_or("a.out"));
        let def_names: Vec<u32> = defs.iter().map(|d| strings.add(d)).collect();
        let need_names: BTreeMap<String, (u32, Vec<u32>)> = needs
            .iter()
            .map(|(lib, versions)| {
                let lib_name = strings.add(lib);
                (
                    lib.clone(),
                    (lib_name, versions.iter().map(|v| strings.add(v)).collect()),
                )
            })
            .collect();

        let strtab_off = self.place(&strings.data, 1);
        let strsz = strings.data.len() as u64;

        let mut dynsym = vec![0u8; SYM_SIZE as usize];
        for (s, &name) in symbols.iter().zip(&sym_names) {
            dynsym.extend_from_slice(&name.to_le_bytes());
            dynsym.push((s.binding << 4) | (s.kind & 0xf));
            dynsym.push(0);
            let shndx: u16 = if s.defined { 7 } else { SHN_UNDEF };
            dynsym.extend_from_slice(&shndx.to_le_bytes());
            dynsym.extend_from_slice(&s.value.to_le_bytes());
            dynsym.extend_from_slice(&s.size.to_le_bytes());
        }
        let symtab_off = self.place(&dynsym, 8);

        let hash_off = match b.hash {
            HashStyle::Sysv => {
                let mut t = Vec::new();
                let nsyms32 = nsyms as u32;
                t.extend_from_slice(&1u32.to_le_bytes());
                t.extend_from_slice(&nsyms32.to_le_bytes());
                t.extend_from_slice(&u32::from(nsyms > 1).to_le_bytes());
                for i in 0..nsyms32 {
                    let next = if i >= 1 && i + 1 < nsyms32 { i + 1 } else { 0 };
                    t.extend_from_slice(&next.to_le_bytes());
                }
                Some((DT_HASH, self.place(&t, 8)))
            }
            HashStyle::Gnu => {
                let symoffset = undefined as u32 + 1;
                let defined: Vec<&&Symbol> = symbols[undefined..].iter().collect();
                let mut t = Vec::new();
                t.extend_from_slice(&1u32.to_le_bytes()); // nbuckets
                t.extend_from_slice(&symoffset.to_le_bytes());
                t.extend_from_slice(&1u32.to_le_bytes()); // bloom words
                t.extend_from_slice(&6u32.to_le_bytes()); // bloom shift
                t.extend_from_slice(&u64::MAX.to_le_bytes());
                let bucket = if defined.is_empty() { 0 } else { symoffset };
                t.extend_from_slice(&bucket.to_le_bytes());
                for (i, s) in defined.iter().enumerate() {
                    let mut h = gnu_hash(&s.name) & !1;
                    if i + 1 == defined.len() {
                        h |= 1;
                    }
                    t.extend_from_slice(&h.to_le_bytes());
                }
                Some((DT_GNU_HASH, self.place(&t, 8)))
            }
            HashStyle::SectionsOnly | HashStyle::None => None,
        };

        let versym_off = has_versions.then(|| {
            let mut t = vec![0u8; 2];
            for s in &symbols {
                let mut index = match &s.version {
                    Some(Version::Defined { version }) => version_index[&(None, version.clone())],
                    Some(Version::Needed { library, version }) => {
                        version_index[&(Some(library.clone()), version.clone())]
                    }
                    None => 1,
                };
                if s.hidden {
                    index |= VERSYM_HIDDEN;
                }
                t.extend_from_slice(&index.to_le_bytes());
            }
            self.place(&t, 2)
        });

        let verdef_off = (!defs.is_empty()).then(|| {
            let mut t = Vec::new();
            let entries: Vec<(u16, u16, u32, String)> =
                std::iter::once((VER_FLG_BASE, 1u16, base_name, String::new()))
                    .chain(
                        defs.iter()
                            .zip(&def_names)
                            .map(|(d, &n)| (0, version_index[&(None, d.clone())], n, d.clone())),
                    )
                    .collect();
            for (i, (flags, index, name, text)) in entries.iter().enumerate() {
                let last = i + 1 == entries.len();
                t.extend_from_slice(&1u16.to_le_bytes());
                t.extend_from_slice(&flags.to_le_bytes());
                t.extend_from_slice(&index.to_le_bytes());
                t.extend_from_slice(&1u16.to_le_bytes());
                t.extend_from_slice(&elf_hash(text).to_le_bytes());
                t.extend_from_slice(&20u32.to_le_bytes());
                t.extend_from_slice(&(if last { 0u32 } else { 28 }).to_le_bytes());
                t.extend_from_slice(&name.to_le_bytes());
                t.extend_from_slice(&0u32.to_le_bytes());
            }
            (self.place(&t, 8), entries.len() as u64)
        });

        let verneed_off = (!needs.is_empty()).then(|| {
            let mut t = Vec::new();
            let count = need_names.len();
            for (i, (library, (lib_name, version_names))) in need_names.iter().enumerate() {
                let n = version_names.len() as u32;
                let last = i + 1 == count;
                t.extend_from_slice(&1u16.to_le_bytes());
                t.extend_from_slice(&(n as u16).to_le_bytes());
                t.extend_from_slice(&lib_name.to_le_bytes());
                t.extend_from_slice(&16u32.to_le_bytes());
                t.extend_from_slice(&(if last { 0 } else { 16 + 16 * n }).to_le_bytes());
                for (j, (&vn, v)) in version_names.iter().zip(&needs[library]).enumerate() {
                    let index = version_index[&(Some(library.clone()), v.clone())];
                    t.extend_from_slice(&elf_hash(v).to_le_bytes());
                    t.extend_from_slice(&0u16.to_le_bytes());
                    t.extend_from_slice(&index.to_le_bytes());
                    t.extend_from_slice(&vn.to_le_bytes());
                    let aux_last = j + 1 == version_names.len();
                    t.extend_from_slice(&(if aux_last { 0u32 } else { 16 }).to_le_bytes());
                }
            }
            (self.place(&t, 8), count as u64)
        });

        let rela_table = |relocs: &[Reloc]| -> Vec<u8> {
            let mut t = Vec::new();
            for (i, r) in relocs.iter().enumerate() {
                let sym = r.symbol.as_deref().map_or(0, |n| sym_index[n]);
                t.extend_from_slice(&(0x2000 + 8 * i as u64).to_le_bytes());
                t.extend_from_slice(&((sym << 32) | u64::from(r.kind)).to_le_bytes());
                t.extend_from_slice(&0u64.to_le_bytes());
            }
            t
        };
        let rela = (!b.relocations.is_empty()).then(|| {
            let t = rela_table(&b.relocations);
            (self.place(&t, 8), t.len() as u64)
        });
        let jmprel = (!b.plt_relocations.is_empty()).then(|| {
            let t = rela_table(&b.plt_relocations);
            (self.place(&t, 8), t.len() as u64)
        });
        let relr = (b.relr_words > 0).then(|| {
            let t = vec![0u8; 8 * b.relr_words as usize];
            (self.place(&t, 8), t.len() as u64)
        });
        let init_array = (b.init_array_entries > 0).then(|| {
            let t = vec![0u8; 8 * b.init_array_entries as usize];
            (self.place(&t, 8), t.len() as u64)
        });
        let tls = b.tls.map(|(file_size, memory_size, align)| {
            let at = self.place(&vec![0xab; file_size as usize], align.max(1) as usize);
            (at, file_size, memory_size, align)
        });

        let code = b.code.as_ref().map(|(bytes, functions)| {
            let text = self.place(bytes, 16);
            while self.out.len() % 8 != 0 {
                self.out.push(0);
            }
            let frame = self.out.len() as u64;
            // A record is a 4-byte length and a body padded with DW_CFA_nop (0)
            // to a multiple of 4.
            let record = |t: &mut Vec<u8>, mut body: Vec<u8>| {
                while body.len() % 4 != 0 {
                    body.push(0);
                }
                t.extend_from_slice(&(body.len() as u32).to_le_bytes());
                t.extend_from_slice(&body);
            };
            // CIE: version 1; FDE pointers pcrel | sdata4 (0x1b).
            let mut cie = vec![0u8, 0, 0, 0, 1];
            if b.personality {
                cie.extend_from_slice(b"zPLR\0");
                cie.extend_from_slice(&[1, 0x78, 16, 7, 0x9b]);
                cie.extend_from_slice(&0x1234i32.to_le_bytes());
                cie.extend_from_slice(&[0x1b, 0x1b]);
            } else {
                cie.extend_from_slice(b"zR\0");
                cie.extend_from_slice(&[1, 0x78, 16, 1, 0x1b]);
            }
            let mut t: Vec<u8> = Vec::new();
            record(&mut t, cie);
            let mut fdes = Vec::new();
            for &(offset, length) in functions {
                let start = frame + t.len() as u64;
                let cie_pointer = (start + 4 - frame) as u32;
                let pc_begin = (text + offset) as i64 - (start + 8) as i64;
                let mut body = Vec::new();
                body.extend_from_slice(&cie_pointer.to_le_bytes());
                body.extend_from_slice(&(pc_begin as i32).to_le_bytes());
                body.extend_from_slice(&(length as u32).to_le_bytes());
                if b.personality {
                    body.push(4);
                    body.extend_from_slice(&0i32.to_le_bytes()); // LSDA
                } else {
                    body.push(0);
                }
                record(&mut t, body);
                fdes.push((text + offset, start));
            }
            t.extend_from_slice(&0u32.to_le_bytes());
            self.place(&t, 8);

            fdes.sort();
            let hdr = self.out.len() as u64;
            let mut h = vec![1u8, 0x1b, 0x03, 0x3b];
            h.extend_from_slice(&((frame as i64 - (hdr + 4) as i64) as i32).to_le_bytes());
            h.extend_from_slice(&(fdes.len() as u32).to_le_bytes());
            for (pc, fde) in &fdes {
                h.extend_from_slice(&((*pc as i64 - hdr as i64) as i32).to_le_bytes());
                h.extend_from_slice(&((*fde as i64 - hdr as i64) as i32).to_le_bytes());
            }
            self.place(&h, 4);
            (text, bytes.len() as u64, hdr, h.len() as u64)
        });

        let mut dynamic: Vec<(u64, u64)> = Vec::new();
        dynamic.extend(needed.iter().map(|&n| (DT_NEEDED, u64::from(n))));
        if let Some(n) = soname {
            dynamic.push((DT_SONAME, u64::from(n)));
        }
        if let Some(n) = runpath {
            dynamic.push((DT_RUNPATH, u64::from(n)));
        }
        if let Some((tag, off)) = hash_off {
            dynamic.push((tag, self.vaddr(off)));
        }
        dynamic.push((DT_STRTAB, self.vaddr(strtab_off)));
        dynamic.push((DT_STRSZ, strsz));
        dynamic.push((DT_SYMTAB, self.vaddr(symtab_off)));
        dynamic.push((DT_SYMENT, SYM_SIZE));
        if let Some(off) = versym_off {
            dynamic.push((DT_VERSYM, self.vaddr(off)));
        }
        if let Some((off, n)) = verdef_off {
            dynamic.push((DT_VERDEF, self.vaddr(off)));
            dynamic.push((DT_VERDEFNUM, n));
        }
        if let Some((off, n)) = verneed_off {
            dynamic.push((DT_VERNEED, self.vaddr(off)));
            dynamic.push((DT_VERNEEDNUM, n));
        }
        if let Some((off, size)) = rela {
            dynamic.push((DT_RELA, self.vaddr(off)));
            dynamic.push((DT_RELASZ, size));
            dynamic.push((DT_RELAENT, RELA_SIZE));
        }
        if let Some((off, size)) = jmprel {
            dynamic.push((DT_JMPREL, self.vaddr(off)));
            dynamic.push((DT_PLTRELSZ, size));
            dynamic.push((DT_PLTREL, DT_RELA));
        }
        if let Some((off, size)) = relr {
            dynamic.push((DT_RELR, self.vaddr(off)));
            dynamic.push((DT_RELRSZ, size));
            dynamic.push((DT_RELRENT, 8));
        }
        if let Some((off, size)) = init_array {
            dynamic.push((DT_INIT_ARRAY, self.vaddr(off)));
            dynamic.push((DT_INIT_ARRAYSZ, size));
        }
        if b.flags != 0 {
            dynamic.push((DT_FLAGS, b.flags));
        }
        if b.flags_1 != 0 {
            dynamic.push((DT_FLAGS_1, b.flags_1));
        }
        dynamic.push((DT_NULL, 0));
        let dyn_bytes: Vec<u8> = dynamic
            .iter()
            .flat_map(|(t, v)| t.to_le_bytes().into_iter().chain(v.to_le_bytes()))
            .collect();
        let dynamic_off = self.place(&dyn_bytes, 8);

        let (shoff, shnum) = if b.hash == HashStyle::SectionsOnly {
            let mut t = vec![0u8; SHDR_SIZE as usize];
            let mut s = [0u8; SHDR_SIZE as usize];
            s[4..8].copy_from_slice(&SHT_DYNSYM.to_le_bytes());
            s[16..24].copy_from_slice(&self.vaddr(symtab_off).to_le_bytes());
            s[24..32].copy_from_slice(&symtab_off.to_le_bytes());
            s[32..40].copy_from_slice(&(dynsym.len() as u64).to_le_bytes());
            s[56..64].copy_from_slice(&SYM_SIZE.to_le_bytes());
            t.extend_from_slice(&s);
            (self.place(&t, 8), 2u16)
        } else {
            (0, 0)
        };

        let total = self.out.len() as u64;
        let mut phdrs: Vec<[u64; 7]> = Vec::new(); // kind|flags, offset, vaddr, paddr, filesz, memsz, align
        let ph = |kind: u32, flags: u32, off: u64, filesz: u64, memsz: u64, align: u64| {
            [
                u64::from(kind) | (u64::from(flags) << 32),
                off,
                off + b.bias,
                off + b.bias,
                filesz,
                memsz,
                align,
            ]
        };
        if let Some(off) = interp_off {
            let len = b.interpreter.as_ref().map_or(0, |i| i.len() as u64 + 1);
            phdrs.push(ph(PT_INTERP, PF_R, off, len, len, 1));
        }
        let main_flags = if code.is_some() { PF_R } else { PF_R | PF_X };
        phdrs.push(ph(PT_LOAD, main_flags, 0, total, total, 0x1000));
        if let Some((text, len, _, _)) = code {
            phdrs.push(ph(PT_LOAD, PF_R | PF_X, text, len, len, 16));
        }
        phdrs.push(ph(
            PT_DYNAMIC,
            PF_R | PF_W,
            dynamic_off,
            dyn_bytes.len() as u64,
            dyn_bytes.len() as u64,
            8,
        ));
        if let Some((off, filesz, memsz, align)) = tls {
            phdrs.push(ph(PT_TLS, PF_R, off, filesz, memsz, align));
        }
        if let Some((_, _, hdr, len)) = code {
            phdrs.push(ph(PT_GNU_EH_FRAME, PF_R, hdr, len, len, 4));
        }
        phdrs.push(ph(PT_GNU_STACK, PF_R | PF_W, 0, 0, 0, 16));
        for (i, p) in phdrs.iter().enumerate() {
            let at = EHDR_SIZE as usize + i * PHDR_SIZE as usize;
            for (j, word) in p.iter().enumerate() {
                self.out[at + j * 8..at + j * 8 + 8].copy_from_slice(&word.to_le_bytes());
            }
        }

        let entry = if b.interpreter.is_some() {
            b.bias + 0x1000
        } else {
            0
        };
        let h = &mut self.out[..EHDR_SIZE as usize];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = ELFCLASS64;
        h[5] = ELFDATA2LSB;
        h[6] = EV_CURRENT;
        h[16..18].copy_from_slice(&b.object_type.to_le_bytes());
        h[18..20].copy_from_slice(&b.machine.to_le_bytes());
        h[20..24].copy_from_slice(&1u32.to_le_bytes());
        h[24..32].copy_from_slice(&entry.to_le_bytes());
        h[32..40].copy_from_slice(&EHDR_SIZE.to_le_bytes());
        h[40..48].copy_from_slice(&shoff.to_le_bytes());
        h[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
        h[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
        h[56..58].copy_from_slice(&(phdrs.len() as u16).to_le_bytes());
        let shentsize = if shnum > 0 { SHDR_SIZE as u16 } else { 0 };
        h[58..60].copy_from_slice(&shentsize.to_le_bytes());
        h[60..62].copy_from_slice(&shnum.to_le_bytes());
        self.out
    }
}
