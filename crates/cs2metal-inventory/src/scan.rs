use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use cs2metal_elf::{Elf, SymbolBinding, SymbolKind, consts};

use crate::{Demand, Family, HostLibrary, Inventory, Module, Needed, Skipped};

/// The scan could not start.
#[derive(Debug)]
pub enum Error {
    NotADirectory(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(p) => write!(f, "{} is not a directory", p.display()),
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

struct Object {
    path: String,
    elf: Elf,
}

/// Scans `root` recursively. Files are opened read-only and only ELF objects are
/// read in full. Symbolic links are not followed into directories; a link to an
/// ELF object inside the tree becomes an extra name for that object, as the
/// loader would find `libfoo.so.1` through `libfoo.so.1 -> libfoo.so.1.2`.
pub fn scan(root: &Path) -> Result<Inventory, Error> {
    let meta = fs::metadata(root).map_err(|e| Error::Io(root.to_owned(), e))?;
    if !meta.is_dir() {
        return Err(Error::NotADirectory(root.to_owned()));
    }
    let mut files = Vec::new();
    let mut links = Vec::new();
    walk(root, root, &mut files, &mut links)?;
    files.sort();

    let mut objects = Vec::new();
    let mut skipped = Vec::new();
    for (relative, path) in files {
        match read_elf(&path) {
            Ok(None) => {}
            Ok(Some(bytes)) => match cs2metal_elf::parse(&bytes) {
                Ok(elf) => objects.push(Object {
                    path: relative,
                    elf,
                }),
                Err(e) => skipped.push(Skipped {
                    path: relative,
                    reason: e.to_string(),
                }),
            },
            Err(e) => skipped.push(Skipped {
                path: relative,
                reason: e.to_string(),
            }),
        }
    }

    let mut aliases = Vec::new();
    for (link_name, target) in links {
        if let Ok(target) = target.strip_prefix(root) {
            aliases.push((link_name, relative_string(target)));
        }
    }
    Ok(resolve(objects, skipped, &aliases))
}

fn walk(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
    links: &mut Vec<(String, PathBuf)>,
) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.to_owned(), e))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| Error::Io(path.clone(), e))?;
        if kind.is_dir() {
            walk(root, &path, files, links)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            files.push((relative_string(relative), path.clone()));
        } else if kind.is_symlink() {
            if let (Ok(target), Some(name)) = (fs::canonicalize(&path), path.file_name()) {
                if target.is_file() {
                    links.push((name.to_string_lossy().into_owned(), target));
                }
            }
        }
    }
    Ok(())
}

fn relative_string(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The whole file if it starts with the ELF magic, without reading other files
/// beyond their first four bytes.
fn read_elf(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let mut file = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    match file.read_exact(&mut magic) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if &magic != b"\x7fELF" {
        return Ok(None);
    }
    let mut bytes = magic.to_vec();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn host_library<'h>(
    host: &'h mut BTreeMap<String, HostLibrary>,
    name: &str,
) -> &'h mut HostLibrary {
    host.entry(name.to_owned()).or_insert_with(|| HostLibrary {
        name: name.to_owned(),
        family: Family::of(name),
        required_by: BTreeSet::new(),
        symbols: BTreeMap::new(),
    })
}

fn add_reference(demand: &mut Demand, version: Option<&str>, weak: bool, module: &str) {
    if demand.referenced_by.is_empty() {
        demand.weak_only = weak;
    } else {
        demand.weak_only &= weak;
    }
    if let Some(v) = version {
        demand.versions.insert(v.to_owned());
    }
    demand.referenced_by.insert(module.to_owned());
}

