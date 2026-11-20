# wask

Rust project. No crates yet; the toolchain is pinned in `rust-toolchain.toml`
(stable, with clippy, rustfmt, rust-analyzer and rust-src).

## Checks

Run these before committing. CI (`.github/workflows/ci.yml`) runs the same three
once a `Cargo.toml` exists.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Commit `Cargo.lock`. Keep `target/` out of git.
