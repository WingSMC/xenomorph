use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use xenomorph_common::config::Config;
use xenomorph_common::formatter::format_xenomorph;

pub fn run_format(args: &[String]) -> Result<FormatSummary, String> {
    if args.len() > 1 {
        return Err("Usage: xeno format [file.xen]".to_string());
    }

    let config = Config::get();
    let paths = match args.first() {
        Some(path) => {
            let path = PathBuf::from(path);
            let path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .map_err(|error| format!("Unable to determine current directory: {error}"))?
                    .join(path)
            };
            validate_single_file(&path)?;
            vec![path]
        }
        None => collect_xenomorph_files(&config.workdir)
            .map_err(|error| format!("Unable to scan workspace: {error}"))?,
    };

    let mut summary = FormatSummary::default();
    for path in paths {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("Unable to read '{}': {error}", path.display()))?;
        let formatted = format_xenomorph(&source, &config.formatter);
        summary.files += 1;
        if formatted == source {
            continue;
        }
        fs::write(&path, formatted)
            .map_err(|error| format!("Unable to write '{}': {error}", path.display()))?;
        summary.changed += 1;
    }

    Ok(summary)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatSummary {
    pub files: usize,
    pub changed: usize,
}

fn validate_single_file(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("'{}' is not a file.", path.display()));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("xen") {
        return Err(format!("'{}' is not a .xen file.", path.display()));
    }
    Ok(())
}

fn collect_xenomorph_files(workspace: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_xenomorph_files_from(workspace, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_xenomorph_files_from(directory: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if is_ignored_directory(&entry.file_name().to_string_lossy()) {
                continue;
            }
            collect_xenomorph_files_from(&path, files)?;
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("xen")
        {
            files.push(path);
        }
    }

    Ok(())
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(name, ".git" | ".xenomorph" | "node_modules" | "target")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("xenomorph-{name}-{suffix}"));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    #[test]
    fn workspace_collection_is_recursive_sorted_and_ignores_build_directories() {
        let root = temporary_directory("format-files");
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).expect("nested source directory should be created");
        fs::create_dir_all(root.join("target/generated"))
            .expect("ignored directory should be created");
        fs::write(root.join("z.xen"), "type Z = string;").expect("fixture should be written");
        fs::write(nested.join("a.xen"), "type A = string;").expect("fixture should be written");
        fs::write(nested.join("ignored.txt"), "text").expect("fixture should be written");
        fs::write(
            root.join("target/generated/ignored.xen"),
            "type Ignored = string;",
        )
        .expect("fixture should be written");

        let files = collect_xenomorph_files(&root).expect("workspace should be scanned");

        assert_eq!(files, vec![nested.join("a.xen"), root.join("z.xen")]);
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn validates_xenomorph_file_inputs() {
        let root = temporary_directory("format-validation");
        let xen = root.join("model.xen");
        let text = root.join("model.txt");
        fs::write(&xen, "type Model = string;").expect("fixture should be written");
        fs::write(&text, "text").expect("fixture should be written");

        assert_eq!(validate_single_file(&xen), Ok(()));
        assert!(validate_single_file(&text).is_err());
        assert!(validate_single_file(&root.join("missing.xen")).is_err());
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }
}
