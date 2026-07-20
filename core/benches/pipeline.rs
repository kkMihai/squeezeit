use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use image::{ImageFormat, RgbaImage};
use squeezeit::{
    GpuContext, ProcessingBackend, SqueezeReport, SqueezeSettings, SqueezerRegistry, batch,
};

fn unique_temp_dir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("squeezeit-bench-{label}-{pid}-{nanos}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn synth_png(width: u32, height: u32) -> Vec<u8> {
    let img = RgbaImage::from_fn(width, height, |x, y| {
        let n = x.wrapping_mul(31).wrapping_add(y.wrapping_mul(17));
        image::Rgba([
            (x & 0xff) as u8,
            (y & 0xff) as u8,
            ((x.wrapping_add(y)) & 0xff) as u8 ^ (n & 0x3f) as u8,
            255,
        ])
    });
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .expect("encode synthetic png");
    buf
}

fn settings_for(backend: ProcessingBackend) -> SqueezeSettings {
    SqueezeSettings {
        dry_run: true,
        make_backup: false,
        backend,
        ..Default::default()
    }
}
fn gpu() -> Option<Arc<GpuContext>> {
    static CTX: std::sync::OnceLock<Option<Arc<GpuContext>>> = std::sync::OnceLock::new();
    CTX.get_or_init(|| {
        let ctx = GpuContext::create_standalone().ok().map(Arc::new)?;
        ctx.warm_up();
        Some(ctx)
    })
    .clone()
}

fn run_once(
    targets: &[PathBuf],
    root: &Path,
    registry: &SqueezerRegistry,
    backend: ProcessingBackend,
) {
    let report = SqueezeReport::new();
    let cancel = AtomicBool::new(false);
    batch::run_targets(
        targets,
        root,
        registry,
        &settings_for(backend),
        None,
        &report,
        &cancel,
    )
    .expect("run_targets");
}

fn bench_single_texture(c: &mut Criterion) {
    let mut group = c.benchmark_group("squeeze_single");
    group.sample_size(10);

    for &side in &[1024u32, 2048] {
        let dir = unique_temp_dir("single");
        let target = dir.join("tile_d.png");
        fs::write(&target, synth_png(side, side)).expect("write test file");
        let targets = vec![target.clone()];

        let cpu_registry = SqueezerRegistry::with_defaults();
        group.bench_with_input(BenchmarkId::new("cpu", side), &side, |b, _| {
            b.iter(|| run_once(&targets, &target, &cpu_registry, ProcessingBackend::CPU));
        });

        if let Some(ctx) = gpu() {
            let gpu_registry = SqueezerRegistry::with_defaults_gpu(Some(ctx));
            run_once(&targets, &target, &gpu_registry, ProcessingBackend::GPU);
            group.bench_with_input(BenchmarkId::new("gpu", side), &side, |b, _| {
                b.iter(|| run_once(&targets, &target, &gpu_registry, ProcessingBackend::GPU));
            });
        }

        let _ = fs::remove_dir_all(&dir);
    }
    group.finish();
}

fn bench_batch_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("squeeze_batch");
    group.sample_size(10);

    let total_files = 64usize;
    let side = 512u32;

    let dir = unique_temp_dir("throughput");
    let png = synth_png(side, side);
    let mut targets = Vec::with_capacity(total_files);
    for i in 0..total_files {
        let sub = dir.join(format!("dir_{:02}", i % 8));
        fs::create_dir_all(&sub).expect("create subdir");
        let path = sub.join(format!("texture_{i:04}_d.png"));
        fs::write(&path, &png).expect("write texture");
        targets.push(path);
    }

    let cpu_registry = SqueezerRegistry::with_defaults();
    group.bench_function(
        BenchmarkId::new("cpu", format!("{total_files}x{side}")),
        |b| {
            b.iter(|| run_once(&targets, &dir, &cpu_registry, ProcessingBackend::CPU));
        },
    );

    if let Some(ctx) = gpu() {
        let gpu_registry = SqueezerRegistry::with_defaults_gpu(Some(ctx));
        run_once(&targets, &dir, &gpu_registry, ProcessingBackend::GPU);
        group.bench_function(
            BenchmarkId::new("gpu", format!("{total_files}x{side}")),
            |b| {
                b.iter(|| run_once(&targets, &dir, &gpu_registry, ProcessingBackend::GPU));
            },
        );
    }

    let _ = fs::remove_dir_all(&dir);
    group.finish();
}

fn bench_collect_targets(c: &mut Criterion) {
    let mut group = c.benchmark_group("collect_targets/walk");
    let registry = SqueezerRegistry::with_defaults();

    for &total in &[16usize, 64, 256] {
        let dir = unique_temp_dir("collect");
        for i in 0..total {
            let sub = dir.join(format!("dir_{:02}", i % 8));
            fs::create_dir_all(&sub).expect("create subdir");
            fs::write(sub.join(format!("texture_{i:04}.png")), synth_png(16, 16))
                .expect("write texture");
            fs::write(sub.join(format!("notes_{i:04}.txt")), b"not a texture")
                .expect("write noise");
        }

        group.bench_with_input(BenchmarkId::from_parameter(total), &total, |b, _| {
            b.iter(|| {
                let targets = batch::collect_targets(&dir, &registry);
                assert!(!targets.is_empty());
            });
        });

        let _ = fs::remove_dir_all(&dir);
    }

    group.finish();
}

fn bench_real_corpus(c: &mut Criterion) {
    let Ok(dir) = std::env::var("SQUEEZEIT_BENCH_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let cpu_registry = SqueezerRegistry::with_defaults();
    let targets = batch::collect_targets(&dir, &cpu_registry);
    if targets.is_empty() {
        eprintln!("SQUEEZEIT_BENCH_DIR set but no containers found under {dir:?} — skipping");
        return;
    }

    let mut group = c.benchmark_group("squeeze_corpus");
    group.sample_size(10);
    let label = format!("{}files", targets.len());

    group.bench_function(BenchmarkId::new("cpu", &label), |b| {
        b.iter(|| run_once(&targets, &dir, &cpu_registry, ProcessingBackend::CPU));
    });

    if let Some(ctx) = gpu() {
        let gpu_registry = SqueezerRegistry::with_defaults_gpu(Some(ctx));
        run_once(&targets, &dir, &gpu_registry, ProcessingBackend::GPU);
        group.bench_function(BenchmarkId::new("gpu", &label), |b| {
            b.iter(|| run_once(&targets, &dir, &gpu_registry, ProcessingBackend::GPU));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_single_texture,
    bench_batch_throughput,
    bench_collect_targets,
    bench_real_corpus
);
criterion_main!(benches);
