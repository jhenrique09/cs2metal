use std::collections::BTreeMap;

use crate::consts::*;
use crate::error::{Error, Result};
use crate::reader::{Reader, offset_of};
use crate::{
    DynamicFlags, DynamicSymbol, Elf, Initializers, ObjectType, Relocations, Segment,
    SymbolBinding, SymbolKind, SymbolVersion, TlsTemplate, VersionNeed,
};

/// Upper bound on linked-list walks (version tables, GNU hash chains) so that a
/// corrupted `next` field cannot loop forever.
const MAX_WALK: u64 = 1 << 24;

/// Parses an ELF64 little-endian object.
pub fn parse(data: &[u8]) -> Result<Elf> {
    let r = Reader::new(data);
    if r.bytes(0, 4).map_err(|_| Error::NotElf)? != b"\x7fELF" {
        return Err(Error::NotElf);
    }
    let class = r.u8(4)?;
    if class != ELFCLASS64 {
        return Err(Error::UnsupportedClass(class));
    }
    let encoding = r.u8(5)?;
    if encoding != ELFDATA2LSB {
        return Err(Error::UnsupportedEncoding(encoding));
    }
    let version = r.u8(6)?;
    if version != EV_CURRENT {
        return Err(Error::UnsupportedVersion(version));
    }
    r.bytes(0, EHDR_SIZE)?;

    let object_type = match r.u16(16)? {
        ET_REL => ObjectType::Relocatable,
        ET_EXEC => ObjectType::Executable,
        ET_DYN => ObjectType::SharedObject,
        ET_CORE => ObjectType::Core,
        other => ObjectType::Other(other),
    };
    let machine = r.u16(18)?;
    let entry = r.u64(24)?;
    let segments = program_headers(&r)?;
    let map = AddressMap {
        segments: &segments,
    };

    let interpreter = match segments.iter().find(|s| s.kind == PT_INTERP) {
        Some(s) => Some(r.cstr(s.offset)?),
        None => None,
    };
    let tls = segments
        .iter()
        .find(|s| s.kind == PT_TLS)
        .map(|s| TlsTemplate {
            vaddr: s.vaddr,
            file_size: s.file_size,
            memory_size: s.memory_size,
            align: s.align,
        });

    let mut elf = Elf {
        object_type,
        machine,
        entry,
        interpreter,
        dynamic: false,
        soname: None,
        needed: Vec::new(),
        rpath: Vec::new(),
        runpath: Vec::new(),
        tls,
        symbols: Vec::new(),
        version_needs: Vec::new(),
        version_definitions: Vec::new(),
        relocations: Relocations::default(),
        initializers: Initializers::default(),
        flags: DynamicFlags::default(),
        segments: Vec::new(),
    };

    if let Some(dynamic) = segments.iter().find(|s| s.kind == PT_DYNAMIC) {
        elf.dynamic = true;
        let tags = dynamic_tags(&r, dynamic)?;
        parse_dynamic(&r, &map, &tags, &mut elf)?;
    }
    elf.segments = segments;
    Ok(elf)
}

fn program_headers(r: &Reader<'_>) -> Result<Vec<Segment>> {
    let phoff = r.u64(32)?;
    let phentsize = u64::from(r.u16(54)?);
    let phnum = u64::from(r.u16(56)?);
    if phnum == 0 {
        return Ok(Vec::new());
    }
    if phentsize != PHDR_SIZE {
        return Err(Error::BadEntrySize {
            table: "program header",
            size: phentsize,
        });
    }
    (0..phnum)
        .map(|i| {
            let at = offset_of(phoff, i, PHDR_SIZE)?;
            Ok(Segment {
                kind: r.u32(at)?,
                flags: r.u32(at + 4)?,
                offset: r.u64(at + 8)?,
                vaddr: r.u64(at + 16)?,
                file_size: r.u64(at + 32)?,
                memory_size: r.u64(at + 40)?,
                align: r.u64(at + 48)?,
            })
        })
        .collect()
}

/// Translates virtual addresses to file offsets through the `PT_LOAD` segments.
struct AddressMap<'s> {
    segments: &'s [Segment],
}

impl AddressMap<'_> {
    fn offset(&self, vaddr: u64) -> Result<u64> {
        self.segments
            .iter()
            .filter(|s| s.kind == PT_LOAD)
            .find(|s| vaddr >= s.vaddr && vaddr - s.vaddr < s.file_size)
            .and_then(|s| s.offset.checked_add(vaddr - s.vaddr))
            .ok_or(Error::UnmappedAddress(vaddr))
    }
}

/// The dynamic tags in file order. Repeated tags (`DT_NEEDED`) keep every value.
struct Tags {
    entries: Vec<(u64, u64)>,
}

