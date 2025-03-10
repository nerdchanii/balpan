// This is referred from the helix codebase: 
// https://github.com/helix-editor/helix/blob/master/helix-loader/src/lib.rs 
pub mod config;
pub mod grammar;
pub mod analyzer;
pub mod utils;
pub mod tree_sitter_extended;
pub mod language;

use etcetera::base_strategy::{choose_base_strategy, BaseStrategy};
use std::path::{Path, PathBuf};
use toml::{map::Map, Value};

static RUNTIME_DIRS: once_cell::sync::Lazy<Vec<PathBuf>> =
    once_cell::sync::Lazy::new(prioritize_runtime_dirs);

static CONFIG_FILE: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();

pub fn initialize_config_file(specified_file: Option<PathBuf>) {
    let config_file = specified_file.unwrap_or_else(|| {
        let config_dir = config_dir();

        if !config_dir.exists() {
            std::fs::create_dir_all(&config_dir).ok();
        }

        config_dir.join("config.toml")
    });

    // We should only initialize this value once.
    CONFIG_FILE.set(config_file).ok();
}

/// A list of runtime directories from highest to lowest priority
///
/// The priority is:
///
/// 1. sibling directory to `CARGO_MANIFEST_DIR` (if environment variable is set)
/// 2. subdirectory of user config directory (always included)
/// 3. `BALPAN_RUNTIME` (if environment variable is set)
/// 4. subdirectory of path to balpan executable (always included)
///
/// Postcondition: returns at least two paths (they might not exist).
fn prioritize_runtime_dirs() -> Vec<PathBuf> {
    const RT_DIR: &str = "runtime";
    // Adding higher priority first
    let mut rt_dirs = Vec::new();
    if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        // this is the directory of the crate being run by cargo, we need the workspace path so we take the parent
        let path = PathBuf::from(dir).parent().unwrap().join(RT_DIR);
        log::debug!("runtime dir: {}", path.to_string_lossy());
        rt_dirs.push(path);
    }

    let conf_rt_dir = config_dir().join(RT_DIR);
    rt_dirs.push(conf_rt_dir);

    if let Ok(dir) = std::env::var("BALPAN_RUNTIME") {
        rt_dirs.push(dir.into());
    }

    // fallback to location of the executable being run
    // canonicalize the path in case the executable is symlinked
    let exe_rt_dir = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .and_then(|path| path.parent().map(|path| path.to_path_buf().join(RT_DIR)))
        .unwrap();
    rt_dirs.push(exe_rt_dir);
    rt_dirs
}

/// Runtime directories ordered from highest to lowest priority
///
/// All directories should be checked when looking for files.
///
/// Postcondition: returns at least one path (it might not exist).
pub fn runtime_dirs() -> &'static [PathBuf] {
    &RUNTIME_DIRS
}

/// Find file with path relative to runtime directory
///
/// `rel_path` should be the relative path from within the `runtime/` directory.
/// The valid runtime directories are searched in priority order and the first
/// file found to exist is returned, otherwise None.
fn find_runtime_file(rel_path: &Path) -> Option<PathBuf> {
    RUNTIME_DIRS.iter().find_map(|rt_dir| {
        let path = rt_dir.join(rel_path);
        if path.exists() {
            return Some(path);
        }

        None
    })
}

/// Find file with path relative to runtime directory
///
/// `rel_path` should be the relative path from within the `runtime/` directory.
/// The valid runtime directories are searched in priority order and the first
/// file found to exist is returned, otherwise the path to the final attempt
/// that failed.
pub fn runtime_file(rel_path: &Path) -> PathBuf {
    find_runtime_file(rel_path).unwrap_or_else(|| {
        RUNTIME_DIRS
            .last()
            .map(|dir| dir.join(rel_path))
            .unwrap_or_default()
    })
}

enum StrategyType {
    Config,
    Cache,
}

