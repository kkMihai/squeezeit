use std::path::Path;
use std::sync::Arc;

use squeezeit::{
    Backend, GpuContext, SqueezeOutcome, SqueezeSettings, SqueezerRegistry, TextureJob,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: squeeze_file <path> [cpu|gpu]");
    let backend = match args.next().as_deref() {
        Some("gpu") => Backend::Gpu,
        _ => Backend::Cpu,
    };

    let path = Path::new(&path);
    let bytes = std::fs::read(path).expect("read input");

    let gpu = (backend == Backend::Gpu).then(|| {
        let ctx = Arc::new(GpuContext::create_standalone().expect("GPU context"));
        ctx.warm_up();
        ctx
    });

    let registry = SqueezerRegistry::new(gpu, None);
    let settings = SqueezeSettings {
        backend,
        dry_run: true,
        ..Default::default()
    };

    squeeze(&registry, path, &bytes, &settings);

    let (compressed, busy, failures) = squeezeit::gpu::counters();
    println!("gpu counters: {compressed} compressed, {busy} busy, {failures} failed");
}

fn squeeze(registry: &SqueezerRegistry, path: &Path, bytes: &[u8], settings: &SqueezeSettings) {
    let job = TextureJob {
        path,
        bytes,
        asset_hint: squeezeit::gta5::family_from_filename(path),
        pool_saturated: false,
    };

    let started = std::time::Instant::now();
    match registry.squeeze(&job, settings).expect("squeeze failed") {
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
            let round_trip = TextureJob { bytes: &out, ..job };
            match registry.squeeze(&round_trip, settings) {
                Ok(_) => println!("round-trip: output re-parses OK"),
                Err(e) => println!("round-trip FAILED: {e}"),
            }
        }
        SqueezeOutcome::Locked { reason } => println!("locked: {reason}"),
        SqueezeOutcome::Skipped { reason } => println!("skipped: {reason}"),
    }
}
