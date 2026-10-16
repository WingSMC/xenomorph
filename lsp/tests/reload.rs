use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Server {
    child: Child,
    input: Option<ChildStdin>,
    messages: Receiver<Value>,
    pending_messages: Vec<Value>,
    directory: PathBuf,
}

impl Server {
    fn start(directory: PathBuf) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xenomorph_lsp"))
            .current_dir(&directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length: ") {
                        length = Some(value.trim().parse::<usize>().unwrap());
                    }
                }
                let mut bytes = vec![0; length.expect("LSP content length")];
                reader.read_exact(&mut bytes).unwrap();
                if sender
                    .send(serde_json::from_slice(&bytes).unwrap())
                    .is_err()
                {
                    return;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            messages,
            pending_messages: Vec::new(),
            directory,
        }
    }

    fn send(&mut self, message: Value) {
        let bytes = serde_json::to_vec(&message).unwrap();
        let input = self.input.as_mut().unwrap();
        write!(input, "Content-Length: {}\r\n\r\n", bytes.len()).unwrap();
        input.write_all(&bytes).unwrap();
        input.flush().unwrap();
    }

    fn request(&mut self, id: u32, method: &str, params: Value) -> Value {
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(message);
        self.wait_for_message(|message| message.get("id") == Some(&json!(id)))
    }

    fn notification(&mut self, method: &str, params: Value) {
        let mut message = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(message);
    }

    fn wait_for_message(&mut self, matches: impl Fn(&Value) -> bool) -> Value {
        if let Some(index) = self.pending_messages.iter().position(&matches) {
            return self.pending_messages.remove(index);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if matches(&message) {
                return message;
            }
            self.pending_messages.push(message);
        }
    }

    fn wait_for_log(&mut self, text: &str) {
        self.wait_for_message(|message| {
            message["method"] == "window/logMessage"
                && message["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(text))
        });
    }

    fn diagnostics(&mut self, uri: &tower_lsp::lsp_types::Url) -> Value {
        self.wait_for_message(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri.as_str()
        })["params"]["diagnostics"]
            .clone()
    }

    fn open(&mut self, uri: &tower_lsp::lsp_types::Url, text: &str) {
        self.notification("textDocument/didOpen", json!({
            "textDocument": { "uri": uri, "languageId": "xenomorph", "version": 1, "text": text }
        }));
    }

    fn change(&mut self, uri: &tower_lsp::lsp_types::Url, version: u32, text: &str) {
        self.notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }),
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn config(directory: &Path, plugins: &[&str]) {
    let libraries = Path::new(env!("CARGO_BIN_EXE_xenomorph_lsp"))
        .parent()
        .unwrap();
    fs::write(
        directory.join("xenomorph.toml"),
        format!(
            "[parser]\nentry = \"index\"\n[plugins]\npath = {}\nplugins = {}\n",
            serde_json::to_string(libraries.to_str().unwrap()).unwrap(),
            serde_json::to_string(plugins).unwrap(),
        ),
    )
    .unwrap();
}