fn get_dir(target: StrategyType) -> PathBuf {
    let target_str = match target {
        StrategyType::Config => "config",
        StrategyType::Cache => "cache",
    };

    // Check if the directory override environment variable is set
    if let Ok(dir) = std::env::var(format!("BALPAN_{}_DIR", target_str.to_uppercase())) {
        return PathBuf::from(dir);
    }

    let strategy = choose_base_strategy()
        .unwrap_or_else(|_| panic!("Unable to find the {target_str} directory strategy!"));
    let mut path = match target {
        StrategyType::Config => strategy.config_dir(),
        StrategyType::Cache => strategy.cache_dir(),
    };

    path.push("balpan");

    path
}

pub fn config_dir() -> PathBuf {
    get_dir(StrategyType::Config)
}

pub fn cache_dir() -> PathBuf {
    get_dir(StrategyType::Cache)
}

pub fn config_file() -> PathBuf {
    CONFIG_FILE
        .get()
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| config_dir().join("config.toml"))
}

pub fn workspace_config_file() -> PathBuf {
    find_workspace().0.join(".balpan").join("config.toml")
}

pub fn lang_config_file() -> PathBuf {
    config_dir().join("languages.toml")
}

pub fn log_file() -> PathBuf {
    cache_dir().join("balpan.log")
}

fn get_name(v: &Value) -> Option<&str> {
    v.get("name").and_then(Value::as_str)
}

/// Merge two TOML documents, merging values from `right` onto `left`
///
/// When an array exists in both `left` and `right`, `right`'s array is
/// used. When a table exists in both `left` and `right`, the merged table
/// consists of all keys in `left`'s table unioned with all keys in `right`
/// with the values of `right` being merged recursively onto values of
/// `left`.
///
/// `merge_toplevel_arrays` controls whether a top-level array in the TOML
/// document is merged instead of overridden. This is useful for TOML
/// documents that use a top-level array of values like the `languages.toml`,
/// where one usually wants to override or add to the array instead of
/// replacing it altogether.
pub fn merge_toml_values(left: toml::Value, right: toml::Value, merge_depth: usize) -> toml::Value {
    match (left, right) {
        (Value::Array(left_items), Value::Array(right_items)) => {
            toml_array_value(merge_depth, left_items, right_items)
        }
        (Value::Table(left_map), Value::Table(right_map)) => {
            toml_table_value(merge_depth, left_map, right_map)
        }
        // Catch everything else we didn't handle, and use the right value
        (_, value) => value,
    }
}

fn toml_array_value(
    merge_depth: usize,
    mut left_items: Vec<Value>,
    right_items: Vec<Value>,
) -> toml::Value {
    // The top-level arrays should be merged but nested arrays should
    // act as overrides. For the `languages.toml` config, this means
    // that you can specify a sub-set of languages in an overriding
    // `languages.toml` but that nested arrays like Language Server
    // arguments are replaced instead of merged.
    if merge_depth == 0 {
        return Value::Array(right_items);
    }

    left_items.reserve(right_items.len());

    for r_val in right_items {
        let l_val = get_name(&r_val)
            .and_then(|r_name| left_items.iter().position(|v| get_name(v) == Some(r_name)))
            .map(|l_pos| left_items.remove(l_pos));

        let m_val = match l_val {
            Some(l) => merge_toml_values(l, r_val, merge_depth - 1),
            None => r_val,
        };

        left_items.push(m_val);
    }

    Value::Array(left_items)
}

fn toml_table_value(
    merge_depth: usize,
    mut left_map: Map<String, Value>,
    right_map: Map<String, Value>,
) -> toml::Value {
    if merge_depth == 0 {
        return Value::Table(right_map);
    }

    for (r_name, r_val) in right_map {
        match left_map.remove(&r_name) {
            Some(l_val) => {
                let merged_val = merge_toml_values(l_val, r_val, merge_depth - 1);
                left_map.insert(r_name, merged_val);
            }
            None => {
                left_map.insert(r_name, r_val);
            }
        }
    }

    Value::Table(left_map)
}

