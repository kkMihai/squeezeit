# SqueezeIt

A texture optimization engine for game assets. SqueezeIt resizes, re-encodes,
and block-compresses textures (DDS/BC formats) to shrink streaming budgets
without visible quality loss, with first-class support for GTA V archives.

[![CI](https://github.com/kkmihai/squeezeit/actions/workflows/ci.yml/badge.svg)](https://github.com/kkmihai/squeezeit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Workspace

| Crate  | Kind    | Description                                                        |
| ------ | ------- | ----------------------------------------------------------------- |
| `core` | library | The `squeezeit` engine: squeezers, pipeline, GPU backend, backups. |
| `cli`  | binary  | `squeezeit-cli` — batch terminal driver.                          |
| `app`  | binary  | `squeezeit` — interactive terminal UI (ratatui).                  |

## Build

Requires a stable Rust toolchain (edition 2024). On Linux install the GTK
development headers for the file dialog:

```sh
sudo apt-get install -y libgtk-3-dev   # Linux only
```

```sh
cargo build --release --workspace
```

Binaries land in `target/release/` (`squeezeit`, `squeezeit-cli`).

## CLI usage

```sh
squeezeit-cli <PATH> [OPTIONS]
```

`<PATH>` is a file or directory; directories are walked recursively.

| Flag                    | Default  | Description                                             |
| ----------------------- | -------- | ------------------------------------------------------ |
| `--max-res <N>`         | `2048`   | Cap the longest texture dimension.                     |
| `--quality <Q>`         | `normal` | Encoder effort: `fast`, `normal`, `slow`.              |
| `--backend <B>`         | `cpu`    | Processing backend: `cpu` or `gpu`.                    |
| `--overdrive`           | off      | More aggressive size reduction.                        |
| `--clothing-fidelity`   | off      | Preserve detail on clothing textures.                  |
| `--no-mipmaps`          | off      | Skip mipmap generation.                                |
| `--keep-format`         | off      | Keep the source pixel format.                          |
| `--force-convert`       | off      | Convert every image to DDS even when the result grows. |
| `--dry-run`             | off      | Report savings without writing.                        |
| `--backup`              | off      | Copy originals into a backup vault first.              |
| `--backup-dir <DIR>`    | —        | Custom vault location (implies `--backup`).            |
| `--restore`             | off      | Restore originals from the vault, then exit.           |
| `--gta-keys-dir <DIR>`  | —        | Pre-extracted GTA V archive keys.                      |
| `--gta-exe <FILE>`      | —        | Extract GTA V keys from `GTA5.exe`.                     |
| `--verbose`, `-v`       | off      | Verbose logging.                                       |

Example — shrink a mod folder to 1K textures with a backup:

```sh
squeezeit-cli ./mods --max-res 1024 --quality slow --backup
```

Restore:

```sh
squeezeit-cli ./mods --restore
```

## Terminal UI

```sh
squeezeit
```

An interactive front-end over the same engine: pick a target, tune settings,
and watch progress live.

## GPU backend

`--backend gpu` offloads resizing and block compression to the GPU via `wgpu`.
Initialization falls back to the CPU automatically when no adapter is available,
so the flag is always safe to pass.

## Development

Git hooks install automatically. [cargo-husky](https://github.com/rhysd/cargo-husky)
is a dev-dependency that drops the repo's `.cargo-husky/hooks/` into `.git/hooks/`
the first time you run `cargo test`. The pre-commit hook runs `cargo fmt --check`
and `cargo clippy -D warnings` — the same gates as CI.

## Releasing

Versions are cut manually with [cargo-release](https://github.com/crate-ci/cargo-release):

```sh
cargo install cargo-release   # once
cargo release patch           # or: minor | major
```

It bumps the shared workspace version, commits `release: vX.Y.Z`, tags `vX.Y.Z`,
and pushes. The pushed tag triggers [release.yml](.github/workflows/release.yml),
which builds the Linux/Windows/macOS binaries and attaches them to a GitHub
Release. cargo-release only previews unless you pass `-x`, so check first:

```sh
cargo release patch            # dry run — prints the planned steps
cargo release patch --execute  # actually bump, commit, tag, push
```

## License

MIT — see [LICENSE](LICENSE).
