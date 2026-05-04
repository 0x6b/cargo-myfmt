# cargo-myfmt

A `cargo` subcommand that runs `cargo +nightly fmt` with a personal `rustfmt.toml` config, without polluting the project tree.

It writes the embedded config to a tempfile, invokes `cargo +nightly fmt -- --config-path <tempfile>`, then removes it.

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

The `rustfmt.toml` is embedded in `src/main.rs`. Edit and reinstall to change it.

## License

MIT. See [LICENSE](LICENSE) for detail.
