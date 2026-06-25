use std::{
    env::{args, current_dir, temp_dir},
    fs::{read_dir, read_to_string},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use tempfile::{Builder, NamedTempFile};

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
        Err(_) => return ExitCode::FAILURE,
    };
    let conf_path = conf.path().to_owned();

    let forwarded_args: Vec<String> = args().skip(2).collect(); // skip "cargo myfmt"
    let cargo_fmt_args = cargo_fmt_args(&forwarded_args, &conf_path);

    let result = Command::new("cargo")
        .arg("+nightly")
        .arg("fmt")
        .args(cargo_fmt_args)
        .status();

    result.map_or(ExitCode::FAILURE, |s| {
        ExitCode::from(s.code().unwrap_or(1) as u8)
    })
}

fn rustfmt_config() -> std::io::Result<NamedTempFile> {
    let Some((root, gitattributes)) = current_dir().ok().and_then(find_git_root).and_then(|root| {
        let gitattributes = load_gitattributes(&root)?;
        Some((root, gitattributes))
    }) else {
        return config_file_in(temp_dir(), false, RUSTFMT_TOML);
    };

    let ignored = generated_rust_files(&root, &gitattributes);
    if ignored.is_empty() {
        return config_file_in(temp_dir(), false, RUSTFMT_TOML);
    }

    let config = format!(
        "{RUSTFMT_TOML}\nignore = [{}]\n",
        ignored
            .iter()
            .map(|path| format!("{:?}", path.display().to_string()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    config_file_in(root, true, &config)
}

fn config_file_in(dir: PathBuf, hidden: bool, config: &str) -> std::io::Result<NamedTempFile> {
    let prefix = if hidden {
        ".cargo-myfmt-"
    } else {
        "cargo-myfmt-"
    };
    let mut file = Builder::new()
        .prefix(prefix)
        .suffix("-rustfmt.toml")
        .tempfile_in(dir)?;

    file.write_all(config.as_bytes())?;
    file.flush()?;
    Ok(file)
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
            .map(|&idx| self.generated[idx])
            .unwrap_or(false)
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
    use super::*;

    #[test]
    fn appends_config_path_after_existing_separator() {
        let args = cargo_fmt_args(
            &["--".to_owned(), "--check".to_owned()],
            Path::new("/tmp/cfg"),
        );
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
}
