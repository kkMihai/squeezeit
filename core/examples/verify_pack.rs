use squeezeit::gta5;
use std::path::{Path, PathBuf};
const EXTENSIONS: &[&str] = &["ytd", "ydd", "ydr", "yft"];

fn main() -> std::process::ExitCode {
    let Some(root) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: verify_pack <file or folder>");
        return std::process::ExitCode::FAILURE;
    };
    if !root.exists() {
        eprintln!("`{}` does not exist", root.display());
        return std::process::ExitCode::FAILURE;
    }

    let files = collect(&root);
    if files.is_empty() {
        println!("No .ytd/.ydd/.ydr/.yft under `{}`.", root.display());
        return std::process::ExitCode::SUCCESS;
    }

    let (mut sound, mut textures, mut reserved) = (0usize, 0usize, 0u64);
    let mut broken = Vec::new();
    let mut unreadable = 0usize;

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            unreadable += 1;
            continue;
        };
        match gta5::inspect_bytes(path, &bytes) {
            Ok(info) => {
                textures += info.textures.len();
                reserved += info.graphics_bytes;
            }
            Err(_) => unreadable += 1,
        }
        match gta5::verify_bytes(path, &bytes) {
            Ok(()) => sound += 1,
            Err(error) => broken.push((path.clone(), error)),
        }
    }

    println!(
        "Checked {} container(s) under `{}`.",
        files.len(),
        root.display()
    );
    println!("  sound      : {sound}");
    println!("  unsound    : {}", broken.len());
    println!("  unreadable : {unreadable}  (not RSC7, or a layout this build cannot parse)");
    println!("  textures   : {textures}");
    println!(
        "  reserved   : {:.1} MiB of graphics memory across the pack",
        reserved as f64 / (1024.0 * 1024.0)
    );

    if broken.is_empty() {
        return std::process::ExitCode::SUCCESS;
    }
    println!("\nThese would not load reliably:");
    for (path, error) in &broken {
        println!("  {}\n      {error}", path.display());
    }
    std::process::ExitCode::FAILURE
}

fn collect(root: &Path) -> Vec<PathBuf> {
    let claimed = |path: &Path| {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.iter().any(|c| c.eq_ignore_ascii_case(e)))
    };
    if root.is_file() {
        return if claimed(root) {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        };
    }

    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if claimed(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}
