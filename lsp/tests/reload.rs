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
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
        }
    }

    fn notification(&mut self, method: &str, params: Value) {
        let mut message = json!({ "jsonrpc": "2.0", "method": method });
        if !params.is_null() {
            message["params"] = params;
        }
        self.send(message);
    }

    fn wait_for_log(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if message["method"] == "window/logMessage"
                && message["params"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains(text))
            {
                return;
            }
        }
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
        "textDocument": { "uri": uri, "languageId": "xenomorph", "version": 1, "text": "type Draft = symbol;" }
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