/// Finds the current workspace folder.
/// Used as a ceiling dir for LSP root resolution, the filepicker and potentially as a future filewatching root
///
/// This function starts searching the FS upward from the CWD
/// and returns the first directory that contains either `.git` or `.balpan`.
/// If no workspace was found returns (CWD, true).
/// Otherwise (workspace, false) is returned
pub fn find_workspace() -> (PathBuf, bool) {
    let current_dir = std::env::current_dir().expect("unable to determine current directory");
    for ancestor in current_dir.ancestors() {
        if ancestor.join(".git").exists() || ancestor.join(".balpan").exists() {
            return (ancestor.to_owned(), false);
        }
    }

    (current_dir, true)
}

#[cfg(test)]
mod merge_toml_tests {
    use std::str;

    use super::merge_toml_values;
    use toml::Value;

    #[test]
    fn language_toml_map_merges() {
        const USER: &str = r#"
        [[language]]
        name = "nix"
        test = "bbb"
        indent = { tab-width = 4, unit = "    ", test = "aaa" }
        "#;

        let base = include_bytes!("../languages.toml");
        let base = str::from_utf8(base).expect("Couldn't parse built-in languages config");
        let base: Value = toml::from_str(base).expect("Couldn't parse built-in languages config");
        let user: Value = toml::from_str(USER).unwrap();

        let merged = merge_toml_values(base, user, 3);
        let languages = merged.get("language").unwrap().as_array().unwrap();
        let nix = languages
            .iter()
            .find(|v| v.get("name").unwrap().as_str().unwrap() == "nix")
            .unwrap();
        let nix_indent = nix.get("indent").unwrap();

        // We changed tab-width and unit in indent so check them if they are the new values
        assert_eq!(
            nix_indent.get("tab-width").unwrap().as_integer().unwrap(),
            4
        );
        assert_eq!(nix_indent.get("unit").unwrap().as_str().unwrap(), "    ");
        // We added a new keys, so check them
        assert_eq!(nix.get("test").unwrap().as_str().unwrap(), "bbb");
        assert_eq!(nix_indent.get("test").unwrap().as_str().unwrap(), "aaa");
        // We didn't change comment-token so it should be same
        assert_eq!(nix.get("comment-token").unwrap().as_str().unwrap(), "#");
    }

    #[test]
    fn language_toml_nested_array_merges() {
        const USER: &str = r#"
        [[language]]
        name = "typescript"
        language-server = { command = "deno", args = ["lsp"] }
        "#;

        let base = include_bytes!("../languages.toml");
        let base = str::from_utf8(base).expect("Couldn't parse built-in languages config");
        let base: Value = toml::from_str(base).expect("Couldn't parse built-in languages config");
        let user: Value = toml::from_str(USER).unwrap();

        let merged = merge_toml_values(base, user, 3);
        let languages = merged.get("language").unwrap().as_array().unwrap();
        let ts = languages
            .iter()
            .find(|v| v.get("name").unwrap().as_str().unwrap() == "typescript")
            .unwrap();
        assert_eq!(
            ts.get("language-server")
                .unwrap()
                .get("args")
                .unwrap()
                .as_array()
                .unwrap(),
            &vec![Value::String("lsp".into())]
        )
    }

    #[test]
    fn allow_env_variable_override() {
        const USER: &str = r#"
        [[language]]
        name = "typescript"
        language-server = { command = "deno", args = ["lsp"] }
        "#;

        let base = include_bytes!("../languages.toml");
        let base = str::from_utf8(base).expect("Couldn't parse built-in languages config");
        let base: Value = toml::from_str(base).expect("Couldn't parse built-in languages config");
        let user: Value = toml::from_str(USER).unwrap();

        std::env::set_var("BALPAN_CONFIG_DIR", "/tmp");
        let merged = merge_toml_values(base, user, 3);
        std::env::remove_var("BALPAN_CONFIG_DIR");

        let languages = merged.get("language").unwrap().as_array().unwrap();
        let ts = languages
            .iter()
            .find(|v| v.get("name").unwrap().as_str().unwrap() == "typescript")
            .unwrap();
        assert_eq!(
            ts.get("language-server")
                .unwrap()
                .get("args")
                .unwrap()
                .as_array()
                .unwrap(),
            &vec![Value::String("lsp".into())]
        )
    }
}
