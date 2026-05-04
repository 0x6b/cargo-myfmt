use std::{
    env::{args, temp_dir},
    fs::{remove_file, write},
    process::{Command, ExitCode},
};

const RUSTFMT_TOML: &str = r#"# https://github.com/rust-lang/rustfmt/blob/master/Configurations.md
chain_width = 70
comment_width = 100
group_imports = "StdExternalCrate"
imports_granularity = "Crate"
max_width = 100
merge_derives = false
struct_lit_width = 50
use_field_init_shorthand = true
use_small_heuristics = "Max"
wrap_comments = true
"#;

fn main() -> ExitCode {
    let conf = temp_dir().join("rustfmt.toml");

    if write(&conf, RUSTFMT_TOML).is_err() {
        return ExitCode::FAILURE;
    }

    let result = Command::new("cargo")
        .arg("+nightly")
        .arg("fmt")
        .args(args().skip(2)) // skip "cargo myfmt"
        .arg("--")
        .arg("--config-path")
        .arg(&conf)
        .status();

    let _ = remove_file(&conf);

    result.map_or(ExitCode::FAILURE, |s| ExitCode::from(s.code().unwrap_or(1) as u8))
}
