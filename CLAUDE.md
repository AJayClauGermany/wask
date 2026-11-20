# wask

A markdown task runner (mask-compatible, `SKILL.md`-aware); see `README.md` for the
format. Single binary crate with no dependencies: keep it that way unless asked.
The toolchain is pinned in `rust-toolchain.toml`.

- `src/markdown.rs` parses files into the command tree in `src/model.rs`
- `src/cli.rs` matches arguments and prints help; `src/run.rs` executes scripts
- `src/render.rs` handles `--render` and the permission gate for `!` commands
- `src/yaml.rs` is the frontmatter/config subset parser; `src/config.rs` the settings
- `tests/cli.rs` runs the real binary against files in a sandbox

## Checks

Run these before committing. CI (`.github/workflows/ci.yml`) runs the same three.

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Commit `Cargo.lock`. Keep `target/` out of git.
