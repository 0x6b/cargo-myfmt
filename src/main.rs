use std::{
    collections::HashSet,
    env::{args, current_dir, temp_dir},
    fs::{read_dir, read_to_string},
    io::{Error, ErrorKind, Result, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use tempfile::{Builder, NamedTempFile};
use toml::{Table, Value, to_string_pretty};

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
    let conf = match rustfmt_config() {
        Ok(conf) => conf,
        Err(error) => {
            eprintln!("cargo-myfmt: failed to prepare rustfmt config: {error}");
            return ExitCode::FAILURE;
        }
    };
    let conf_path = conf.path().to_owned();

    let forwarded_args: Vec<String> = args().skip(2).collect(); // skip "cargo myfmt"
    let cargo_fmt_args = cargo_fmt_args(&forwarded_args, &conf_path);

    let result = Command::new("cargo")
        .arg("+nightly")
        .arg("fmt")
        .args(cargo_fmt_args)
        .status();

    result.map_or(ExitCode::FAILURE, |s| ExitCode::from(s.code().unwrap_or(1) as u8))
}

fn rustfmt_config() -> Result<NamedTempFile> {
    let root = current_dir().ok().and_then(find_git_root);
    let local_config = root.as_deref().map(load_local_rustfmt_config).transpose()?.flatten();
    let ignored = root
        .as_deref()
        .and_then(|root| load_gitattributes(root).map(|gitattributes| (root, gitattributes)))
        .map(|(root, gitattributes)| generated_rust_files(root, &gitattributes))
        .unwrap_or_default();
    let config = merged_rustfmt_config(
        local_config.as_ref().map(|config| config.content.as_str()),
        &ignored,
    )?;

    let config_dir = local_config
        .as_ref()
        .and_then(|config| config.path.parent().map(Path::to_owned))
        .or_else(|| (!ignored.is_empty()).then(|| root.clone()).flatten())
        .unwrap_or_else(temp_dir);
    let hidden = local_config.is_some() || !ignored.is_empty();

    config_file_in(config_dir, hidden, &config)
}

fn config_file_in(dir: PathBuf, hidden: bool, config: &str) -> Result<NamedTempFile> {
    let prefix = if hidden { ".cargo-myfmt-" } else { "cargo-myfmt-" };
    let mut file = Builder::new()
        .prefix(prefix)
        .suffix("-rustfmt.toml")
        .tempfile_in(dir)?;

    file.write_all(config.as_bytes())?;
    file.flush()?;
    Ok(file)
}

struct LocalRustfmtConfig {
    path: PathBuf,
    content: String,
}

fn load_local_rustfmt_config(root: &Path) -> Result<Option<LocalRustfmtConfig>> {
    for name in ["rustfmt.toml", ".rustfmt.toml"] {
        let path = root.join(name);
        if path.exists() {
            let content = read_to_string(&path)?;
            return Ok(Some(LocalRustfmtConfig { path, content }));
        }
    }

    Ok(None)
}

fn merged_rustfmt_config(local_config: Option<&str>, ignored: &[PathBuf]) -> Result<String> {
    let mut merged = parse_rustfmt_config(RUSTFMT_TOML)?;

    if let Some(local_config) = local_config {
        for (key, value) in parse_rustfmt_config(local_config)? {
            merged.insert(key, value);
        }
    }

    append_ignored_files(&mut merged, ignored);

    to_string_pretty(&merged).map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

fn parse_rustfmt_config(config: &str) -> Result<Table> {
    config
        .parse()
        .map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

fn append_ignored_files(config: &mut Table, ignored: &[PathBuf]) {
    if ignored.is_empty() {
        return;
    }

    let Some(ignore) = config.remove("ignore") else {
        config.insert("ignore".to_owned(), ignored_files_value(Vec::new(), ignored));
        return;
    };

    let Value::Array(existing) = ignore else {
        config.insert("ignore".to_owned(), ignore);
        return;
    };

    config.insert("ignore".to_owned(), ignored_files_value(existing, ignored));
}

fn ignored_files_value(mut existing: Vec<Value>, ignored: &[PathBuf]) -> Value {
    let mut seen: HashSet<String> =
        existing.iter().filter_map(Value::as_str).map(str::to_owned).collect();

    for path in ignored {
        let path = path.display().to_string();
        if seen.insert(path.clone()) {
            existing.push(Value::String(path));
        }
    }

    Value::Array(existing)
}

fn cargo_fmt_args(forwarded_args: &[String], conf: &Path) -> Vec<String> {
    let mut result = forwarded_args.to_vec();

    if !result.iter().any(|arg| arg == "--") {
        result.push("--".to_owned());
    }

    result.push("--config-path".to_owned());
    result.push(conf.display().to_string());

    result
}

fn find_git_root(mut path: PathBuf) -> Option<PathBuf> {
    loop {
        if path.join(".git").exists() {
            return Some(path);
        }

        if !path.pop() {
            return None;
        }
    }
}

struct GitAttrRule {
    glob: Glob,
    generated: bool,
}

struct GitAttrMatcher {
    globset: GlobSet,
    generated: Vec<bool>,
}

impl GitAttrMatcher {
    fn is_generated(&self, path: &str) -> bool {
        self.globset
            .matches(path)
            .last()
            .is_some_and(|&idx| self.generated[idx])
    }
}

fn load_gitattributes(root: &Path) -> Option<GitAttrMatcher> {
    let content = read_to_string(root.join(".gitattributes")).ok()?;
    let mut rules = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(pattern) = parts.next() else {
            continue;
        };

        let generated = parts.find_map(|attr| match attr {
            "linguist-generated" | "linguist-generated=true" => Some(true),
            "-linguist-generated" | "linguist-generated=false" | "!linguist-generated" => {
                Some(false)
            }
            _ => None,
        });

        let Some(generated) = generated else {
            continue;
        };

        if let Ok(glob) = Glob::new(pattern) {
            rules.push(GitAttrRule { glob, generated });
        }
    }

    if rules.is_empty() {
        return None;
    }

    let mut builder = GlobSetBuilder::new();
    let mut generated = Vec::with_capacity(rules.len());

    for rule in rules {
        builder.add(rule.glob);
        generated.push(rule.generated);
    }

    builder
        .build()
        .ok()
        .map(|globset| GitAttrMatcher { globset, generated })
}

fn generated_rust_files(root: &Path, gitattributes: &GitAttrMatcher) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_generated_rust_files(root, root, gitattributes, &mut files);
    files.sort();
    files
}

fn collect_generated_rust_files(
    root: &Path,
    dir: &Path,
    gitattributes: &GitAttrMatcher,
    files: &mut Vec<PathBuf>,
) {
    let Ok(entries) = read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            let name = entry.file_name();
            if name == ".git" || name == ".jj" || name == "target" {
                continue;
            }

            collect_generated_rust_files(root, &path, gitattributes, files);
            continue;
        }

        if !file_type.is_file() || path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }

        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");

        if gitattributes.is_generated(&relative) {
            files.push(relative.into());
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    use toml::Table;

    use super::*;

    #[test]
    fn appends_config_path_after_existing_separator() {
        let args = cargo_fmt_args(&["--".to_owned(), "--check".to_owned()], Path::new("/tmp/cfg"));
        assert_eq!(args, ["--", "--check", "--config-path", "/tmp/cfg"]);
    }

    #[test]
    fn adds_separator_when_forwarding_only_cargo_fmt_args() {
        let args = cargo_fmt_args(&["--all".to_owned()], Path::new("/tmp/cfg"));
        assert_eq!(args, ["--all", "--", "--config-path", "/tmp/cfg"]);
    }

    #[test]
    fn later_gitattributes_rule_overrides_earlier_rule() {
        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("src/*.rs").unwrap());
        builder.add(Glob::new("src/manual.rs").unwrap());

        let matcher = GitAttrMatcher {
            globset: builder.build().unwrap(),
            generated: vec![true, false],
        };

        assert!(matcher.is_generated("src/generated.rs"));
        assert!(!matcher.is_generated("src/manual.rs"));
    }

    #[test]
    fn basename_gitattributes_pattern_matches_nested_file() {
        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("*.rs").unwrap());

        let matcher = GitAttrMatcher {
            globset: builder.build().unwrap(),
            generated: vec![true],
        };

        assert!(matcher.is_generated("src/generated.rs"));
    }

    #[test]
    fn local_config_overrides_embedded_defaults() {
        let config = merged_rustfmt_config(Some("max_width = 120\n"), &[]).unwrap();
        let parsed: Table = config.parse().unwrap();

        assert_eq!(parsed["max_width"].as_integer(), Some(120));
        assert_eq!(parsed["chain_width"].as_integer(), Some(70));
    }

    #[test]
    fn generated_ignores_are_added_to_local_ignore() {
        let config = merged_rustfmt_config(
            Some("ignore = [\"src/local.rs\"]\n"),
            &[PathBuf::from("src/generated.rs"), PathBuf::from("src/local.rs")],
        )
        .unwrap();
        let parsed: Table = config.parse().unwrap();
        let ignored: Vec<_> = parsed["ignore"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();

        assert_eq!(ignored, ["src/local.rs", "src/generated.rs"]);
    }
}