/// Resolves imports against the bundled modules.
///
/// The loader's global scope is approximated by every bundled module: CS2 opens
/// most of its libraries with `dlopen`, so `DT_NEEDED` closures alone would miss
/// providers. A versioned import is bundled when the library its version names is
/// bundled, or when a bundled module exports the same name (glibc accepts an
/// unversioned definition for a versioned reference from another object, which is
/// how allocators interpose `malloc`).
fn resolve(objects: Vec<Object>, skipped: Vec<Skipped>, aliases: &[(String, String)]) -> Inventory {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        if let Some(soname) = &o.elf.soname {
            by_name.entry(soname.clone()).or_insert(i);
        }
    }
    for (i, o) in objects.iter().enumerate() {
        by_name.entry(base_name(&o.path).to_owned()).or_insert(i);
    }
    for (alias, target) in aliases {
        if let Some(i) = objects.iter().position(|o| &o.path == target) {
            by_name.entry(alias.clone()).or_insert(i);
        }
    }

    let mut exporters: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, o) in objects.iter().enumerate() {
        for s in o.elf.exports() {
            exporters.entry(s.name.as_str()).or_default().push(i);
        }
    }

    let mut inventory = Inventory {
        skipped,
        ..Inventory::default()
    };
    let mut host: BTreeMap<String, HostLibrary> = BTreeMap::new();
    for (index, o) in objects.iter().enumerate() {
        let elf = &o.elf;
        let needed: Vec<Needed> = elf
            .needed
            .iter()
            .map(|name| Needed {
                name: name.clone(),
                bundled: by_name.get(name).map(|&i| objects[i].path.clone()),
            })
            .collect();
        for n in needed.iter().filter(|n| n.bundled.is_none()) {
            host_library(&mut host, &n.name)
                .required_by
                .insert(o.path.clone());
        }

        for import in elf.imports() {
            let weak = import.binding == SymbolBinding::Weak;
            let version = import.version.as_ref();
            let provided_elsewhere = exporters
                .get(import.name.as_str())
                .is_some_and(|list| list.iter().any(|&e| e != index));
            match version.and_then(|v| v.library.as_deref().map(|l| (v, l))) {
                Some((v, library)) => {
                    let library_bundled = by_name.contains_key(library);
                    if library_bundled || provided_elsewhere {
                        if !library_bundled {
                            let exporter = exporters[import.name.as_str()]
                                .iter()
                                .find(|&&e| e != index)
                                .map(|&e| objects[e].path.clone());
                            if let Some(exporter) = exporter {
                                inventory
                                    .interposed
                                    .entry(import.name.clone())
                                    .or_insert(exporter);
                            }
                        }
                        continue;
                    }
                    let lib = host_library(&mut host, library);
                    lib.required_by.insert(o.path.clone());
                    let demand = lib.symbols.entry(import.name.clone()).or_default();
                    add_reference(demand, Some(&v.name), weak, &o.path);
                }
                None => {
                    if provided_elsewhere {
                        continue;
                    }
                    let demand = inventory
                        .unattributed
                        .entry(import.name.clone())
                        .or_default();
                    add_reference(demand, None, weak, &o.path);
                }
            }
        }

        for (&kind, &count) in &elf.relocations.counts {
            let name = consts::x86_64_relocation_name(kind)
                .map_or_else(|| format!("type {kind}"), str::to_owned);
            *inventory.relocations.entry(name).or_default() += count;
        }
        if elf.relocations.relr_words > 0 {
            *inventory
                .relocations
                .entry("RELR words".to_owned())
                .or_default() += elf.relocations.relr_words;
        }
        if let Some(interp) = &elf.interpreter {
            inventory.interpreters.insert(interp.clone());
        }

        inventory.modules.push(Module {
            path: o.path.clone(),
            soname: elf.soname.clone(),
            object_type: elf.object_type,
            machine: elf.machine_name().to_owned(),
            pie: elf.is_pie(),
            needed,
            imports: elf.imports().count(),
            exports: elf.exports().count(),
            ifunc_exports: elf
                .exports()
                .filter(|s| s.kind == SymbolKind::GnuIfunc)
                .count(),
            uses_tls: elf.tls.is_some() || elf.flags.static_tls,
            static_tls: elf.flags.static_tls,
            copy_relocations: elf.relocations.copy_symbols.clone(),
            text_relocations: elf.flags.text_relocations,
        });
    }

    inventory.host_libraries = host.into_values().collect();
    inventory
        .host_libraries
        .sort_by(|a, b| (a.family, &a.name).cmp(&(b.family, &b.name)));
    inventory
}

/// The bundled modules reachable from `start` through `DT_NEEDED`, in load order.
/// Exposed for the loader milestone; the inventory itself uses the global scope.
pub fn needed_closure(inventory: &Inventory, start: &str) -> Vec<String> {
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(path) = queue.pop_front() {
        if !seen.insert(path.clone()) {
            continue;
        }
        if let Some(module) = inventory.module(&path) {
            queue.extend(module.needed.iter().filter_map(|n| n.bundled.clone()));
            order.push(path);
        }
    }
    order
}
