//! `cs2metal`: command-line front end. Every command here is read-only with
//! respect to the game installation.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use cs2metal_elf::{Elf, consts};
use cs2metal_integrity::{Baseline, Mode};

/// Where baselines go by default: an ignored directory of the checkout.
const DEFAULT_BASELINE: &str = "baselines/cs2-linux.json";

/// Exit status of `baseline verify` when the installation differs from its baseline.
const EXIT_MISMATCH: u8 = 4;

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
    /// Record or verify the integrity baseline of an installation.
    Baseline {
        #[command(subcommand)]
        action: BaselineAction,
    },
}

#[derive(Debug, Subcommand)]
enum BaselineAction {
    /// Hash every file of the installation. Run it right after steamcmd's `validate`.
    Record {
        game_dir: PathBuf,
        /// Where to write the baseline.
        #[arg(long, default_value = DEFAULT_BASELINE)]
        output: PathBuf,
        /// Replace an existing baseline.
        #[arg(long)]
        force: bool,
    },
    /// Compare the installation with its baseline. Exits with status 4 on any difference.
    Verify {
        game_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_BASELINE)]
        baseline: PathBuf,
        /// Hash again only files whose size or modification time changed.
        #[arg(long)]
        quick: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("cs2metal: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
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
            Ok(ExitCode::SUCCESS)
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
            Ok(ExitCode::SUCCESS)
        }
        Command::Baseline { action } => baseline(action),
    }
}

fn gigabytes(bytes: u64) -> f64 {
    bytes as f64 / 1e9
}

/// Runs `work` while a second thread reports the bytes hashed every few seconds.
fn with_progress<T>(work: impl FnOnce(&AtomicU64) -> T) -> T {
    let progress = AtomicU64::new(0);
    let done = AtomicBool::new(false);
    let start = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut last = Instant::now();
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                if last.elapsed() >= Duration::from_secs(5) {
                    last = Instant::now();
                    eprintln!(
                        "cs2metal: {:.1} GB hashed ({:.0} s)",
                        gigabytes(progress.load(Ordering::Relaxed)),
                        start.elapsed().as_secs_f64()
                    );
                }
            }
        });
        let result = work(&progress);
        done.store(true, Ordering::Relaxed);
        result
    })
}

fn baseline(action: BaselineAction) -> Result<ExitCode, String> {
    match action {
        BaselineAction::Record {
            game_dir,
            output,
            force,
        } => {
            if output.exists() && !force {
                return Err(format!(
                    "{} exists; pass --force to replace it",
                    output.display()
                ));
            }
            let start = Instant::now();
            let baseline = with_progress(|p| cs2metal_integrity::record(&game_dir, p))
                .map_err(|e| e.to_string())?;
            write_report(&output, &baseline.to_json())?;
            eprintln!(
                "cs2metal: recorded {} files and {} links, {:.1} GB, in {:.0} s, to {}",
                baseline.files.len(),
                baseline.links.len(),
                gigabytes(baseline.total_bytes()),
                start.elapsed().as_secs_f64(),
                output.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        BaselineAction::Verify {
            game_dir,
            baseline,
            quick,
        } => {
            let text = fs::read_to_string(&baseline)
                .map_err(|e| format!("{}: {e}", baseline.display()))?;
            let recorded = Baseline::from_json(&text).map_err(|e| e.to_string())?;
            let mode = if quick { Mode::Quick } else { Mode::Full };
            let report =
                with_progress(|p| cs2metal_integrity::verify(&game_dir, &recorded, mode, p))
                    .map_err(|e| e.to_string())?;
            const SHOWN: usize = 20;
            for difference in report.differences.iter().take(SHOWN) {
                println!("{difference}");
            }
            if report.differences.len() > SHOWN {
                println!("... and {} more", report.differences.len() - SHOWN);
            }
            eprintln!(
                "cs2metal: {} files checked, {} hashed ({:.1} GB), {} differences",
                report.files_checked,
                report.files_hashed,
                gigabytes(report.bytes_hashed),
                report.differences.len()
            );
            Ok(if report.is_clean() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(EXIT_MISMATCH)
            })
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
