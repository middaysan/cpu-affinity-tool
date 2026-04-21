# Contributing

Thanks for your interest in improving CPU Affinity Tool.

This project is currently a Windows-first utility. Linux code exists in the repository, but it is still experimental and is not part of the supported release contract.

## Build basics

Use Rust stable and build the Windows binary with:

```bash
cargo build --release --bin cpu-affinity-tool
```

Useful verification commands:

```bash
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test --manifest-path libs/os_api/Cargo.toml
cargo test
cargo build --release
```

## Reporting bugs

Please use the bug report issue template and include:

- Windows version
- CPU model
- CPU Affinity Tool version
- exact reproduction steps
- expected behavior
- actual behavior
- screenshots or logs when possible

If the problem involves launch, monitoring, or affinity correction, include whether the target was added with **Open App** or **Find Installed**.

## Requesting features

Please use the feature request template.

Feature requests should explain:

- the real problem
- the proposed behavior
- alternatives you considered

Do not open large implementation pull requests for new features without discussing them first.

## Pull request expectations

- Keep changes scoped
- Avoid mixing refactors with feature work unless they are required
- Update documentation when behavior, platform truth, release process, or repository structure changes
- Preserve truthful Windows-first positioning
- Do not add claims about unsupported platforms or unverified performance gains

## Coding and review expectations

- Keep comments in English
- Keep user-facing repository text in English
- Prefer targeted changes over broad rewrites
- Run the relevant checks before asking for review
