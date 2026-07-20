#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod logging;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use humansize::{DECIMAL, format_size};
use indicatif::{ProgressBar, ProgressStyle};
use squeezeit::batch;
use squeezeit::{
    BackupVault, GpuContext, ProcessingBackend, Quality, SqueezeReport, SqueezeSettings,
    SqueezerRegistry,
};
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;

use logging::BarLayer;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum QualityArg {
    Fast,
    Normal,
    Slow,
}

impl From<QualityArg> for Quality {
    fn from(q: QualityArg) -> Self {
        match q {
            QualityArg::Fast => Quality::Fast,
            QualityArg::Normal => Quality::Normal,
            QualityArg::Slow => Quality::Slow,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BackendArg {
    Cpu,
    Gpu,
}

impl From<BackendArg> for ProcessingBackend {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Cpu => ProcessingBackend::CPU,
            BackendArg::Gpu => ProcessingBackend::GPU,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "squeezeit", version, about)]
struct Cli {
    #[arg(help = "File or directory to optimize (directories are walked recursively)")]
    path: PathBuf,

    #[arg(
        long,
        default_value_t = 2048,
        help = "Cap the longest texture side; larger textures are downscaled to fit"
    )]
    max_res: u32,

    #[arg(
        long,
        value_enum,
        default_value_t = QualityArg::Normal,
        help = "Encoder effort (higher = better quality, slower; same output size)"
    )]
    quality: QualityArg,

    #[arg(
        long,
        value_enum,
        default_value_t = BackendArg::Cpu,
        help = "Processing backend; gpu falls back to cpu when no adapter is present"
    )]
    backend: BackendArg,

    #[arg(
        long,
        help = "Also downscale normal/specular maps to half their diffuse's size"
    )]
    overdrive: bool,

    #[arg(
        long,
        overrides_with = "no_mipmaps",
        help = "Generate mipmaps for textures missing them [default: on]"
    )]
    mipmaps: bool,

    #[arg(long, overrides_with = "mipmaps", help = "Skip mipmap generation")]
    no_mipmaps: bool,

    #[arg(
        long,
        help = "Keep the original pixel format (resize only, no re-encode)"
    )]
    keep_format: bool,

    #[arg(
        long,
        help = "Convert every image to DDS even when the result is larger"
    )]
    force_convert: bool,

    #[arg(long, help = "Report savings without writing anything to disk")]
    dry_run: bool,

    #[arg(long, help = "Back up originals into a vault before overwriting")]
    backup: bool,

    #[arg(long, help = "Vault location for backups (implies --backup)")]
    backup_dir: Option<PathBuf>,

    #[arg(long, help = "Restore originals from the vault, then exit")]
    restore: bool,

    #[arg(long, help = "Directory of pre-extracted GTA V archive keys")]
    gta_keys_dir: Option<PathBuf>,

    #[arg(long, help = "Extract GTA V archive keys from GTA5.exe")]
    gta_exe: Option<PathBuf>,

    #[arg(short, long, help = "Verbose per-file logging")]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    squeezeit::platform::sustain_background_performance();

    let cli = Cli::parse();

    if !cli.path.exists() {
        bail!("`{}` does not exist", cli.path.display());
    }

    if cli.restore {
        return restore(&cli);
    }

    let settings = SqueezeSettings {
        max_dimension: cli.max_res,
        overdrive: cli.overdrive,
        generate_mipmaps: cli.mipmaps || !cli.no_mipmaps,
        keep_source_format: cli.keep_format,
        force_convert: cli.force_convert,
        quality: cli.quality.into(),
        make_backup: cli.backup || cli.backup_dir.is_some(),
        dry_run: cli.dry_run,
        backend: cli.backend.into(),
        disable_ui_when_processing: false,
        gta_install_path: None,
    };

    let gpu = match settings.backend {
        ProcessingBackend::GPU => match GpuContext::create_standalone() {
            Ok(ctx) => {
                ctx.warm_up();
                Some(Arc::new(ctx))
            }
            Err(e) => {
                eprintln!("warning: GPU initialization failed ({e}); falling back to CPU");
                None
            }
        },
        ProcessingBackend::CPU => None,
    };

    let registry = match load_gta_keys(&cli)? {
        Some(keys) => SqueezerRegistry::with_gta_keys_gpu(keys, gpu),
        None => SqueezerRegistry::with_defaults_gpu(gpu),
    };
    let targets = batch::collect_targets(&cli.path, &registry);
    if targets.is_empty() {
        println!(
            "No optimizable textures found under `{}`.",
            cli.path.display()
        );
        return Ok(());
    }

    println!(
        "Squeezing {} texture(s) under `{}`{}…",
        targets.len(),
        cli.path.display(),
        if cli.dry_run { " (dry run)" } else { "" }
    );

    let bar = ProgressBar::new(targets.len() as u64).with_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )?
        .progress_chars("#>-"),
    );

    tracing_subscriber::registry()
        .with(
            BarLayer {
                bar: bar.clone(),
                verbose: cli.verbose,
            }
            .with_filter(Targets::new().with_target("squeezeit", Level::DEBUG)),
        )
        .init();

    let report = SqueezeReport::new();
    let cancel = AtomicBool::new(false);
    batch::run_targets(
        &targets,
        &cli.path,
        &registry,
        &settings,
        cli.backup_dir.clone(),
        &report,
        &cancel,
    )
    .context("batch run failed")?;

    bar.finish_and_clear();
    print_summary(&report, cli.dry_run);
    Ok(())
}

