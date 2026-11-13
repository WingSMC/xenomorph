use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn explicit_abstract_schema_config_collects_metadata_without_initialization() {
    let directory = std::env::temp_dir().join(format!(
        "xenomorph-schema-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let libraries = Path::new(env!("CARGO_BIN_EXE_xeno")).parent().unwrap();
    let config = directory.join("xenomorph.toml");
    fs::write(&config, format!(
        "abstract = true\n[plugins]\npath = {}\nplugins = [\"xenomorph_typescript\", \"xenomorph_test_plugin\"]\n",
        serde_json::to_string(libraries.to_str().unwrap()).unwrap(),
    )).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
        .args(["schema", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.join(".xenomorph").join("xenomorph.schema.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema["properties"]["plugins"]["properties"]["typescript"]["type"],
        "object"
    );
    assert!(
        !directory.join("events").exists(),
        "Schema discovery initialized a runtime plugin"
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn file_type_requires_config_and_blocks_output_without_it() {
    let directory = std::env::temp_dir().join(format!(
        "xenomorph-file-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    fs::create_dir(&directory).unwrap();
    let libraries = Path::new(env!("CARGO_BIN_EXE_xeno")).parent().unwrap();
    let config_path = directory.join("xenomorph.toml");
    let source = "pub type Upload = file; pub type Request = { upload: Upload, optional: ?file, uploads: file[] };";
    fs::write(directory.join("index.xen"), source).unwrap();
    for (plugin, settings, expected, guidance) in [
        (
            "java",
            "file_type = 'java.io.InputStream'",
            "Request.java",
            "[plugins.java]",
        ),
        (
            "pydantic",
            "file_type = { module = 'pydantic', name = 'Base64Bytes' }",
            "index.py",
            "[plugins.pydantic.file_type]",
        ),
    ] {
        let config = format!(
            "[parser]\nentry = 'index'\n[plugins]\npath = {}\nplugins = ['xenomorph_{plugin}']\n[plugins.{plugin}]\n",
            serde_json::to_string(libraries.to_str().unwrap()).unwrap(),
        );
        fs::write(&config_path, &config).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
            .arg("generate")
            .current_dir(&directory)
            .output()
            .unwrap();
        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.status.success(), "{diagnostics}");
        assert!(diagnostics.contains(guidance), "{diagnostics}");
        assert!(
            !directory.join(expected).exists(),
            "missing mapping wrote {expected}"
        );
        // Graph loading uses the same analyzer capability as the LSP.
        let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
            .args(["graph", "--json"])
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(guidance),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!directory.join(expected).exists());
        fs::write(&config_path, format!("{config}file_type = 42\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
            .arg("generate")
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            !directory.join(expected).exists(),
            "invalid mapping wrote {expected}"
        );
        fs::write(&config_path, format!("{config}{settings}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
            .arg("generate")
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(directory.join(expected).exists());
        let output = Command::new(env!("CARGO_BIN_EXE_xeno"))
            .args(["graph", "--json"])
            .current_dir(&directory)
            .output()
            .unwrap();
        let graph: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(graph["modules"][0]["diagnostics"]["errors"], 0, "{graph}");
    }
    fs::remove_dir_all(directory).unwrap();
}
