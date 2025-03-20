use std::fs;
use std::path::PathBuf;
use toml::Table;

fn find_config_file() -> Option<PathBuf> {
    let mut current_dir = std::env::current_dir().ok()?;

    loop {
        let config_path = current_dir.join(".xenomorphrc");
        if config_path.exists() {
            return Some(config_path);
        }

        if !current_dir.pop() {
            break;
        }
    }

    None
}

pub fn workdir() -> Option<PathBuf> {
    let config = find_config_file();
    match config {
        Some(c) => Some(c.parent().unwrap().to_path_buf()),
        None => None,
    }
}

pub fn workdir_path(relative_path: &str) -> PathBuf {
    let workdir = workdir().expect("No workdir found");
    workdir.join(relative_path)
}

/// Loads the configuration from a .xenomorphrc file found in the directory hierarchy.
pub fn load_config() -> Option<Table> {
    let config_path = find_config_file()?;
    let content = fs::read_to_string(config_path).ok()?;
    content.parse::<Table>().ok()
}

pub fn load_config_key<'a>(key: &str) -> toml::Value {
    match load_config() {
        Some(conf) => get_config_value(&conf, key),
        None => panic!("No .xenomorphrc file found in directory hierarchy"),
    }
}

pub fn get_config_value<'a>(config: &'a Table, key: &str) -> toml::Value {
    if let Some(c) = config.get(key) {
        c.to_owned()
    } else {
        panic!("No {} configuration found in .xenomorphrc file", key);
    }
}
