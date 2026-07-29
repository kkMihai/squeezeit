# SqueezeIt

A texture optimizer for GTA V and FiveM assets. It opens `.ytd`, `.ydd`, `.ydr`, `.yft`
and `.rpf` files directly, resizes and re-compresses the textures inside them, and writes
the container back. Loose `.dds`, `.png`, `.jpg`, `.tga` and `.bmp` files work too.

The point is streaming budget. Oversized textures in a clothing or vehicle pack are the
usual reason a server stutters when players spawn near each other, and most of that size
is textures nobody looks at closely enough to notice.

Everything is done in memory. Nothing gets unpacked to a temp folder.

[![CI](https://github.com/kkMihai/squeezeit/actions/workflows/ci.yml/badge.svg)](https://github.com/kkMihai/squeezeit/actions/workflows/ci.yml)

## Back up before you run it, and test in game after

This is the part that matters more than any setting in this readme.

Optimization is lossy and the failure modes are not always obvious. A pack can look fine
in OpenIV and still break hair strands at mid distance, shift the colour of a pair of
shoes, or crash the client during load on some GPUs. You will not catch that by looking
at the output file size.

So:

1. Keep a copy of the originals. Either your own copy, or run with `--backup` / turn on
   the Backup Vault so SqueezeIt keeps one for you.
2. Run the optimization.
3. Clear your FiveM client cache and actually load the pack in game.
4. Look at it. Hair at a few distances, shoes and fabric up close, vehicle paint, anything
   with soft alpha.
5. Only then roll it out.

Do this per pack, not once for the whole server. When something does break, you want to
know which batch caused it.

If a run goes wrong and you used the vault, put everything back with:

```sh
squeezeit-cli ./yourpack --restore
```

There is also `--dry-run`, which does the whole job and reports what it would have saved
without writing a single byte. Worth running first on anything you care about.

### The vault and FiveM

The vault lives in `.squeezeit-backup/` inside the folder you pointed at, which means it
can end up inside a resource's `stream/` directory. FiveM walks `stream/` recursively and
picks assets up by file extension, and it does not care that the folder name starts with a
dot. Left alone, that would hand the engine a second copy of every asset under the same
internal name, and which copy wins is not something you control.

So every file in the vault gets a `.sqzbak` suffix on the way in. `hair_006_u.ydd` is
stored as `hair_006_u.ydd.sqzbak`, which no extension-based scanner recognises, FiveM and
OpenIV included. Restore strips it again. Vaults created before this change still restore
fine.

That makes the vault safe to leave in place, but it still doubles the folder on disk and
travels with the resource if you zip it up. If you would rather it were nowhere near your
server at all, send it somewhere else:

```sh
squeezeit-cli ./yourpack --backup --backup-dir D:/backups/yourpack
```

## Build

Stable Rust, edition 2024. Windows is what gets built and tested; the release binaries are
`x86_64-pc-windows-msvc`.

```sh
cargo build --release --workspace
```

You get `squeezeit.exe` (the terminal UI) and `squeezeit-cli.exe` in `target/release/`.

## The terminal UI

```sh
squeezeit
```

| Key | Does |
| --- | --- |
| `o` / `f` | pick a folder / pick individual files |
| `a` or Tab | open settings |
| `↑` `↓` | move between settings |
| `←` `→` | change the highlighted setting |
| `s` or Enter | start |
| `c` | cancel a running batch |
| `r` | restore from the backup vault |
| `q` | quit |

Mouse works too. Clicking a row selects it; the `‹` and `›` arrows change the value.

The settings list is filtered by the preset you pick. Options a preset overrides are
greyed out and say which preset is overriding them, and the clothing and vehicle opt-ins
only appear when the preset actually touches those files. If a switch looks like it does
nothing, that is because it does nothing, and the UI says so instead of letting you
believe otherwise.

Settings are saved to `settings.yaml` next to the executable.

## The CLI

```sh
squeezeit-cli <PATH> [OPTIONS]
```

`<PATH>` is a file or a folder. Folders are walked all the way down. Run
`squeezeit-cli --help` for the full flag list, it is generated from the code and will not
go stale like a table in here would.

The ones you will actually use:

```sh
# see what it would do, change nothing
squeezeit-cli ./mods --dry-run

# cap at 1024, best quality encode, keep the originals in a vault
squeezeit-cli ./mods --max-res 1024 --quality slow --backup

# undo that
squeezeit-cli ./mods --restore
```

## Presets

A preset decides what each kind of asset is allowed to lose. `auto` works it out per file
from the shaders it declares and its filename, which is what you want for a mixed folder.
The named presets force one rule set onto everything, for folders you already know are all
one thing.

| Preset | Diffuse cap | Mipmaps | Format | Overdrive | GPU |
| --- | --- | --- | --- | --- | --- |
| `clothing` | 1024 | kept, tail trimmed, never added | never BC1 | off | opt-in |
| `hair` | 1024, floor 512 | never touched at all | locked | off | off |
| `hair-strict` | no resize at all | never touched at all | locked | off | off |
| `vehicles` | your limit | full chain for props, opt-in for vehicles | aggressive | allowed | allowed |
| `custom` | your limit | your switch | aggressive | allowed | allowed |

Caps are ceilings, not targets. The real limit is always the lower of your `--max-res` and
the preset's own cap, so turning `--max-res` down still works everywhere.

Hair and clothing are locked down because of things that actually broke: generated mipmaps
alone were enough to break hair strands, Overdrive shifted shoe colours, and generated
mipmaps combined with a format change lined up with `amdxx64.dll` crashing during load.
The full reasoning, and the three opt-ins that let you override it, are in
[docs/presets.md](docs/presets.md).

`custom` turns the family rules off entirely. It exists so you can do something the
presets consider unsafe. If you use it, see the section at the top of this readme again.

## What it will not touch

Under every preset, including `custom`:

- `script_rt*` textures are left completely alone. They are live render targets.
- Liveries, `_sign_` textures and weapon skins are never downscaled. People read those
  close up.
- Normal maps are never downgraded to BC1.
- A container is never written if its header would claim more data than the file actually
  contains. That check runs on every path and fails the file instead of shipping something
  that crashes on load.

Drawables (`.ydd`, `.ydr`, `.yft`) keep their original segment size. Textures are patched
into the slot they already occupy and the leftover space is zeroed, so the file still gets
smaller after deflate but nothing can grow. That is also why drawables never gain mipmaps.

## Encrypted archives

Encrypted `.rpf` files need the GTA V archive keys. Point SqueezeIt at your `GTA5.exe` and
it pulls them out itself:

```sh
squeezeit-cli ./mods --gta-exe "C:/Program Files/Rockstar Games/GTA V/GTA5.exe"
```

Or use `--gta-keys-dir` if you already have them extracted. In the UI it is the GTA V Path
setting.

Archives containing escrow or per-file-encrypted resources are skipped rather than
rebuilt, because rebuilding them would corrupt them.

## GPU

`--backend gpu` moves resizing and block compression onto the graphics card through
`wgpu`. It falls back to the CPU on its own when there is no adapter, so the flag is
always safe to pass, and individual textures the GPU cannot take fall back too.

Hair always runs on the CPU no matter what this is set to. Clothing does too unless you
turn on the cloth GPU opt-in (`--cloth-gpu`), which is off by default because GPU block
compression on cloth correlated with driver resets and corrupt drawables on AMD cards.

## Development

```sh
cargo test --workspace
cargo bench -p squeezeit
```

The first `cargo test` installs the git hooks via cargo-husky. They run `cargo fmt` and
`cargo clippy -D warnings` before each commit, same as CI.

Benchmarks are synthetic by default. Point them at real assets with
`SQUEEZEIT_BENCH_DIR=/path/to/assets`.

There is also a single-file debug tool:

```sh
cargo run --example squeeze_file -- path/to/thing.ytd
```

See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR, particularly if you are
touching the policy rules.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

Use it, fork it, sell services with it, run it on a paid server, do what you like. The one
condition is that if you distribute it, modified or not, you ship the source under the
same license. So a rebranded closed-source resale is not on the table, but everything else
is.
