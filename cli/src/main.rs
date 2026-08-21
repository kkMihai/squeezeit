#[cfg(not(windows))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod logging;

use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};
use humansize::{DECIMAL, format_size};
use indicatif::{ProgressBar, ProgressStyle};
use logging::BarLayer;
use squeezeit::scan::{FileScan, Verdict};
use squeezeit::{
    Backend, BackupVault, Exclusions, FormatMode, GpuContext, Liveries, Preset, Quality, Safety,
    ScriptRt, SizeLimit, SqueezeReport, SqueezeSettings, SqueezerRegistry, batch, scan,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::Level;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::prelude::*;
const SIZE: &str = "Size and format";
const PROTECT: &str = "What to protect";
const THIS_RUN: &str = "This run";
const MACHINE: &str = "Machine";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum QualityArg {
    #[value(help = "Near instant, slightly rougher gradients")]
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
    #[value(help = "Work out what each file is and treat it accordingly")]
    Auto,
    #[value(help = "Treat everything as body and outfit textures")]
    Clothing,
    #[value(help = "Treat everything as hair")]
    Hair,
    #[value(help = "Treat everything as vehicles and props")]
    Vehicles,
    #[value(help = "Drop the per-asset rules and use the switches alone")]
    Custom,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ScriptRtArg {
    #[value(help = "Leave them exactly as they are, faults and all")]
    Lock,
    #[value(help = "Rewrite compressed or mipmapped ones as the engine wants them")]
    Repair,
    #[value(help = "Repair those and change nothing else in the pack")]
    Only,
}

impl From<ScriptRtArg> for ScriptRt {
    fn from(a: ScriptRtArg) -> Self {
        match a {
            ScriptRtArg::Lock => ScriptRt::Lock,
            ScriptRtArg::Repair => ScriptRt::Repair,
            ScriptRtArg::Only => ScriptRt::Only,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    #[value(help = "Swap format when that saves space, leave it alone when it does not")]
    Auto,
    #[value(help = "Resize only. Re-encodes nothing except a loose .dds")]
    Keep,
    #[value(help = "Always write DDS, even when the DDS ends up bigger")]
    Dds,
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
            PresetArg::Vehicles => Preset::Vehicles,
            PresetArg::Custom => Preset::Custom,
        }
    }
}

impl From<FormatArg> for FormatMode {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Auto => FormatMode::Auto,
            FormatArg::Keep => FormatMode::Keep,
            FormatArg::Dds => FormatMode::ForceDds,
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
        help_heading = SIZE,
        help = "How each file is handled"
    )]
    preset: PresetArg,
    #[arg(
        long,
        default_value_t = SizeLimit::default(),
        value_name = "PIXELS|keep",
        help_heading = SIZE,
        help = "Biggest side a texture may keep, or `keep` to resize nothing"
    )]
    max_res: SizeLimit,
    #[arg(
        long,
        value_enum,
        default_value_t = QualityArg::Normal,
        help_heading = SIZE,
        help = "How long the compressor spends per texture. Same output size either way"
    )]
    quality: QualityArg,
    #[arg(
        long,
        value_enum,
        default_value_t = FormatArg::Auto,
        help_heading = SIZE,
        help = "What file format comes out"
    )]
    format: FormatArg,
    #[arg(
        long,
        overrides_with = "no_mipmaps",
        help_heading = PROTECT,
        help = "Build the smaller copies the game uses at a distance [default: on]"
    )]
    mipmaps: bool,
    #[arg(
        long,
        overrides_with = "mipmaps",
        help_heading = PROTECT,
        help = "Leave existing mipmaps as they are"
    )]
    no_mipmaps: bool,
    #[arg(
        long,
        help_heading = PROTECT,
        help = "Also halve the bumpiness and shininess maps against their colour map"
    )]
    overdrive: bool,
    #[arg(
        long,
        help_heading = PROTECT,
        help = "Let clothing and vehicles gain mipmaps. Test one pack in game before trusting it"
    )]
    relaxed: bool,
    #[arg(
        long,
        help_heading = PROTECT,
        help = "Resize liveries, signs and weapon skins too. They are protected by default"
    )]
    include_liveries: bool,
    #[arg(
        long,
        value_enum,
        default_value_t = ScriptRtArg::Repair,
        help_heading = PROTECT,
        help = "What to do about the surfaces the game draws on at runtime"
    )]
    script_rt: ScriptRtArg,
    #[arg(
        long,
        value_name = "NAME",
        help_heading = PROTECT,
        help = "Leave this texture exactly as it is. Repeat for more"
    )]
    exclude: Vec<String>,
    #[arg(
        long,
        value_name = "FILE",
        help_heading = PROTECT,
        help = "Leave alone every texture named in this file, one name per line"
    )]
    exclude_from: Option<PathBuf>,
    #[arg(
        long,
        help_heading = PROTECT,
        help = "Make the exclusion list care about capitals"
    )]
    match_case: bool,
    #[arg(
        long,
        help_heading = THIS_RUN,
        help = "Report what would be saved without writing anything"
    )]
    dry_run: bool,
    #[arg(
        long,
        help_heading = THIS_RUN,
        help = "List what each texture is and what the size rules would do to it, then stop"
    )]
    scan: bool,
    #[arg(
        long,
        help_heading = THIS_RUN,
        help = "Move originals into a vault before overwriting them"
    )]
    backup: bool,
    #[arg(
        long,
        value_name = "DIR",
        help_heading = THIS_RUN,
        help = "Put that vault somewhere else. Implies --backup"
    )]
    backup_dir: Option<PathBuf>,
    #[arg(
        long,
        help_heading = THIS_RUN,
        help = "Put the originals back from the vault, then quit"
    )]
    restore: bool,
    #[arg(
        long,
        value_enum,
        default_value_t = BackendArg::Cpu,
        help_heading = MACHINE,
        help = "Where the work happens. gpu falls back to cpu with no graphics card"
    )]
    backend: BackendArg,
    #[arg(
        short,
        long,
        help_heading = MACHINE,
        help = "Log every file, not just the failures"
    )]
    verbose: bool,
}