impl Tags {
    fn get(&self, tag: u64) -> Option<u64> {
        self.entries
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, v)| *v)
    }

    fn all(&self, tag: u64) -> impl Iterator<Item = u64> + '_ {
        self.entries
            .iter()
            .filter(move |(t, _)| *t == tag)
            .map(|(_, v)| *v)
    }

    fn require(&self, tag: u64, name: &'static str) -> Result<u64> {
        self.get(tag).ok_or(Error::MissingTag(name))
    }
}

fn dynamic_tags(r: &Reader<'_>, dynamic: &Segment) -> Result<Tags> {
    let count = dynamic.file_size / DYN_SIZE;
    let mut entries = Vec::new();
    for i in 0..count {
        let at = offset_of(dynamic.offset, i, DYN_SIZE)?;
        let tag = r.u64(at)?;
        if tag == DT_NULL {
            break;
        }
        entries.push((tag, r.u64(at + 8)?));
    }
    Ok(Tags { entries })
}

fn parse_dynamic(r: &Reader<'_>, map: &AddressMap<'_>, tags: &Tags, elf: &mut Elf) -> Result<()> {
    let strtab = match tags.get(DT_STRTAB) {
        Some(addr) => {
            let size = tags.require(DT_STRSZ, "DT_STRSZ")?;
            Some(Reader::new(r.bytes(map.offset(addr)?, size)?))
        }
        None => None,
    };
    let string = |offset: u64| -> Result<String> {
        strtab
            .as_ref()
            .ok_or(Error::MissingTag("DT_STRTAB"))?
            .cstr(offset)
    };

    elf.needed = tags.all(DT_NEEDED).map(&string).collect::<Result<_>>()?;
    elf.soname = tags.get(DT_SONAME).map(&string).transpose()?;
    elf.rpath = split_paths(tags.get(DT_RPATH).map(&string).transpose()?);
    elf.runpath = split_paths(tags.get(DT_RUNPATH).map(&string).transpose()?);

    let flags = tags.get(DT_FLAGS).unwrap_or(0);
    let flags_1 = tags.get(DT_FLAGS_1).unwrap_or(0);
    elf.flags = DynamicFlags {
        bind_now: flags & DF_BIND_NOW != 0
            || flags_1 & DF_1_NOW != 0
            || tags.get(DT_BIND_NOW).is_some(),
        text_relocations: flags & DF_TEXTREL != 0 || tags.get(DT_TEXTREL).is_some(),
        static_tls: flags & DF_STATIC_TLS != 0,
        symbolic: flags & DF_SYMBOLIC != 0 || tags.get(DT_SYMBOLIC).is_some(),
    };
    elf.initializers = Initializers {
        init: tags.get(DT_INIT),
        fini: tags.get(DT_FINI),
        init_array_entries: tags.get(DT_INIT_ARRAYSZ).unwrap_or(0) / 8,
        fini_array_entries: tags.get(DT_FINI_ARRAYSZ).unwrap_or(0) / 8,
        preinit_array_entries: tags.get(DT_PREINIT_ARRAYSZ).unwrap_or(0) / 8,
    };

    let versions = Versions::parse(r, map, tags, &string)?;
    elf.version_needs = versions.needs.clone();
    elf.version_definitions = versions.definitions.clone();

    let mut symbol_names = Vec::new();
    if let Some(symtab) = tags.get(DT_SYMTAB) {
        let syment = tags.get(DT_SYMENT).unwrap_or(SYM_SIZE);
        if syment != SYM_SIZE {
            return Err(Error::BadEntrySize {
                table: "symbol",
                size: syment,
            });
        }
        let count = symbol_count(r, map, tags)?;
        let base = map.offset(symtab)?;
        let versym = tags.get(DT_VERSYM).map(|a| map.offset(a)).transpose()?;
        // Index 0 is the reserved null symbol.
        for index in 1..count {
            let at = offset_of(base, index, SYM_SIZE)?;
            let info = r.u8(at + 4)?;
            let name = string(u64::from(r.u32(at)?))?;
            let defined = r.u16(at + 6)? != SHN_UNDEF;
            let version = match versym {
                Some(v) => versions.resolve(r.u16(offset_of(v, index, 2)?)?, defined),
                None => None,
            };
            symbol_names.push(name.clone());
            elf.symbols.push(DynamicSymbol {
                name,
                kind: symbol_kind(info & 0xf),
                binding: symbol_binding(info >> 4),
                defined,
                value: r.u64(at + 8)?,
                size: r.u64(at + 16)?,
                version,
            });
        }
    }

    elf.relocations = relocations(r, map, tags, &symbol_names)?;
    Ok(())
}

