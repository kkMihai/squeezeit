# Contributing to SqueezeIt

First off, thank you for considering contributing to SqueezeIt! It's people like you that make this tool better for everyone.

## Development Setup

1. **Fork the Repository**: Start by forking the repository to your own GitHub account.
2. **Clone**: Clone your fork locally.
   ```sh
   git clone https://github.com/your-username/squeezeit.git
   cd squeezeit
   ```
3. **Rust Toolchain**: Ensure you have the stable Rust toolchain (edition 2024) installed via `rustup`. Since this is a Windows-only project, ensure you have the MSVC build tools installed.

## Submitting Changes

All contributions must go through a Pull Request (PR). Direct commits to the `main` branch are restricted. 

Here is the standard workflow:

1. **Create a branch**: Create a feature or bugfix branch from `main`.
   ```sh
   git checkout -b feature/my-awesome-feature
   ```
2. **Write Code**: Make your changes.
3. **Test and Format**: We use standard Rust tooling to enforce code quality. Run the following commands to ensure your code passes CI:
   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets --all-features -D warnings
   cargo test --workspace --all-features
   ```
   *Note: Git hooks will install automatically the first time you run `cargo test` (via `cargo-husky`), which will run `cargo fmt` and `cargo clippy` before every commit.*
4. **Commit**: Commit your changes with descriptive commit messages.
5. **Push**: Push your branch to your fork.
   ```sh
   git push origin feature/my-awesome-feature
   ```
6. **Pull Request**: Open a Pull Request against the `main` branch of the upstream repository. Ensure your PR description clearly describes the problem you're solving and how your changes fix it.

## Code Review

Once your PR is submitted, it will be reviewed by the maintainers. 
- All GitHub Actions (CI) checks must pass.
- A maintainer must approve the PR before it can be merged.
- Be open to feedback and ready to make adjustments if requested.

## Reporting Issues

If you find a bug or have a feature request, please open an Issue on GitHub. Include as much detail as possible (logs, steps to reproduce, OS details, etc.) to help us understand and resolve the issue.
