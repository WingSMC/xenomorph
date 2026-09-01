use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use xenomorph_common::config::{
    default_config_toml, graft_config_toml, RC_SCHEMA_RELATIVE_PATH, WORKSPACE_CONFIG_FILE,
};

const GITIGNORE_ENTRY: &str = ".xenomorph/";

pub fn run_init() -> Result<(), String> {
    let original_directory = std::env::current_dir()
        .map_err(|error| format!("Unable to determine current directory: {error}"))?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();

    let folder_name = prompt(&mut input, &mut output, "Folder name: ")?;
    let folder_name = validate_folder_name(&folder_name)?;
    let project_directory = original_directory.join(&folder_name);
    if project_directory.exists() {
        return Err(format!(
            "Cannot initialize '{}': the path already exists.",
            project_directory.display()
        ));
    }

    let parent_repository = discover_git_repository(&original_directory)?;
    fs::create_dir(&project_directory).map_err(|error| {
        format!(
            "Unable to create project directory '{}': {error}",
            project_directory.display()
        )
    })?;
    std::env::set_current_dir(&project_directory).map_err(|error| {
        format!(
            "Unable to enter project directory '{}': {error}",
            project_directory.display()
        )
    })?;

    run_git(
        &project_directory,
        ["init"],
        "initialize the Git repository",
    )?;
    let remote = prompt(
        &mut input,
        &mut output,
        "Repository remote (leave blank to skip): ",
    )?;
    let remote = remote.trim();
    if !remote.is_empty() {
        run_git(
            &project_directory,
            ["remote", "add", "origin", remote],
            "add the Git remote",
        )?;
    }
    drop(input);
    drop(output);

    write_default_config(&project_directory)?;
    ensure_gitignore_entry(&project_directory.join(".gitignore"))?;
    generate_schema(&project_directory)?;

    if let Some(parent_repository) = parent_repository {
        add_parent_submodule(
            &parent_repository,
            &project_directory,
            (!remote.is_empty()).then_some(remote),
        )?;
        println!(
            "✓ Added '{}' to '{}' as a Git submodule",
            folder_name.display(),
            parent_repository.display()
        );
    }

    println!(
        "✓ Initialized Xenomorph project in {}",
        project_directory.display()
    );
    Ok(())
}

pub fn run_graft(repository_url: &str) -> Result<(), String> {
    let original_directory = std::env::current_dir()
        .map_err(|error| format!("Unable to determine current directory: {error}"))?;
    let repository_url = repository_url.trim();
    if repository_url.is_empty() {
        return Err("Repository URL cannot be empty.".to_string());
    }
    if discover_git_repository(&original_directory)?.is_none() {
        return Err(format!(
            "Cannot graft into '{}': it is not inside a Git repository.",
            original_directory.display()
        ));
    }

    let config_path = original_directory.join(WORKSPACE_CONFIG_FILE);
    if config_path.exists() {
        return Err(format!(
            "Cannot graft into '{}': '{}' already exists.",
            original_directory.display(),
            WORKSPACE_CONFIG_FILE
        ));
    }

    let default_name = repository_name(repository_url)?;
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let alias = prompt(
        &mut input,
        &mut output,
        &format!("Name/path alias [{default_name}]: "),
    )?;
    drop(input);
    drop(output);

    let alias = if alias.trim().is_empty() {
        PathBuf::from(default_name)
    } else {
        validate_graft_path(&alias)?
    };
    let grafted_directory = original_directory.join(&alias);
    if grafted_directory.exists() {
        return Err(format!(
            "Cannot graft '{}': the path already exists.",
            grafted_directory.display()
        ));
    }

    let alias_argument = git_path(&alias);
    run_git(
        &original_directory,
        [
            "submodule",
            "add",
            "--",
            repository_url,
            alias_argument.as_str(),
        ],
        "add the Xenomorph Git submodule",
    )?;

    let grafted_config = grafted_directory.join(WORKSPACE_CONFIG_FILE);
    if !grafted_config.is_file() {
        return Err(format!(
            "Grafted repository does not contain '{}' at its root.",
            WORKSPACE_CONFIG_FILE
        ));
    }

    generate_schema(&grafted_directory)?;
    write_graft_config(&config_path, &alias)?;
    run_git(
        &original_directory,
        ["add", "--", WORKSPACE_CONFIG_FILE],
        "stage the grafted Xenomorph config",
    )?;

    println!("✓ Grafted '{}' as '{}'", repository_url, git_path(&alias));
    Ok(())
}

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    message: &str,
) -> Result<String, String> {
    output
        .write_all(message.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|error| format!("Unable to write prompt: {error}"))?;

    let mut value = String::new();
    let bytes_read = input
        .read_line(&mut value)
        .map_err(|error| format!("Unable to read input: {error}"))?;
    if bytes_read == 0 {
        return Err(format!("No value was provided for '{message}'."));
    }
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

fn validate_folder_name(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let path = Path::new(value);
    let mut components = path.components();
    let valid = !value.is_empty()
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none();

    if !valid {
        return Err("Folder name must be one non-empty relative path component.".to_string());
    }
    Ok(path.to_path_buf())
}

fn validate_graft_path(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(
            "Name/path alias must be a non-empty relative path without '.' or '..'.".to_string(),
        );
    }
    Ok(path.to_path_buf())
}