fn load_gta_keys(cli: &Cli) -> anyhow::Result<Option<Arc<squeezeit::rpf::GtaKeys>>> {
    if let Some(exe) = &cli.gta_exe {
        let keys = squeezeit::rpf::GtaKeys::extract_from_exe(exe, cli.gta_keys_dir.as_deref())
            .context("key extraction from GTA5.exe failed")?;
        return Ok(Some(Arc::new(keys)));
    }
    if let Some(dir) = &cli.gta_keys_dir {
        let keys = squeezeit::rpf::GtaKeys::load_from_path(dir)
            .context("loading pre-extracted GTA keys failed")?;
        return Ok(Some(Arc::new(keys)));
    }
    Ok(None)
}

fn restore(cli: &Cli) -> anyhow::Result<()> {
    let vault = BackupVault::new(&cli.path, cli.backup_dir.clone())
        .context("failed to open the backup vault")?;
    let vault_dir = vault.backup_root().to_path_buf();
    let restored = vault
        .restore_all()
        .context("restore failed — the vault and manifest are untouched")?;
    println!(
        "Restored {restored} file(s) from `{}`.",
        vault_dir.display()
    );
    Ok(())
}

fn print_summary(report: &SqueezeReport, dry_run: bool) {
    let s = report.snapshot();
    println!();
    println!("── SqueezeIt summary ─────────────────────────");
    println!("  optimized : {}", s.optimized);
    println!("  locked    : {}  (script_rt safety)", s.locked);
    println!("  skipped   : {}", s.skipped);
    println!("  failed    : {}", s.failed);
    println!(
        "  throughput: {:.1} files/s  ({:.1}s elapsed)",
        s.files_per_sec(),
        s.elapsed_secs()
    );
    println!(
        "  bytes     : {} -> {}",
        format_size(s.bytes_before, DECIMAL),
        format_size(s.bytes_after, DECIMAL),
    );
    let (gpu_compressed, gpu_busy, gpu_failures) = squeezeit::gpu::counters();
    if gpu_compressed > 0 || gpu_busy > 0 || gpu_failures > 0 {
        println!(
            "  gpu       : {gpu_compressed} textures compressed, {gpu_busy} routed to CPU (busy), {gpu_failures} failed to CPU"
        );
    }
    println!(
        "  saved     : {} ({:.1}% streaming budget relief){}",
        format_size(s.bytes_saved(), DECIMAL),
        s.percent_saved(),
        if dry_run {
            "  [dry run — nothing written]"
        } else {
            ""
        },
    );
}
