#[cfg(not(windows))]
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
use squeezeit::{
    Backend, BackupVault, GpuContext, GtaKeys, Preset, Quality, SqueezeReport, SqueezeSettings,
    SqueezerRegistry, batch,
};
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;

use logging::BarLayer;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum QualityArg {
    #[value(help = "Near-instant, slightly rougher gradients")]
    Fast,
    #[value(help = "The sensible middle")]
    Normal,
    #[value(help = "Best looking, slowest")]
    Slow,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BackendArg {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PresetArg {
    #[value(help = "Pick the rules per file from its shaders and name")]
    Auto,
    #[value(help = "Force the ped body and outfit rules")]
    Clothing,
    #[value(help = "Force the hair and alpha-card rules")]
    Hair,
    #[value(help = "Hair rules, and no resizing either")]
    HairStrict,
    #[value(help = "Force the vehicle and prop rules")]
    Vehicles,
    #[value(help = "No family rules at all, just the switches below")]
    Custom,
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

impl From<BackendArg> for Backend {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Cpu => Backend::Cpu,
            BackendArg::Gpu => Backend::Gpu,
        }
    }
}

impl From<PresetArg> for Preset {
    fn from(p: PresetArg) -> Self {
        match p {
            PresetArg::Auto => Preset::Auto,
            PresetArg::Clothing => Preset::Clothing,
            PresetArg::Hair => Preset::Hair,
            PresetArg::HairStrict => Preset::HairStrict,
            PresetArg::Vehicles => Preset::VehiclesProps,
            PresetArg::Custom => Preset::Custom,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "squeezeit",
    version,
    about = "Shrink game textures, keep the pixels"
)]
struct Cli {
    #[arg(help = "File or folder to squeeze. Folders are walked all the way down")]
    path: PathBuf,

    #[arg(
        long,
        value_enum,
        default_value_t = PresetArg::Auto,
        help = "Rule set deciding what each asset family may lose"
    )]
    preset: PresetArg,

    #[arg(
        long,
        default_value_t = 2048,
        help = "Biggest side a texture may keep. Bigger ones are halved until they fit"
    )]
    max_res: u32,

    #[arg(
        long,
        value_enum,
        default_value_t = QualityArg::Normal,
        help = "Encoder effort. Same output size either way, only fidelity and time change"
    )]
    quality: QualityArg,

    #[arg(
        long,
        value_enum,
        default_value_t = BackendArg::Cpu,
        help = "Where the work happens. gpu falls back to cpu when there is no adapter"
    )]
    backend: BackendArg,

    #[arg(
        long,
        help = "Also halve normal and specular maps against their paired diffuse"
    )]
    overdrive: bool,

    #[arg(
        long,
        overrides_with = "no_mipmaps",
        help = "Generate mipmaps for textures that ship without them [default: on]"
    )]
    mipmaps: bool,

    #[arg(long, overrides_with = "mipmaps", help = "Skip mipmap generation")]
    no_mipmaps: bool,

    #[arg(
        long,
        help = "Keep the original pixel format. Resize only, no re-encode"
    )]
    keep_format: bool,

    #[arg(
        long,
        help = "Write DDS for every image, even when the DDS comes out bigger"
    )]
    force_convert: bool,

    #[arg(
        long,
        help = "Let clothing textures that pass every safety check grow a mip chain"
    )]
    cloth_mips: bool,

    #[arg(long, help = "Let clothing textures be block-compressed on the GPU")]
    cloth_gpu: bool,

    #[arg(long, help = "Let vehicle textures grow full mip chains")]
    vehicle_mips: bool,

    #[arg(long, help = "Report the savings without writing anything")]
    dry_run: bool,

    #[arg(long, help = "Copy originals into a vault before overwriting them")]
    backup: bool,

    #[arg(long, help = "Where that vault lives. Implies --backup")]
    backup_dir: Option<PathBuf>,

    #[arg(long, help = "Put the originals back from the vault, then quit")]
    restore: bool,

    #[arg(long, help = "Folder of already-extracted GTA V archive keys")]
    gta_keys_dir: Option<PathBuf>,

    #[arg(long, help = "Pull the GTA V archive keys straight out of GTA5.exe")]
    gta_exe: Option<PathBuf>,

    #[arg(short, long, help = "Log every file, not just the failures")]
    verbose: bool,
}

impl Cli {
    fn settings(&self) -> SqueezeSettings {
        SqueezeSettings {
            max_dimension: self.max_res,
            preset: self.preset.into(),
            quality: self.quality.into(),
            backend: self.backend.into(),
            overdrive: self.overdrive,
            generate_mipmaps: self.mipmaps || !self.no_mipmaps,
            keep_source_format: self.keep_format,
            force_convert: self.force_convert,
            cloth_mips: self.cloth_mips,
            cloth_gpu: self.cloth_gpu,
            vehicle_mips: self.vehicle_mips,
            make_backup: self.backup || self.backup_dir.is_some(),
            dry_run: self.dry_run,
        }
    }
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

    let settings = cli.settings();
    let gpu = match settings.backend {
        Backend::Gpu => match GpuContext::create_standalone() {
            Ok(ctx) => {
                ctx.warm_up();
                Some(Arc::new(ctx))
            }
            Err(e) => {
                eprintln!("warning: {e}; running on the CPU");
                None
            }
        },
        Backend::Cpu => None,
    };

    let registry = SqueezerRegistry::new(gpu, load_gta_keys(&cli)?);
    let targets = batch::collect_targets(&cli.path, &registry);
    if targets.is_empty() {
        println!("Nothing to squeeze under `{}`.", cli.path.display());
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

fn load_gta_keys(cli: &Cli) -> anyhow::Result<Option<Arc<GtaKeys>>> {
    if let Some(exe) = &cli.gta_exe {
        let keys =
            squeezeit::gta_keys::from_exe(exe).context("key extraction from GTA5.exe failed")?;
        return Ok(Some(Arc::new(keys)));
    }
    if let Some(dir) = &cli.gta_keys_dir {
        let keys = GtaKeys::load_from_path(dir).context("loading pre-extracted GTA keys failed")?;
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
    let (compressed, busy, failures) = squeezeit::gpu::counters();
    if compressed > 0 || busy > 0 || failures > 0 {
        println!(
            "  gpu       : {compressed} compressed, {busy} sent to CPU (busy), {failures} \
             sent to CPU (failed)"
        );
    }
    println!(
        "  saved     : {} ({:.1}% off the streaming budget){}",
        format_size(s.bytes_saved(), DECIMAL),
        s.percent_saved(),
        if dry_run {
            "  [dry run — nothing written]"
        } else {
            ""
        },
    );
}
