# SqueezeIt

A texture optimization engine for game assets. SqueezeIt resizes, re-encodes, and block-compresses textures (DDS/BC formats) to shrink streaming budgets without visible quality loss, featuring first-class support for GTA V archives.

[![CI](https://github.com/kkmihai/squeezeit/actions/workflows/ci.yml/badge.svg)](https://github.com/kkmihai/squeezeit/actions/workflows/ci.yml)
[![License: Custom](https://img.shields.io/badge/license-Custom-blue.svg)](LICENSE)

## Workspace

| Crate  | Kind    | Description                                                        |
| ------ | ------- | ----------------------------------------------------------------- |
| `core` | library | The `squeezeit` engine: squeezers, pipeline, GPU backend, backups. |
| `cli`  | binary  | `squeezeit-cli` — batch terminal driver.                          |
| `app`  | binary  | `squeezeit` — interactive terminal UI (ratatui).                  |

## Build

Building requires a stable Rust toolchain (edition 2024). 

To build the entire workspace:

```sh
cargo build --release --workspace
```

The compiled binaries will be available in `target/release/` (`squeezeit`, `squeezeit-cli`).

## CLI Usage

```sh
squeezeit-cli <PATH> [OPTIONS]
```

`<PATH>` is a file or directory; directories are walked recursively.

### Options

| Flag                    | Default  | Description                                             |
| ----------------------- | -------- | ------------------------------------------------------- |
| `--max-res <N>`         | `2048`   | Cap the longest texture side; larger textures are downscaled. |
| `--quality <Q>`         | `normal` | Encoder effort: `fast`, `normal`, `slow`.               |
| `--backend <B>`         | `cpu`    | Processing backend: `cpu` or `gpu`. GPU falls back to CPU if no adapter is present. |
| `--overdrive`           | off      | Also downscale normal/specular maps to half their diffuse's size. |
| `--keep-format`         | off      | Keep the original pixel format (resize only, no re-encode). |
| `--force-convert`       | off      | Convert every image to DDS even when the result is larger. |
| `--dry-run`             | off      | Report savings without writing anything to disk.        |
| `--backup`              | off      | Back up originals into a vault before overwriting.      |
| `--backup-dir <DIR>`    | —        | Custom vault location for backups (implies `--backup`). |
| `--restore`             | off      | Restore originals from the vault, then exit.            |
| `--gta-keys-dir <DIR>`  | —        | Directory of pre-extracted GTA V archive keys.          |
| `--gta-exe <FILE>`      | —        | Extract GTA V archive keys from `GTA5.exe`.             |
| `--verbose`, `-v`       | off      | Verbose per-file logging.                               |

### Examples

Shrink a mod folder to 1024 max resolution with a slow (highest quality) encoding, and keep backups:

```sh
squeezeit-cli ./mods --max-res 1024 --quality slow --backup
```

Restore originals from the backup vault:

```sh
squeezeit-cli ./mods --restore
```

## Terminal UI

For an interactive front-end over the same engine, run:

```sh
squeezeit
```

You can pick a target, tune settings, and watch progress live.

## GPU Backend

`--backend gpu` offloads resizing and block compression to the GPU via `wgpu`.
Initialization falls back to the CPU automatically when no adapter is available, so the flag is always safe to pass.

## Development

Git hooks install automatically. [cargo-husky](https://github.com/rhysd/cargo-husky) is a dev-dependency that drops the repo's `.cargo-husky/hooks/` into `.git/hooks/` the first time you run `cargo test`. The pre-commit hook runs `cargo fmt --check` and `cargo clippy -D warnings` — the same gates as CI.

## Releasing

*Note: Only the author can publish and release this software.*

Versions are cut manually with [cargo-release](https://github.com/crate-ci/cargo-release):

```sh
cargo install cargo-release   # once
cargo release patch           # or: minor | major
```

It bumps the shared workspace version, commits `release: vX.Y.Z`, tags `vX.Y.Z`, and pushes. The pushed tag triggers [release.yml](.github/workflows/release.yml), which builds the Windows binaries and attaches them to a GitHub Release. cargo-release only previews unless you pass `-x`, so check first:

```sh
cargo release patch            # dry run — prints the planned steps
cargo release patch --execute  # actually bump, commit, tag, push
```

## License

This project is licensed under the SqueezeIt Custom License — see [LICENSE](LICENSE) for more details.
