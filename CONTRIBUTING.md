# Contributing

Thanks for looking. Here is how to get a change in.

## Setup

Fork it, clone your fork, and make sure you have a stable Rust toolchain on edition 2024.
This is a Windows-first project, so you want the MSVC build tools.

```bash
git clone https://github.com/your-username/squeezeit.git
```

```bash
cargo test --workspace
```

That first `cargo test` also installs the git hooks through
[cargo-husky](https://github.com/rhysd/cargo-husky). They run `fmt` and `clippy` before
every commit, which is the same gate CI uses.

## Making a change

Branch off `master`, write the change, then run what CI runs:

```bash
cargo fmt --all
```

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```bash
cargo test --workspace --all-features
```

CI runs the same three on `windows-latest`, with `-D warnings` coming from `RUSTFLAGS`
rather than the clippy invocation. Windows is the only target in the matrix.

Open a PR against `master`. Direct commits to `master` are blocked. Say what was broken and
how the change fixes it.

## Adding a setting

Don't, if you can avoid it. There are sixteen and that is already a lot to read before
running anything.

If you do need one, two rules apply:

- **It has to change the output.** Anything that only changes how the run is displayed
  belongs somewhere other than `SqueezeSettings`.
- **It cannot contradict another setting.** If you find yourself writing "X wins if you
  turn on both", you have found an enum, not two booleans. Fold the choices into one
  setting instead of documenting a precedence rule.

A setting the presets always overrule is worse than no setting, because the UI has to grey
it out and explain itself. `Preset::allows` records which knobs a preset overrules, and
`vetoed_knobs_match_the_resolved_policy` in `core/src/squeezers/policy.rs` fails if that
list ever disagrees with the policy it actually resolves to. Keep it that way. A switch
that silently does nothing is the failure mode this whole design is avoiding.

Adding a row to the UI means adding it to `Row::ALL` in `app/src/app/ui/settings.rs` and
nothing else. Rows do not appear and disappear. `every_label_fits_the_narrow_layout` will
tell you if your label, control or hint is too wide for the column.

## Testing

```bash
cargo test --workspace
```

The tests are synthetic and need nothing but the repo. They hold the invariants that can
be checked without game assets: that no packed block crosses a page boundary
(`core/src/rsc7/pages.rs`), that the policy matrix resolves the way the presets advertise
(`core/src/squeezers/policy.rs`), what each texture role is allowed to become
(`core/src/squeezers/texture.rs`), and that a `settings.yaml` from a previous release still
loads (`app/src/app/persist.rs`).

What they cannot check is whether a rebuilt container actually loads in the engine. That
is what `verify_pack` is for, and it wants real files:

```bash
cargo run --release -p squeezeit --example verify_pack -- path/to/pack
```

Any change to how a container is written needs a run of that against a pack spanning
clothing, vehicles, maps and props, with the counts it prints, in the PR. Run it in
release: block compression in a debug build is slow enough to look like a hang. Point it
at a copy, though nothing in it writes.

## The one rule about writing containers

A RAGE resource segment is a run of pages, and the engine allocates those pages
separately. A block that crosses a page boundary reads into a different allocation once
the file is loaded. It works when the allocator happens to hand out adjacent pages and
crashes when it does not, so the bug shows up as a crash on somebody else's machine and
never on yours.

No texture in a shipping `.ytd` crosses a page boundary. This is not a convention anyone
chose, it is the format.

So: nothing decides where a block goes except `PageLayout::pack` in
`core/src/rsc7/pages.rs`, and every container is read back by
`core/src/squeezers/gta5/verify.rs` before it is allowed to reach disk. If you add a path
that writes a container, it goes through both. `verify_pack` is the same check as a
standalone tool:

```bash
cargo run --release -p squeezeit --example verify_pack -- path/to/pack
```

## Touching the policy

`core/src/squeezers/policy.rs` decides what each asset family is allowed to lose. Every
rule in there guards a failure you can reproduce in game. See
[docs/presets.md](docs/presets.md).

If you loosen one, the PR needs a test pack you actually loaded in FiveM and the three
failures you checked for: alpha-cut hair thinning out at middle distance, colours drifting
on fabric, and the pack failing to load. "It compiled" is not evidence.

Tightening a rule is always fine.

One rule in there is not about what fits, it is about what looks right: clothing and hair
never gain mipmaps. Generated mip chains make garments and hair look wrong at close and
middle distance. `Policy::mip_exception_applies` states the refusal explicitly, and
`no_drawable_clothing_texture_ever_grows_a_mip_chain` holds it there. Do not let it drift
open.

## Changing the config format

`settings.yaml` is written next to the executable and people have one. Renaming a field
means adding `#[serde(alias = "old_name")]` in `app/src/app/persist.rs`, and
`a_config_from_an_older_build_still_loads` is where you prove it works. A field with no
successor should land on its safe default rather than a guess at what the user meant.

## Bugs

Open an issue. Include the file that broke if you can share it, the preset and settings you
used, your GPU, and whether it reproduces on `--backend cpu`. That last one narrows it down
more than anything else.
