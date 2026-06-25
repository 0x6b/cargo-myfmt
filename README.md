# cargo-myfmt

A `cargo` subcommand that runs `cargo +nightly fmt` with a personal `rustfmt.toml` config, without keeping that config in the project tree.

It writes the embedded config to a temporary file, invokes `cargo +nightly fmt -- --config-path <tempfile>`, then removes it. If the workspace root has a `.gitattributes`, Rust files marked with `linguist-generated` or `linguist-generated=true` are skipped.

## Install

```sh
cargo install --path .
```

## Usage

```sh
cargo myfmt              # format the current crate
cargo myfmt -- --check   # extra args are forwarded to rustfmt
```

Requires the nightly toolchain (`rustup toolchain install nightly`) since the embedded config uses unstable options.

## Config

The `rustfmt.toml` is embedded in `src/main.rs` and intentionally overrides any project-local rustfmt config. Edit and reinstall to change it.

## License

MIT. See [LICENSE](LICENSE) for detail.