impl Cli {
    fn settings(&self) -> anyhow::Result<SqueezeSettings> {
        let mut names = self.exclude.clone();
        if let Some(path) = &self.exclude_from {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the exclusion list `{}`", path.display()))?;
            names.extend(exclusion_names(&text));
        }

        Ok(SqueezeSettings {
            preset: self.preset.into(),
            size_limit: self.max_res,
            quality: self.quality.into(),
            format: self.format.into(),
            mipmaps: self.mipmaps || !self.no_mipmaps,
            overdrive: self.overdrive,
            safety: if self.relaxed {
                Safety::Relaxed
            } else {
                Safety::Protected
            },
            liveries: if self.include_liveries {
                Liveries::Include
            } else {
                Liveries::Protect
            },
            script_rt: self.script_rt.into(),
            exclusions: Exclusions {
                names,
                ignore_case: !self.match_case,
            },
            backend: self.backend.into(),
            backup: self.backup || self.backup_dir.is_some(),
            dry_run: self.dry_run,
        })
    }
}

fn exclusion_names(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
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

    let settings = cli.settings()?;

    if cli.scan {
        let registry = SqueezerRegistry::new(None);
        let targets = batch::collect_targets(&cli.path, &registry);
        print_scan(&scan::scan(&targets, &settings), cli.verbose);
        return Ok(());
    }

    let gpu = match settings.backend {
        Backend::Gpu => match GpuContext::create_standalone() {
            Ok(ctx) => {
                println!("Warming up the GPU compressor (compiling shaders, a few seconds)...");
                if ctx.wait_until_ready() {
                    Some(Arc::new(ctx))
                } else {
                    eprintln!("warning: the GPU compressor never came up; running on the CPU");
                    None
                }
            }
            Err(e) => {
                eprintln!("warning: {e}; running on the CPU");
                None
            }
        },
        Backend::Cpu => None,
    };

    let registry = SqueezerRegistry::new(gpu);
    let targets = batch::collect_targets(&cli.path, &registry);
    if targets.is_empty() {
        println!("Nothing to squeeze under `{}`.", cli.path.display());
        return Ok(());
    }

    println!(
        "Squeezing {} texture(s) under `{}`{}",
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

fn restore(cli: &Cli) -> anyhow::Result<()> {
    let vault = BackupVault::new(&cli.path, cli.backup_dir.clone())
        .context("failed to open the backup vault")?;
    let vault_dir = vault.backup_root().to_path_buf();
    let restored = vault
        .restore_all()
        .context("restore failed, the vault and manifest are untouched")?;
    println!(
        "Restored {restored} file(s) from `{}`.",
        vault_dir.display()
    );
    Ok(())
}

fn print_scan(files: &[FileScan], verbose: bool) {
    if files.is_empty() {
        println!("Nothing to scan.");
        return;
    }

    let (mut textures, mut resizing, mut reserved) = (0usize, 0usize, 0u64);
    let mut protected = 0usize;
    for file in files {
        textures += file.textures.len();
        resizing += file.resizing();
        reserved += file.reserved;
        protected += file
            .textures
            .iter()
            .filter(|t| {
                matches!(
                    t.verdict,
                    Verdict::Excluded | Verdict::RenderTarget | Verdict::Protected
                )
            })
            .count();

        if !verbose && file.resizing() == 0 {
            continue;
        }
        println!(
            "\n{}  [{}]  {} reserved",
            file.path.display(),
            family_label(file.family),
            format_size(file.reserved, DECIMAL),
        );
        for tex in &file.textures {
            if !verbose && tex.verdict != Verdict::Resize {
                continue;
            }
            let change = match tex.resize_to {
                Some((w, h)) => format!("{}x{} -> {w}x{h}", tex.width, tex.height),
                None => format!("{}x{}", tex.width, tex.height),
            };
            println!(
                "    {:<9} {:<22} lv {:<3} {}",
                verdict_label(tex.verdict),
                change,
                tex.levels,
                tex.name,
            );
        }
    }

    println!();
    println!("-- SqueezeIt scan ----------------------------");
    println!("  files     : {}", files.len());
    println!("  textures  : {textures}");
    println!("  resizing  : {resizing}");
    println!("  protected : {protected}  (excluded, livery or render target)");
    println!(
        "  reserved  : {}  (graphics memory these containers hold today)",
        format_size(reserved, DECIMAL)
    );
    if !verbose {
        println!("  note      : pass -v to list every texture, not just the ones that move");
    }
    println!("  note      : format changes need the pixels, so they are not predicted here");
}

fn family_label(family: squeezeit::AssetFamily) -> &'static str {
    match family {
        squeezeit::AssetFamily::Vehicle => "vehicle",
        squeezeit::AssetFamily::PedCloth => "clothing",
        squeezeit::AssetFamily::PedHair => "hair",
        squeezeit::AssetFamily::Generic => "generic",
    }
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Resize => "resize",
        Verdict::AlreadyFits => "fits",
        Verdict::Excluded => "excluded",
        Verdict::RenderTarget => "screen",
        Verdict::Protected => "livery",
    }
}

