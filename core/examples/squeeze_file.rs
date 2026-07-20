use std::path::Path;
use std::sync::Arc;

use squeezeit::{
    GpuContext, ProcessingBackend, SqueezeOutcome, SqueezeSettings, SqueezerRegistry, TextureJob,
};

fn squeeze(registry: &SqueezerRegistry, path: &Path, bytes: &[u8], settings: &SqueezeSettings) {
    let squeezer = registry.find(path).expect("no squeezer claims this file");
    let job = TextureJob {
        path,
        bytes,
        asset_hint: None,
        pool_saturated: false,
    };
    let started = std::time::Instant::now();
    match squeezer.squeeze(&job, settings).expect("squeeze failed") {
        SqueezeOutcome::Optimized {
            bytes: out,
            extension,
        } => {
            println!(
                "optimized: {} -> {} bytes ({:.1}% saved, .{extension}, {:?})",
                bytes.len(),
                out.len(),
                100.0 * (bytes.len() - out.len()) as f64 / bytes.len() as f64,
                started.elapsed(),
            );
            let job = TextureJob {
                path,
                bytes: &out,
                asset_hint: None,
                pool_saturated: false,
            };
            match squeezer.squeeze(&job, settings) {
                Ok(_) => println!("round-trip: output re-parses OK"),
                Err(e) => println!("round-trip FAILED: {e}"),
            }
        }
        SqueezeOutcome::Locked { reason } => println!("locked: {reason}"),
        SqueezeOutcome::Skipped { reason } => println!("skipped: {reason}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: squeeze_file <path> [cpu|gpu]");
    let backend = match args.next().as_deref() {
        Some("gpu") => ProcessingBackend::GPU,
        _ => ProcessingBackend::CPU,
    };
    let path = Path::new(&path);
    let bytes = std::fs::read(path).expect("read input");

    let gpu = match backend {
        ProcessingBackend::GPU => {
            let ctx = Arc::new(GpuContext::create_standalone().expect("GPU context"));
            ctx.warm_up();
            Some(ctx)
        }
        ProcessingBackend::CPU => None,
    };
    let registry = SqueezerRegistry::with_defaults_gpu(gpu);
    let settings = SqueezeSettings {
        backend,
        dry_run: true,
        make_backup: false,
        ..Default::default()
    };
    squeeze(&registry, path, &bytes, &settings);
    let (compressed, busy, failures) = squeezeit::gpu::counters();
    println!("gpu counters: {compressed} compressed, {busy} busy, {failures} failed");
}