fn split_paths(value: Option<String>) -> Vec<String> {
    value
        .map(|v| {
            v.split(':')
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn symbol_kind(kind: u8) -> SymbolKind {
    match kind {
        STT_NOTYPE => SymbolKind::NoType,
        STT_OBJECT => SymbolKind::Object,
        STT_FUNC => SymbolKind::Func,
        STT_SECTION => SymbolKind::Section,
        STT_FILE => SymbolKind::File,
        STT_COMMON => SymbolKind::Common,
        STT_TLS => SymbolKind::Tls,
        STT_GNU_IFUNC => SymbolKind::GnuIfunc,
        other => SymbolKind::Other(other),
    }
}

fn symbol_binding(binding: u8) -> SymbolBinding {
    match binding {
        STB_LOCAL => SymbolBinding::Local,
        STB_GLOBAL => SymbolBinding::Global,
        STB_WEAK => SymbolBinding::Weak,
        STB_GNU_UNIQUE => SymbolBinding::GnuUnique,
        other => SymbolBinding::Other(other),
    }
}

/// The number of entries in the dynamic symbol table, including the null entry.
///
/// The dynamic segment does not record it, so it is recovered the way loaders and
/// tools do: `nchain` of the SysV hash table, or the highest index reachable through
/// the GNU hash table. Section headers are the last resort.
fn symbol_count(r: &Reader<'_>, map: &AddressMap<'_>, tags: &Tags) -> Result<u64> {
    if let Some(hash) = tags.get(DT_HASH) {
        return Ok(u64::from(r.u32(map.offset(hash)? + 4)?));
    }
    if let Some(gnu) = tags.get(DT_GNU_HASH) {
        return gnu_hash_symbol_count(r, map.offset(gnu)?);
    }
    section_symbol_count(r)
}

fn gnu_hash_symbol_count(r: &Reader<'_>, at: u64) -> Result<u64> {
    let nbuckets = u64::from(r.u32(at)?);
    let symoffset = u64::from(r.u32(at + 4)?);
    let bloom_words = u64::from(r.u32(at + 8)?);
    let buckets = offset_of(at + 16, bloom_words, 8)?;
    let chains = offset_of(buckets, nbuckets, 4)?;

    let mut highest = 0u64;
    for i in 0..nbuckets {
        highest = highest.max(u64::from(r.u32(offset_of(buckets, i, 4)?)?));
    }
    if highest < symoffset {
        // Every bucket is empty: only the unhashed (undefined) symbols exist.
        return Ok(symoffset);
    }
    // Walk the last non-empty chain to the entry whose low bit marks its end.
    for index in highest..highest.saturating_add(MAX_WALK) {
        let value = r.u32(offset_of(chains, index - symoffset, 4)?)?;
        if value & 1 != 0 {
            return Ok(index + 1);
        }
    }
    Err(Error::UnknownSymbolCount)
}

fn section_symbol_count(r: &Reader<'_>) -> Result<u64> {
    let shoff = r.u64(40)?;
    let shentsize = u64::from(r.u16(58)?);
    let shnum = u64::from(r.u16(60)?);
    if shoff == 0 || shnum == 0 || shentsize != SHDR_SIZE {
        return Err(Error::UnknownSymbolCount);
    }
    for i in 0..shnum {
        let at = offset_of(shoff, i, SHDR_SIZE)?;
        if r.u32(at + 4)? == SHT_DYNSYM {
            let size = r.u64(at + 32)?;
            return Ok(size / SYM_SIZE);
        }
    }
    Err(Error::UnknownSymbolCount)
}

/// GNU symbol versioning: version indices from `DT_VERDEF` and `DT_VERNEED`.
#[derive(Default)]
struct Versions {
    /// Index to version name, for versions this object defines.
    defined: BTreeMap<u16, String>,
    /// Index to (version name, library), for versions this object requires.
    needed: BTreeMap<u16, (String, String)>,
    needs: Vec<VersionNeed>,
    definitions: Vec<String>,
}

impl Versions {
    fn parse(
        r: &Reader<'_>,
        map: &AddressMap<'_>,
        tags: &Tags,
        string: &dyn Fn(u64) -> Result<String>,
    ) -> Result<Self> {
        let mut out = Self::default();
        if let Some(addr) = tags.get(DT_VERDEF) {
            let count = tags.require(DT_VERDEFNUM, "DT_VERDEFNUM")?.min(MAX_WALK);
            let mut at = map.offset(addr)?;
            for _ in 0..count {
                let flags = r.u16(at + 2)?;
                let index = r.u16(at + 4)?;
                let aux = u64::from(r.u32(at + 12)?);
                let next = u64::from(r.u32(at + 16)?);
                let name = string(u64::from(r.u32(at + aux)?))?;
                if flags & VER_FLG_BASE == 0 {
                    out.definitions.push(name.clone());
                }
                out.defined.insert(index & VERSYM_INDEX, name);
                if next == 0 {
                    break;
                }
                at = at.checked_add(next).ok_or(Error::UnknownSymbolCount)?;
            }
        }
        if let Some(addr) = tags.get(DT_VERNEED) {
            let count = tags.require(DT_VERNEEDNUM, "DT_VERNEEDNUM")?.min(MAX_WALK);
            let mut at = map.offset(addr)?;
            for _ in 0..count {
                let aux_count = u64::from(r.u16(at + 2)?);
                let library = string(u64::from(r.u32(at + 4)?))?;
                let mut aux = at + u64::from(r.u32(at + 8)?);
                let next = u64::from(r.u32(at + 12)?);
                let mut versions = Vec::new();
                for _ in 0..aux_count {
                    let index = r.u16(aux + 6)?;
                    let name = string(u64::from(r.u32(aux + 8)?))?;
                    let aux_next = u64::from(r.u32(aux + 12)?);
                    out.needed
                        .insert(index & VERSYM_INDEX, (name.clone(), library.clone()));
                    versions.push(name);
                    if aux_next == 0 {
                        break;
                    }
                    aux = aux.checked_add(aux_next).ok_or(Error::UnknownSymbolCount)?;
                }
                out.needs.push(VersionNeed { library, versions });
                if next == 0 {
                    break;
                }
                at = at.checked_add(next).ok_or(Error::UnknownSymbolCount)?;
            }
        }
        Ok(out)
    }

    fn resolve(&self, versym: u16, defined: bool) -> Option<SymbolVersion> {
        let index = versym & VERSYM_INDEX;
        if index <= 1 {
            // 0: local, 1: global, unversioned.
            return None;
        }
        let hidden = versym & VERSYM_HIDDEN != 0;
        let need = || {
            self.needed
                .get(&index)
                .map(|(name, library)| SymbolVersion {
                    name: name.clone(),
                    library: Some(library.clone()),
                    hidden,
                })
        };
        let def = || {
            self.defined.get(&index).map(|name| SymbolVersion {
                name: name.clone(),
                library: None,
                hidden,
            })
        };
        if defined {
            def().or_else(need)
        } else {
            need().or_else(def)
        }
    }
}

fn relocations(
    r: &Reader<'_>,
    map: &AddressMap<'_>,
    tags: &Tags,
    symbol_names: &[String],
) -> Result<Relocations> {
    if let Some(rel) = tags.get(DT_REL) {
        return Err(Error::UnsupportedRelocationFormat(rel));
    }
    let mut out = Relocations::default();

    let rela = match tags.get(DT_RELA) {
        Some(addr) => {
            let size = tags.require(DT_RELASZ, "DT_RELASZ")?;
            let entsize = tags.get(DT_RELAENT).unwrap_or(RELA_SIZE);
            if entsize != RELA_SIZE {
                return Err(Error::BadEntrySize {
                    table: "relocation",
                    size: entsize,
                });
            }
            Some((addr, size))
        }
        None => None,
    };
    let plt = match tags.get(DT_JMPREL) {
        Some(addr) => {
            let kind = tags.get(DT_PLTREL).unwrap_or(DT_RELA);
            if kind != DT_RELA {
                return Err(Error::UnsupportedRelocationFormat(kind));
            }
            Some((addr, tags.require(DT_PLTRELSZ, "DT_PLTRELSZ")?))
        }
        None => None,
    };
    // Some linkers make DT_RELASZ cover .rela.plt too; count those entries once.
    let plt = plt.filter(|&(p, psize)| match rela {
        Some((a, asize)) => !(p >= a && p.saturating_add(psize) <= a.saturating_add(asize)),
        None => true,
    });

    for (addr, size) in rela.into_iter().chain(plt) {
        let base = map.offset(addr)?;
        for i in 0..size / RELA_SIZE {
            let info = r.u64(offset_of(base, i, RELA_SIZE)? + 8)?;
            let kind = (info & 0xffff_ffff) as u32;
            *out.counts.entry(kind).or_default() += 1;
            if kind == R_X86_64_COPY {
                let symbol = usize::try_from(info >> 32).ok();
                if let Some(name) = symbol
                    .and_then(|s| s.checked_sub(1))
                    .and_then(|s| symbol_names.get(s))
                {
                    out.copy_symbols.push(name.clone());
                }
            }
        }
    }

    if let Some(addr) = tags.get(DT_RELR) {
        let size = tags.require(DT_RELRSZ, "DT_RELRSZ")?;
        let entsize = tags.get(DT_RELRENT).unwrap_or(8);
        if entsize != 8 {
            return Err(Error::BadEntrySize {
                table: "RELR",
                size: entsize,
            });
        }
        r.bytes(map.offset(addr)?, size)?;
        out.relr_words = size / 8;
    }
    Ok(out)
}
