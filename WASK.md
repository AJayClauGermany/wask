# wask development tasks

## check

> Format, lint and test: everything CI runs

```bash
set -e
cd "$WASK_DIR"
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## install

> Install the wask binary from this checkout

```bash
cargo install --path "$WASK_DIR"
```
