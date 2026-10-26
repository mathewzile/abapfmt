use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_abapfmt_lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Client {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: &Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
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

    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self.receive();
            if message.get("id").is_some() {
                return message;
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }
}

#[test]
fn formats_documents_and_ranges() {
    let mut c = Client::start();
    let init = c.request(1, "initialize", json!({"capabilities": {}}));
    assert_eq!(
        init["result"]["capabilities"]["documentFormattingProvider"],
        true
    );
    assert_eq!(
        init["result"]["capabilities"]["documentRangeFormattingProvider"],
        true
    );
    c.notify("initialized", json!({}));

    let uri = "file:///nonexistent-dir/a.abap";
    let text = "DATA   lv_a   TYPE i.\nDATA   lv_b   TYPE i.\n";
    c.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": uri, "languageId": "abap", "version": 1, "text": text}}),
    );

    let all = c.request(
        2,
        "textDocument/formatting",
        json!({
        "textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": true}}),
    );
    let edits = all["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(
        edits[0]["newText"],
        "DATA lv_a TYPE i.\nDATA lv_b TYPE i.\n"
    );

    let second_line = c.request(
        3,
        "textDocument/rangeFormatting",
        json!({
        "textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": true},
        "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 5}}}),
    );
    let edits = second_line["result"].as_array().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0]["range"]["start"]["line"], 1);
    assert_eq!(edits[0]["newText"], "DATA lv_b TYPE i.\n");

    // after a change, the new text is formatted
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri, "version": 2},
               "contentChanges": [{"text": "DATA lv_a TYPE i.\n"}]}),
    );
    let none = c.request(
        4,
        "textDocument/formatting",
        json!({
        "textDocument": {"uri": uri}, "options": {"tabSize": 2, "insertSpaces": true}}),
    );
    assert_eq!(none["result"].as_array().unwrap().len(), 0);

    let unknown = c.request(5, "textDocument/formatting", json!({
        "textDocument": {"uri": "file:///other.abap"}, "options": {"tabSize": 2, "insertSpaces": true}}));
    assert!(unknown["error"].is_object());

    c.request(6, "shutdown", Value::Null);
    c.notify("exit", Value::Null);
    assert!(c.child.wait().unwrap().success());
}