fn repository_name(repository_url: &str) -> Result<String, String> {
    let repository_url = repository_url.trim().trim_end_matches(['/', '\\']);
    let repository_path = match repository_url.split_once("://") {
        Some((_, location)) => location.split_once('/').map(|(_, path)| path).unwrap_or(""),
        None => repository_url,
    };
    let segment = repository_path
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or_default();
    let name = segment.strip_suffix(".git").unwrap_or(segment);
    if name.is_empty() || matches!(name, "." | "..") {
        return Err(format!(
            "Unable to derive a local folder name from repository URL '{repository_url}'."
        ));
    }
    Ok(name.to_string())
}

fn write_default_config(project_directory: &Path) -> Result<(), String> {
    let config_path = project_directory.join(WORKSPACE_CONFIG_FILE);
    fs::write(&config_path, default_config_toml()?)
        .map_err(|error| format!("Unable to write '{}': {error}", config_path.display()))
}

fn write_graft_config(config_path: &Path, grafted_project: &Path) -> Result<(), String> {
    fs::write(config_path, graft_config_toml(grafted_project)?)
        .map_err(|error| format!("Unable to write '{}': {error}", config_path.display()))
}

fn ensure_gitignore_entry(path: &Path) -> Result<(), String> {
    let existing = match fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("Unable to read '{}': {error}", path.display())),
    };
    if existing.lines().any(|line| line.trim() == GITIGNORE_ENTRY) {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("Unable to open '{}': {error}", path.display()))?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        file.write_all(b"\n")
            .map_err(|error| format!("Unable to update '{}': {error}", path.display()))?;
    }
    writeln!(file, "{GITIGNORE_ENTRY}")
        .map_err(|error| format!("Unable to update '{}': {error}", path.display()))
}

fn generate_schema(project_directory: &Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Unable to locate the xeno executable: {error}"))?;
    let config_path = project_directory.join(WORKSPACE_CONFIG_FILE);
    let status = Command::new(&executable)
        .arg("schema")
        .arg("--config")
        .arg(&config_path)
        .current_dir(project_directory)
        .status()
        .map_err(|error| format!("Unable to run 'xeno schema': {error}"))?;
    if !status.success() {
        return Err(format!(
            "'xeno schema' failed with {}.",
            exit_status_description(status.code())
        ));
    }

    let schema_path = project_directory.join(RC_SCHEMA_RELATIVE_PATH);
    if !schema_path.is_file() {
        return Err(format!(
            "'xeno schema' succeeded but did not create '{}'.",
            schema_path.display()
        ));
    }
    Ok(())
}

fn discover_git_repository(directory: &Path) -> Result<Option<PathBuf>, String> {
    let output = git_output(
        directory,
        ["rev-parse", "--show-toplevel"],
        "inspect the parent Git repository",
    )?;
    if !output.status.success() {
        return Ok(None);
    }

    let root = String::from_utf8(output.stdout)
        .map_err(|_| "Git returned a non-UTF-8 repository path.".to_string())?;
    let root = PathBuf::from(root.trim());
    normalized_path(&root).map(Some)
}

fn add_parent_submodule(
    parent_repository: &Path,
    project_directory: &Path,
    remote: Option<&str>,
) -> Result<(), String> {
    run_git(
        project_directory,
        ["add", "--", WORKSPACE_CONFIG_FILE, ".gitignore"],
        "stage the initial project files",
    )?;
    run_git(
        project_directory,
        ["commit", "-m", "Initial Xenomorph project"],
        "create the initial project commit required by Git submodules",
    )?;

    let project_directory = normalized_path(project_directory)?;
    let relative_path = project_directory
        .strip_prefix(parent_repository)
        .map_err(|_| {
            format!(
                "Project directory '{}' is outside parent Git repository '{}'.",
                project_directory.display(),
                parent_repository.display()
            )
        })?;
    let relative_path = git_path(relative_path);
    let local_url = format!("./{relative_path}");
    let submodule_url = remote.unwrap_or(&local_url);
    run_git(
        parent_repository,
        ["submodule", "add", "--", submodule_url, &relative_path],
        "add the project as a Git submodule",
    )
}