#[test]
fn formatting_disconnected_documents_uses_the_current_complete_buffer() {
    use tower_lsp::lsp_types::Url;

    let directory = std::env::temp_dir().join(format!(
        "xenomorph-lsp-formatting-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("index.xen"),
        "pub import other::Other;\ntype Root = string;",
    )
    .unwrap();
    fs::write(directory.join("other.xen"), "type Saved = string;").unwrap();
    config(&directory, &[]);
    let entry = Url::from_file_path(directory.join("index.xen")).unwrap();
    let other = Url::from_file_path(directory.join("other.xen")).unwrap();
    let mut server = Server::start(directory);
    server.request(1, "initialize", json!({ "capabilities": {} }));
    server.notification("initialized", json!({}));
    server.wait_for_log("Language Server initialized");
    server.open(&entry, "pub import other::Other;\ntype Root = string;");
    server.diagnostics(&entry);
    server.diagnostics(&other);
    let source = "// \u{1f600} draft\r\ntype Draft=string;\r\ntype Tail=number;";
    server.open(&other, source);
    server.diagnostics(&entry);
    server.diagnostics(&other);
    let connected = server.request(
        10,
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": other }, "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    assert_eq!(
        connected["result"][0]["range"],
        json!({
            "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 17 }
        })
    );
    server.change(
        &entry,
        2,
        "// pub import other::Other;\ntype Root = string;",
    );
    server.diagnostics(&entry);
    server.diagnostics(&other);
    let edited = source.replace("Draft", "Edited");
    server.change(&other, 2, &edited);
    server.diagnostics(&entry);
    server.diagnostics(&other);
    server.notification("textDocument/didChange", json!({
        "textDocument": { "uri": other, "version": 3 },
        "contentChanges": [
            { "range": { "start": { "line": 1, "character": 5 }, "end": { "line": 1, "character": 11 } }, "text": "Latest" },
            { "range": { "start": { "line": 2, "character": 10 }, "end": { "line": 2, "character": 16 } }, "text": "string" }
        ]
    }));
    server.diagnostics(&entry);
    server.diagnostics(&other);
    let edited = edited
        .replace("Edited", "Latest")
        .replace("Tail=number", "Tail=string");
    let formatted = server.request(
        2,
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": other }, "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    let edits = formatted["result"]
        .as_array()
        .expect("format disconnected document");
    assert_eq!(edits.len(), 1, "{formatted}");
    assert_eq!(
        edits[0]["range"],
        json!({
            "start": { "line": 0, "character": 0 }, "end": { "line": 2, "character": 17 }
        })
    );
    let expected = xenomorph_common::formatter::format_xenomorph(
        &edited,
        &xenomorph_common::config::FormatterConfig::default(),
    );
    assert_eq!(edits[0]["newText"], expected);
    assert_eq!(expected.matches("type Latest").count(), 1);
    assert_eq!(expected.matches("type Tail").count(), 1);
    let symbols = server.request(
        3,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert!(
        symbols["result"].is_null(),
        "Standalone formatting added a module: {symbols}"
    );
    server.change(&entry, 3, "pub import other::Other;\ntype Root = string;");
    server.diagnostics(&entry);
    server.diagnostics(&other);
    let symbols = server.request(
        4,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert_eq!(symbols["result"][0]["name"], "Latest", "{symbols}");
    assert_eq!(symbols["result"][1]["name"], "Tail", "{symbols}");
    let formatted = server.request(
        5,
        "textDocument/formatting",
        json!({
            "textDocument": { "uri": other }, "options": { "tabSize": 4, "insertSpaces": true }
        }),
    );
    assert_eq!(formatted["result"][0]["newText"], expected);
}

#[test]
fn range_formatting_preserves_surrounding_text_in_connected_and_standalone_buffers() {
    use tower_lsp::lsp_types::{TextEdit, Url};
    use xenomorph_common::SourceDocument;

    let directory = std::env::temp_dir().join(format!(
        "xenomorph-lsp-range-formatting-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("index.xen"), "type Saved = string;").unwrap();
    config(&directory, &[]);
    let uris = [
        Url::from_file_path(directory.join("index.xen")).unwrap(),
        Url::from_file_path(directory.join("orphan.xen")).unwrap(),
        Url::parse("untitled:selection.xen").unwrap(),
    ];
    let mut server = Server::start(directory);
    let initialized = server.request(1, "initialize", json!({ "capabilities": {} }));
    assert_eq!(
        initialized["result"]["capabilities"]["documentRangeFormattingProvider"],
        true
    );
    server.notification("initialized", json!({}));
    server.wait_for_log("Language Server initialized");

    let source = "// \u{1f600} draft\r\n   type Before = | string | number;\r\ntype Selected = {\r\nnested: {\r\nvalue: string,\r\n},\r\n};\r\n   type Choice = | string | number;\r\n   type Tail = \"\u{1f600}\";";
    let ranges = [
        (
            json!({ "start": { "line": 4, "character": 3 }, "end": { "line": 5, "character": 0 } }),
            json!({ "start": { "line": 4, "character": 0 }, "end": { "line": 5, "character": 0 } }),
            "value: string,\r\n",
            "        value: string,\n",
        ),
        (
            json!({ "start": { "line": 7, "character": 3 }, "end": { "line": 7, "character": 10 } }),
            json!({ "start": { "line": 7, "character": 0 }, "end": { "line": 8, "character": 0 } }),
            "   type Choice = | string | number;\r\n",
            "type Choice =\n    | string\n    | number;\n",
        ),
        (
            json!({ "start": { "line": 8, "character": 3 }, "end": { "line": 8, "character": 10 } }),
            json!({ "start": { "line": 8, "character": 0 }, "end": { "line": 8, "character": 20 } }),
            "   type Tail = \"\u{1f600}\";",
            "type Tail = \"\u{1f600}\";",
        ),
    ];
    for uri in &uris {
        server.open(uri, "type Initial = string;");
        server.diagnostics(uri);
        server.change(uri, 2, source);
        server.diagnostics(uri);
        for (range, edit_range, original, replacement) in &ranges {
            let response = server.request(
                2,
                "textDocument/rangeFormatting",
                json!({
                    "textDocument": { "uri": uri },
                    "range": range,
                    "options": { "tabSize": 2, "insertSpaces": false }
                }),
            );
            assert_eq!(
                response["result"].as_array().unwrap().len(),
                1,
                "{response}"
            );
            assert_eq!(&response["result"][0]["range"], edit_range, "{response}");
            let edit: TextEdit = serde_json::from_value(response["result"][0].clone()).unwrap();
            assert_eq!(&edit.new_text, replacement, "{response}");
            let document = SourceDocument::new(source.into());
            let start = document
                .offset(edit.range.start.line, edit.range.start.character, true)
                .unwrap() as usize;
            let end = document
                .offset(edit.range.end.line, edit.range.end.character, true)
                .unwrap() as usize;
            let mut applied = source.to_string();
            applied.replace_range(start..end, &edit.new_text);
            assert_eq!(applied, source.replacen(original, replacement, 1));
        }

        let empty = server.request(3, "textDocument/rangeFormatting", json!({
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
            "options": { "tabSize": 4, "insertSpaces": true }
        }));
        assert_eq!(empty["result"], json!([]));
        let invalid = server.request(4, "textDocument/rangeFormatting", json!({
            "textDocument": { "uri": uri },
            "range": { "start": { "line": 0, "character": 4 }, "end": { "line": 0, "character": 5 } },
            "options": { "tabSize": 4, "insertSpaces": true }
        }));
        assert_eq!(invalid["error"]["code"], -32602, "{invalid}");

        // A formatting request must leave the live buffer unchanged.
        let full = server.request(
            5,
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": uri }, "options": { "tabSize": 4, "insertSpaces": true }
            }),
        );
        assert_eq!(
            full["result"][0]["newText"],
            xenomorph_common::formatter::format_xenomorph(
                source,
                &xenomorph_common::config::FormatterConfig::default()
            )
        );
    }

    // Disconnected formatting must not add the standalone buffer to the graph.
    let symbols = server.request(
        6,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uris[1] } }),
    );
    assert!(symbols["result"].is_null(), "{symbols}");
    let unopened = Url::from_file_path(server.directory.join("unopened.xen")).unwrap();
    let response = server.request(7, "textDocument/rangeFormatting", json!({
        "textDocument": { "uri": unopened },
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
        "options": { "tabSize": 4, "insertSpaces": true }
    }));
    assert!(response["result"].is_null(), "{response}");
}

#[test]
fn rapid_import_toggles_leave_the_latest_diagnostic_visible() {
    use tower_lsp::lsp_types::Url;

    let directory = std::env::temp_dir().join(format!(
        "xenomorph-lsp-import-toggles-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("index.xen"),
        "pub import other::Other;\ntype Root = string;",
    )
    .unwrap();
    fs::write(directory.join("other.xen"), "pub type Other = string;").unwrap();
    config(&directory, &[]);
    let entry = Url::from_file_path(directory.join("index.xen")).unwrap();
    let other = Url::from_file_path(directory.join("other.xen")).unwrap();
    let mut server = Server::start(directory);
    server.request(1, "initialize", json!({ "capabilities": {} }));
    server.notification("initialized", json!({}));
    server.wait_for_log("Language Server initialized");
    server.open(&entry, "pub import other::Other;\ntype Root = string;");
    server.diagnostics(&entry);
    server.diagnostics(&other);
    server.open(&other, "pub type Other = string;");
    server.diagnostics(&entry);
    server.diagnostics(&other);

    for batch in 0..10 {
        for edit in 0..50 {
            let text = if (edit + batch) % 2 == 0 {
                "pub import other::Other;\ntype Root = string;"
            } else {
                "// pub import other::Other;\ntype Root = string;"
            };
            server.change(&entry, 2 + batch * 50 + edit, text);
        }
        let final_version = 2 + batch * 50 + 49;
        server.wait_for_message(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == entry.as_str()
                && message["params"]["version"] == final_version
        });
        // Older notifications can be coalesced. Wait for the final version,
        // then inspect the last publication for the imported document.
        while let Ok(message) = server.messages.recv_timeout(Duration::from_millis(50)) {
            server.pending_messages.push(message);
        }
        let latest = server
            .pending_messages
            .iter()
            .rev()
            .find(|message| {
                message["method"] == "textDocument/publishDiagnostics"
                    && message["params"]["uri"] == other.as_str()
            })
            .expect("diagnostics for imported document")["params"]["diagnostics"]
            .clone();
        server.pending_messages.clear();
        if batch % 2 == 0 {
            assert_eq!(
                latest[0]["message"], "Not part of the current module graph.",
                "Stale diagnostic after rapid toggles in batch {batch}: {latest}"
            );
        } else {
            assert_eq!(
                latest,
                json!([]),
                "Stale diagnostic after rapid toggles in batch {batch}"
            );
        }
    }
}

#[test]
fn files_outside_the_module_graph_are_dimmed_and_update_with_imports_and_reload() {
    use tower_lsp::lsp_types::Url;

    let directory = std::env::temp_dir().join(format!(
        "xenomorph-lsp-module-graph-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::create_dir(directory.join("modules")).unwrap();
    fs::write(directory.join("modules/index.xen"), "type Root = string;").unwrap();
    fs::write(directory.join("modules/other.xen"), "type Saved = string;").unwrap();
    fs::write(directory.join("outside.xen"), "type Outside = string;").unwrap();
    fs::write(
        directory.join("xenomorph.toml"),
        "[parser]\nentry = \"modules/index\"\n",
    )
    .unwrap();
    let entry = Url::from_file_path(directory.join("modules/index.xen")).unwrap();
    let other = Url::from_file_path(directory.join("modules/other.xen")).unwrap();
    let outside = Url::from_file_path(directory.join("outside.xen")).unwrap();
    let untitled = Url::parse("untitled:Untitled-1").unwrap();
    let mut server = Server::start(directory);
    let initialized = server.request(1, "initialize", json!({ "capabilities": {} }));
    assert!(initialized.get("error").is_none(), "{initialized}");
    server.notification("initialized", json!({}));
    server.wait_for_log("Language Server initialized");

    let source = "// draft\r\ntype Draft = Missing; // \u{1f600}";
    server.open(&other, source);
    let expected = json!([{
        "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 1, "character": 27 } },
        "severity": 3,
        "message": "Not part of the current module graph.",
        "source": "xenomorph",
        "tags": [1]
    }]);
    assert_eq!(server.diagnostics(&other), expected);
    // Unrelated files must not be indexed simply by opening them.
    let symbols = server.request(
        2,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert!(symbols["result"].is_null(), "{symbols}");

    server.change(&other, 2, "");
    let mut empty = expected.clone();
    empty[0]["range"]["end"] = json!({ "line": 0, "character": 0 });
    assert_eq!(server.diagnostics(&other), empty);
    server.change(&other, 3, source);
    assert_eq!(server.diagnostics(&other), expected);

    // Connecting an already open file must validate its unsaved text.
    server.open(&entry, "import other::Other; type Root = string;");
    let diagnostics = server.diagnostics(&other);
    assert!(
        diagnostics.as_array().unwrap().iter().any(|diagnostic| {
            diagnostic["severity"] == 1
                && diagnostic["message"].as_str().unwrap().contains("Missing")
        }),
        "{diagnostics}"
    );
    assert!(diagnostics
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == 1)
        .all(|diagnostic| diagnostic["tags"].is_null()));
    let symbols = server.request(
        3,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert_eq!(symbols["result"][0]["name"], "Draft", "{symbols}");
    server.diagnostics(&entry);

    server.change(&entry, 2, "type Root = string;");
    assert_eq!(server.diagnostics(&other), expected);
    server.diagnostics(&entry);
    let symbols = server.request(
        20,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert!(
        symbols["result"].is_null(),
        "Disconnected module still has symbols: {symbols}"
    );
    server.open(&outside, "type Outside = string;");
    let diagnostics = server.diagnostics(&outside);
    assert_eq!(diagnostics[0]["message"], expected[0]["message"]);
    assert_eq!(diagnostics[0]["tags"], json!([1]));
    server.diagnostics(&other);
    server.diagnostics(&entry);
    server.open(&untitled, "");
    assert_eq!(server.diagnostics(&untitled), empty);
    server.diagnostics(&outside);
    server.diagnostics(&other);
    server.diagnostics(&entry);

    let reload = server.request(
        4,
        "workspace/executeCommand",
        json!({ "command": "xenomorph.reloadPlugins" }),
    );
    assert_eq!(reload["result"]["reloaded"], true, "{reload}");
    assert_eq!(server.diagnostics(&other), expected);
    assert_eq!(server.diagnostics(&untitled), empty);
    server.diagnostics(&outside);
    server.diagnostics(&entry);
    // Reload must not promote an unrelated open buffer into the graph.
    let symbols = server.request(
        5,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": other } }),
    );
    assert!(symbols["result"].is_null(), "{symbols}");

    server.change(&entry, 3, "import other::Other; type Root = string;");
    let diagnostics = server.diagnostics(&other);
    assert!(diagnostics
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["severity"] == 1));
    server.diagnostics(&outside);
    server.diagnostics(&untitled);
    server.diagnostics(&entry);
    let reload = server.request(
        6,
        "workspace/executeCommand",
        json!({ "command": "xenomorph.reloadPlugins" }),
    );
    assert_eq!(reload["result"]["reloaded"], true, "{reload}");
    let diagnostics = server.diagnostics(&other);
    assert!(diagnostics
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["severity"] == 1));

    server.notification(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": other } }),
    );
    assert_eq!(server.diagnostics(&other), json!([]));
}

#[test]
#[ignore = "Build workspace cdylibs first: cargo build --workspace"]
fn reload_preserves_unsaved_documents_metadata_and_last_valid_runtime() {
    let directory = std::env::temp_dir().join(format!(
        "xenomorph-lsp-reload-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("index.xen"), "type Saved = string;").unwrap();
    config(&directory, &["xenomorph_typescript"]);
    let uri = tower_lsp::lsp_types::Url::from_file_path(directory.join("index.xen")).unwrap();
    let mut server = Server::start(directory);
    let initialized = server.request(
        1,
        "initialize",
        json!({
            "processId": null, "rootUri": null, "capabilities": {}
        }),
    );
    assert!(initialized.get("error").is_none(), "{initialized}");
    assert_eq!(
        initialized["result"]["capabilities"]["executeCommandProvider"]["commands"][0],
        "xenomorph.reloadPlugins"
    );
    server.notification("initialized", json!({}));
    server.wait_for_log("Language Server initialized");
    server.notification("textDocument/didOpen", json!({
        "textDocument": { "uri": uri, "languageId": "xenomorph", "version": 1, "text": "type Draft = TS::symbol;" }
    }));
    let symbols = server.request(
        2,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols["result"][0]["name"], "Draft", "{symbols}");

    config(&server.directory, &[]);
    let reload = server.request(
        3,
        "workspace/executeCommand",
        json!({ "command": "xenomorph.reloadPlugins" }),
    );
    assert_eq!(reload["result"]["reloaded"], true, "{reload}");
    let symbols = server.request(
        4,
        "textDocument/documentSymbol",
        json!({ "textDocument": { "uri": uri } }),
    );
    assert_eq!(symbols["result"][0]["name"], "Draft", "{symbols}");
    let hover_params =
        json!({ "textDocument": { "uri": uri }, "position": { "line": 0, "character": 14 } });
    let hover = server.request(5, "textDocument/hover", hover_params.clone());
    assert!(hover["result"].is_null(), "{hover}");

    config(&server.directory, &["xenomorph_typescript"]);
    server.wait_for_log("reloaded without restarting");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let hover = server.request(6, "textDocument/hover", hover_params.clone());
        if !hover["result"].is_null() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Config-driven reload did not restore symbol metadata"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    fs::write(server.directory.join("xenomorph.toml"), "not valid TOML [").unwrap();
    let reload = server.request(
        7,
        "workspace/executeCommand",
        json!({ "command": "xenomorph.reloadPlugins" }),
    );
    assert!(reload.get("error").is_some(), "{reload}");
    let hover = server.request(8, "textDocument/hover", hover_params);
    assert!(!hover["result"].is_null(), "{hover}");
    let shutdown = server.request(9, "shutdown", Value::Null);
    assert!(shutdown.get("error").is_none(), "{shutdown}");
    server.notification("exit", Value::Null);
    server.input.take();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "LSP did not shut down after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
