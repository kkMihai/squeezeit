# Contributing

Thanks for looking. Here is how to get a change in.

## Setup

Fork it, clone your fork, and make sure you have a stable Rust toolchain on edition 2024.
This is a Windows-first project, so you want the MSVC build tools.

```sh
git clone https://github.com/your-username/squeezeit.git
cd squeezeit
cargo test --workspace
```

That first `cargo test` also installs the git hooks through
[cargo-husky](https://github.com/rhysd/cargo-husky). They run `fmt` and `clippy` before
every commit, which is the same gate CI uses.

## Making a change

Branch off `master`, write the change, then run what CI runs:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Open a PR against `master`. Direct commits to `master` are blocked. Say what was broken and
how the change fixes it.

## Touching the policy

`core/src/squeezers/policy.rs` decides what each asset family is allowed to lose. Every
rule in there exists because something broke on a real machine — see
[docs/presets.md](docs/presets.md).

If you loosen one, the PR needs a test pack you actually loaded in FiveM and the three
failure signatures you checked for: hair strand breakage at mid distance, shoe colour
shift, and an `amdxx64.dll` crash during load. "It compiled" is not evidence.

Tightening a rule is always fine.

## Bugs

Open an issue. Include the file that broke if you can share it, the preset and settings
you used, your GPU, and whether it reproduces on `--backend cpu`. That last one narrows it
down more than anything else.