fn normalized_path(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Unable to resolve path '{}': {error}", path.display()))?;

    #[cfg(windows)]
    {
        let value = canonical.to_string_lossy();
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{unc}")));
        }
        if let Some(local) = value.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(local));
        }
    }

    Ok(canonical)
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn run_git<I, S>(directory: &Path, args: I, operation: &str) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(directory, args, operation)?;
    if output.status.success() {
        return Ok(());
    }

    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if detail.is_empty() {
        format!(
            "Git exited with {}",
            exit_status_description(output.status.code())
        )
    } else {
        detail
    };
    Err(format!("Unable to {operation}: {detail}"))
}

fn git_output<I, S>(directory: &Path, args: I, operation: &str) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .map_err(|error| format!("Unable to {operation}: failed to run Git: {error}"))
}

fn exit_status_description(code: Option<i32>) -> String {
    code.map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "an unknown process error".to_string())
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
        let path = std::env::temp_dir().join(format!("xenomorph-init-{name}-{suffix}"));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

    #[test]
    fn default_config_contains_the_schema_directive() {
        let config = default_config_toml().expect("default config should serialize");

        assert!(config.starts_with("#:schema ./.xenomorph/xenomorph.schema.json\n\n"));
        assert!(config.ends_with('\n'));
    }

    #[test]
    fn folder_name_must_be_one_relative_component() {
        assert_eq!(
            validate_folder_name("models").unwrap(),
            PathBuf::from("models")
        );
        assert!(validate_folder_name("").is_err());
        assert!(validate_folder_name(".").is_err());
        assert!(validate_folder_name("..").is_err());
        assert!(validate_folder_name("models/api").is_err());
        assert!(validate_folder_name("/models").is_err());
    }

    #[test]
    fn graft_alias_accepts_safe_nested_relative_paths() {
        assert_eq!(
            validate_graft_path("schemas/linked-schema").unwrap(),
            PathBuf::from("schemas/linked-schema")
        );
        assert!(validate_graft_path("").is_err());
        assert!(validate_graft_path(".").is_err());
        assert!(validate_graft_path("..").is_err());
        assert!(validate_graft_path("schemas/../linked-schema").is_err());
        assert!(validate_graft_path("/schemas/linked-schema").is_err());
    }

    #[test]
    fn repository_name_supports_https_ssh_and_git_suffixes() {
        assert_eq!(
            repository_name("https://example.com/team/tda-schemas").unwrap(),
            "tda-schemas"
        );
        assert_eq!(
            repository_name("https://example.com/team/tda-schemas.git/").unwrap(),
            "tda-schemas"
        );
        assert_eq!(
            repository_name("git@example.com:team/tda-schemas.git").unwrap(),
            "tda-schemas"
        );
        assert!(repository_name("https://example.com/").is_err());
    }

    #[test]
    fn prompt_trims_only_the_line_ending() {
        let mut input = io::Cursor::new(b"project name\r\n");
        let mut output = Vec::new();

        let value = prompt(&mut input, &mut output, "Folder name: ").unwrap();

        assert_eq!(value, "project name");
        assert_eq!(output, b"Folder name: ");
    }

    #[test]
    fn gitignore_is_created_and_not_duplicated() {
        let root = temporary_directory("gitignore-new");
        let gitignore = root.join(".gitignore");

        ensure_gitignore_entry(&gitignore).unwrap();
        ensure_gitignore_entry(&gitignore).unwrap();

        assert_eq!(fs::read_to_string(&gitignore).unwrap(), ".xenomorph/\n");
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }

    #[test]
    fn gitignore_entry_is_appended_on_a_new_line() {
        let root = temporary_directory("gitignore-existing");
        let gitignore = root.join(".gitignore");
        fs::write(&gitignore, "target/").expect("fixture should be written");

        ensure_gitignore_entry(&gitignore).unwrap();

        assert_eq!(
            fs::read_to_string(&gitignore).unwrap(),
            "target/\n.xenomorph/\n"
        );
        fs::remove_dir_all(root).expect("temporary directory should be removed");
    }
}