fn print_summary(report: &SqueezeReport, dry_run: bool) {
    let s = report.snapshot();
    println!();
    println!("-- SqueezeIt summary -------------------------");
    println!("  smaller   : {}", s.optimized);
    println!("  protected : {}  (left alone on purpose)", s.locked);
    println!("  skipped   : {}  (already small enough)", s.skipped);
    println!("  failed    : {}", s.failed);
    println!(
        "  speed     : {:.1} files/s over {:.1}s",
        s.files_per_sec(),
        s.elapsed_secs()
    );
    println!(
        "  on disk   : {} -> {}  ({} saved, {:.1}%)",
        format_size(s.bytes_before, DECIMAL),
        format_size(s.bytes_after, DECIMAL),
        format_size(s.bytes_saved(), DECIMAL),
        s.percent_saved(),
    );

    if s.has_texture_memory() {
        println!(
            "  textures  : {} -> {}  ({} saved, {:.1}%)",
            format_size(s.textures_before, DECIMAL),
            format_size(s.textures_after, DECIMAL),
            format_size(s.textures_saved(), DECIMAL),
            s.percent_textures_saved(),
        );
        println!("              (texture memory, what the game keeps resident)");
    }
    let (compressed, busy, failures) = squeezeit::gpu::counters();
    if compressed > 0 || busy > 0 || failures > 0 {
        println!(
            "  gpu       : {compressed} done on the card, {busy} handed back (busy), {failures} \
             handed back (failed)"
        );
    }
    if dry_run {
        println!("  note      : preview only, nothing was written");
    }
}
