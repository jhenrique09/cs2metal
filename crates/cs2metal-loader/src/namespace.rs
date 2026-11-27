//! Modules loaded together, with one global scope, as glibc's main namespace.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use cs2metal_elf::{DynamicSymbol, SymbolBinding};

use crate::error::{Error, Result};
use crate::image::{Image, Import};

/// One loaded module.
#[derive(Debug)]
pub struct Module {
    /// The name other modules need it by: its soname, or its file name.
    pub name: String,
    pub path: PathBuf,
    pub image: Image,
}

/// Modules in load order, which is the order of the global lookup scope.
#[derive(Debug, Default)]
pub struct Namespace {
    modules: Vec<Module>,
    /// Searched after a module's `DT_RPATH` and before its `DT_RUNPATH`, like
    /// `LD_LIBRARY_PATH`.
    search: Vec<PathBuf>,
    /// Libraries the host provides itself (the C library, the substitutions):
    /// `DT_NEEDED` entries naming them are not loaded, and imports nothing in the
    /// scope defines go to the host resolver.
    host_libraries: BTreeSet<String>,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// Expands `$ORIGIN` and `${ORIGIN}` in a search path entry.
fn expand(entry: &str, origin: &Path) -> PathBuf {
    let origin = origin.to_string_lossy();
    PathBuf::from(
        entry
            .replace("${ORIGIN}", &origin)
            .replace("$ORIGIN", &origin),
    )
}

impl Namespace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a directory searched for needed libraries, like `LD_LIBRARY_PATH`.
    pub fn search(&mut self, dir: impl Into<PathBuf>) -> &mut Self {
        self.search.push(dir.into());
        self
    }

    /// Declares a library the host provides, such as `libc.so.6`.
    pub fn host_library(&mut self, name: &str) -> &mut Self {
        self.host_libraries.insert(name.to_owned());
        self
    }

    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }

    fn loaded(&self, name: &str, pending: &[Module]) -> bool {
        self.modules.iter().chain(pending).any(|m| m.name == name)
    }

    /// Where glibc would find `name`, needed by the module at `requester`.
    fn find(&self, name: &str, requester: &Module) -> Result<PathBuf> {
        if name.contains('/') {
            return Ok(PathBuf::from(name));
        }
        let elf = requester.image.elf();
        let origin = requester.path.parent().unwrap_or(Path::new("."));
        let rpath: &[String] = if elf.runpath.is_empty() {
            &elf.rpath
        } else {
            &[]
        };
        let dirs = rpath
            .iter()
            .map(|e| expand(e, origin))
            .chain(self.search.iter().cloned())
            .chain(elf.runpath.iter().map(|e| expand(e, origin)));
        dirs.map(|d| d.join(name))
            .find(|p| p.is_file())
            .ok_or_else(|| Error::NotFound {
                name: name.to_owned(),
                needed_by: requester.name.clone(),
            })
    }

    /// Loads the module at `path` and, breadth first, every library it needs that
    /// is neither loaded nor provided by the host. The new modules are relocated
    /// dependencies first and protected; `host` answers imports no module in the
    /// scope defines. Initializers have not run. Returns the module's index.
    pub fn load(
        &mut self,
        path: &Path,
        host: &mut dyn FnMut(Import<'_>) -> Option<u64>,
    ) -> Result<usize> {
        let first = self.modules.len();
        let image = Image::map(path)?;
        let name = image
            .soname()
            .map_or_else(|| file_name(path), str::to_owned);
        if let Some(index) = self.modules.iter().position(|m| m.name == name) {
            return Ok(index);
        }
        let mut pending = vec![Module {
            name,
            path: path.to_owned(),
            image,
        }];
        let mut queue = VecDeque::from([0usize]);
        while let Some(i) = queue.pop_front() {
            let needed = pending[i].image.elf().needed.clone();
            for lib in needed {
                if self.host_libraries.contains(&lib) || self.loaded(&lib, &pending) {
                    continue;
                }
                let found = self.find(&lib, &pending[i])?;
                let image = Image::map(&found)?;
                pending.push(Module {
                    name: lib,
                    path: found,
                    image,
                });
                queue.push_back(pending.len() - 1);
            }
        }
        self.modules.extend(pending);
        let linked = self.link(first, host);
        if linked.is_err() {
            // Nothing of a failed load stays in the scope.
            self.modules.truncate(first);
        }
        linked.map(|()| first)
    }

    /// Relocates and protects the modules from `first` on, dependencies first, so
    /// that IFUNC resolvers find their own imports bound.
    fn link(&self, first: usize, host: &mut dyn FnMut(Import<'_>) -> Option<u64>) -> Result<()> {
        for i in (first..self.modules.len()).rev() {
            let module = &self.modules[i];
            module
                .image
                .relocate(&mut |symbol| self.bind(module, symbol, host))?;
        }
        for module in &self.modules[first..] {
            module.image.protect()?;
        }
        Ok(())
    }

    /// The definition of `name` at `version` earliest in the global scope.
    pub fn lookup(&self, name: &str, version: Option<&str>) -> Option<(&Module, &DynamicSymbol)> {
        self.modules
            .iter()
            .find_map(|m| m.image.find(name, version).map(|s| (m, s)))
    }

    /// The address a reference from `from` to `symbol` binds to.
    fn bind(
        &self,
        from: &Module,
        symbol: &DynamicSymbol,
        host: &mut dyn FnMut(Import<'_>) -> Option<u64>,
    ) -> Result<u64> {
        let version = symbol.version.as_ref().map(|v| v.name.as_str());
        if let Some((module, def)) = self.lookup(&symbol.name, version) {
            return module.image.definition(def);
        }
        let weak = symbol.binding == SymbolBinding::Weak;
        let import = Import {
            name: &symbol.name,
            version,
            weak,
        };
        match host(import) {
            Some(address) => Ok(address),
            None if weak => Ok(0),
            None => Err(Error::UnresolvedIn {
                name: symbol.name.clone(),
                module: from.name.clone(),
            }),
        }
    }

    /// The address of `name` at its default version, earliest in the global scope.
    pub fn symbol(&self, name: &str) -> Option<u64> {
        let (module, def) = self.lookup(name, None)?;
        module.image.definition(def).ok()
    }

    /// Runs the initializers of every module that has not run them, dependencies
    /// first (the reverse of load order), as glibc does for a breadth-first load.
    ///
    /// # Safety
    ///
    /// As for [`Image::run_initializers`], for every module.
    pub unsafe fn run_initializers(&mut self) -> Result<()> {
        for module in self.modules.iter_mut().rev() {
            // SAFETY: the caller upholds the contract for every module.
            unsafe { module.image.run_initializers()? };
        }
        Ok(())
    }

    /// Runs finalizers in load order, the reverse of initialization.
    ///
    /// # Safety
    ///
    /// As for [`Image::run_finalizers`], for every module.
    pub unsafe fn run_finalizers(&mut self) -> Result<()> {
        for module in &mut self.modules {
            // SAFETY: the caller upholds the contract for every module.
            unsafe { module.image.run_finalizers()? };
        }
        Ok(())
    }
}
