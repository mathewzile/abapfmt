//! Where the server finds the configuration: next to local files, and in the workspace folders for
//! documents of remote file systems.
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start(init_params: Value) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_abapfmt_lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut c = Client {
            child,
            stdin,
            stdout,
        };
        c.request(1, "initialize", init_params);
        c.notify("initialized", json!({}));
        c
    }

    fn send(&mut self, message: &Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut length = 0;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length: ") {
                length = v.parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Opens a document and returns the number of edits that formatting it yields.
    fn edit_count(&mut self, id: i64, uri: &str, text: &str) -> usize {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "abap", "version": 1, "text": text}}),
        );
        let r = self.request(
            id,
            "textDocument/formatting",
            json!({"textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": true}}),
        );
        r["result"].as_array().unwrap().len()
    }

    fn finish(mut self) {
        self.request(99, "shutdown", Value::Null);
        self.notify("exit", Value::Null);
        assert!(self.child.wait().unwrap().success());
    }
}

const UNFORMATTED: &str = "DATA   lv_a   TYPE i.\n";
/// Switches off the rule that would format `UNFORMATTED`.
const RULE_OFF: &str = r#"{"rules":{"ALIGN_DECLARATIONS":false}}"#;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("abapfmt-lsp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_uri(p: &Path) -> String {
    format!("file:///{}", p.to_string_lossy().replace('\\', "/"))
}

#[test]
fn local_files_use_the_config_of_their_checkout() {
    let root = temp_dir("local");
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::create_dir_all(root.join("src/zpackage")).unwrap();
    std::fs::write(root.join(".vscode/abapfmt.json"), RULE_OFF).unwrap();
    let mut c = Client::start(json!({"capabilities": {}, "rootUri": null}));
    let uri = file_uri(&root.join("src/zpackage/zcl_a.abap"));
    assert_eq!(c.edit_count(2, &uri, UNFORMATTED), 0);
    // a file outside of the checkout uses the defaults
    let other = file_uri(&temp_dir("elsewhere").join("zcl_b.abap"));
    assert_eq!(c.edit_count(3, &other, UNFORMATTED), 1);
    c.finish();
}

#[test]
fn remote_documents_use_the_workspace_config() {
    let root = temp_dir("remote");
    std::fs::create_dir_all(root.join(".github")).unwrap();
    std::fs::write(root.join(".github/abapfmt.json"), RULE_OFF).unwrap();
    let mut c = Client::start(json!({
        "capabilities": {},
        "workspaceFolders": [
            {"uri": "adt://dev_system/", "name": "DEV"},
            {"uri": file_uri(&root), "name": "workspace"}
        ]
    }));
    // documents of different systems share the workspace configuration
    assert_eq!(
        c.edit_count(2, "adt://dev_system/sap/bc/adt/zcl_a.abap", UNFORMATTED),
        0
    );
    assert_eq!(
        c.edit_count(3, "adt://qa_system/sap/bc/adt/zcl_b.abap", UNFORMATTED),
        0
    );
    c.finish();
}

#[test]
fn explicit_config_path_wins() {
    let root = temp_dir("explicit");
    std::fs::write(root.join("team.json"), RULE_OFF).unwrap();
    let mut c = Client::start(json!({
        "capabilities": {},
        "workspaceFolders": [{"uri": file_uri(&root), "name": "workspace"}],
        "initializationOptions": {"configPath": "team.json"}
    }));
    assert_eq!(
        c.edit_count(2, "adt://dev_system/zcl_a.abap", UNFORMATTED),
        0
    );
    c.finish();
}
