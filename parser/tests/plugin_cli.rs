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
