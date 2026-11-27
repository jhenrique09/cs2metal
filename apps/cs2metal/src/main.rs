//! `cs2metal`: command-line front end. Every command here is read-only with
//! respect to the game installation.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cs2metal_elf::{Elf, consts};

#[derive(Debug, Parser)]
#[command(
    name = "cs2metal",
    version,
    about = "Run the CS2 Linux build on Apple silicon"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Describe one ELF object: identity, segments, dependencies, imports, relocations.
    Inspect {
        file: PathBuf,
        /// Print the full parse as JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// Scan an installation and list what the host must provide.
    Inventory {
        game_dir: PathBuf,
        /// Write the full inventory as JSON to this file.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Write the Markdown report to this file instead of standard output.
        #[arg(long)]
        markdown: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("cs2metal: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Inspect { file, json } => {
            let bytes = fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let elf =
                cs2metal_elf::parse(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
            if json {
                let text = serde_json::to_string_pretty(&elf).map_err(|e| e.to_string())?;
                println!("{text}");
            } else {
                print!("{}", summary(&elf));
            }
            Ok(())
        }
        Command::Inventory {
            game_dir,
            json,
            markdown,
        } => {
            let inventory = cs2metal_inventory::scan(&game_dir).map_err(|e| e.to_string())?;
            if let Some(path) = json {
                let text = serde_json::to_string_pretty(&inventory).map_err(|e| e.to_string())?;
                write_report(&path, &text)?;
            }
            let report = cs2metal_inventory::markdown(&inventory);
            match markdown {
                Some(path) => write_report(&path, &report)?,
                None => print!("{report}"),
            }
            eprintln!(
                "cs2metal: {} modules, {} host libraries, {} host symbols",
                inventory.modules.len(),
                inventory.host_libraries.len(),
                inventory.required_symbol_count()
            );
            Ok(())
        }
    }
}

fn write_report(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn summary(elf: &Elf) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "type:        {:?}{}",
        elf.object_type,
        if elf.is_pie() { " (PIE)" } else { "" }
    );
    let _ = writeln!(w, "machine:     {}", elf.machine_name());
    let _ = writeln!(w, "entry:       {:#x}", elf.entry);
    if let Some((lo, hi)) = elf.load_span() {
        let _ = writeln!(w, "load span:   {lo:#x}..{hi:#x}");
    }
    if let Some(interp) = &elf.interpreter {
        let _ = writeln!(w, "interpreter: {interp}");
    }
    if let Some(soname) = &elf.soname {
        let _ = writeln!(w, "soname:      {soname}");
    }
    for path in elf.rpath.iter().chain(&elf.runpath) {
        let _ = writeln!(w, "search path: {path}");
    }
    if let Some(tls) = &elf.tls {
        let _ = writeln!(
            w,
            "tls:         {} bytes initialized, {} per thread, align {}",
            tls.file_size, tls.memory_size, tls.align
        );
    }
    let _ = writeln!(
        w,
        "flags:       bind_now={} static_tls={} textrel={}",
        elf.flags.bind_now, elf.flags.static_tls, elf.flags.text_relocations
    );

    let _ = writeln!(w, "\nsegments:");
    for s in &elf.segments {
        let _ = writeln!(
            w,
            "  {:<13} {} vaddr {:#012x} file {:#x} mem {:#x}",
            s.name(),
            s.permissions(),
            s.vaddr,
            s.file_size,
            s.memory_size
        );
    }

    let _ = writeln!(w, "\nneeded:");
    for n in &elf.needed {
        let _ = writeln!(w, "  {n}");
    }

    let mut by_library: BTreeMap<&str, usize> = BTreeMap::new();
    for import in elf.imports() {
        let library = import
            .version
            .as_ref()
            .and_then(|v| v.library.as_deref())
            .unwrap_or("(unversioned)");
        *by_library.entry(library).or_default() += 1;
    }
    let _ = writeln!(
        w,
        "\nimports: {} ({} exports)",
        elf.imports().count(),
        elf.exports().count()
    );
    for (library, count) in by_library {
        let _ = writeln!(w, "  {library:<24} {count}");
    }

    let _ = writeln!(w, "\nrelocations: {}", elf.relocations.total());
    for (&kind, &count) in &elf.relocations.counts {
        let name = consts::x86_64_relocation_name(kind).unwrap_or("unknown");
        let _ = writeln!(w, "  {name:<24} {count}");
    }
    if elf.relocations.relr_words > 0 {
        let _ = writeln!(w, "  {:<24} {}", "RELR words", elf.relocations.relr_words);
    }
    out
}
