# Contributing to arxiv-search-rs-mcp

First off, thank you for considering contributing to `arxiv-search-rs-mcp`! It's people like you that make open source software such a great community to learn, inspire, and create.

We welcome contributions from anyone and everyone.

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork locally.
3. Install the Rust toolchain (we recommend using [rustup](https://rustup.rs/)).

## Pre-commit Hooks

We use `pre-commit` to ensure code quality before pushing. Please install it to format and lint your code automatically.

```bash
# Install pre-commit (if you haven't already)
pip install pre-commit

# Install the git hook scripts
pre-commit install
```

This will run `cargo fmt` and `cargo clippy` automatically on your staged files before every commit.

## Continuous Integration (CI)

Our CI pipeline enforces strict code quality checks to maintain a healthy codebase. When you submit a Pull Request, the CI pipeline will verify:

1. **Formatting**: Your code must be formatted with `cargo fmt`.
2. **Linting**: We enforce strict `clippy` lints, including `pedantic` and `nursery` warnings. Please ensure `cargo clippy --all-targets --all-features` passes cleanly without warnings.
3. **Tests**: All tests must pass. You can run them locally with `cargo test`.
4. **Builds**: The workspace must compile successfully on both native and wasm targets.

## Submitting Changes

1. Create a new branch for your feature or bugfix.
2. Make your changes and ensure your commit messages are descriptive.
3. Push your branch to your fork.
4. Open a Pull Request against the `master` branch of the main repository.

## Need Help?

If you have any questions or need help, feel free to open an issue or ask in the Pull Request discussion. We're happy to help you get your code merged!
