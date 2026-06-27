//! End-to-end protocol test: spawn the real `poseidon-lsp` binary and drive it
//! over stdio like an editor would, asserting the engine-grounded behaviours
//! (diagnostics, completion, hover) survive the full LSP round-trip.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

struct Server {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_poseidon-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn poseidon-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                // read headers until a blank line
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let t = line.trim_end();
                    if t.is_empty() {
                        break;
                    }
                    if let Some(n) = t.strip_prefix("Content-Length:") {
                        len = n.trim().parse().unwrap_or(0);
                    }
                }
                let mut buf = vec![0u8; len];
                if r.read_exact(&mut buf).is_err() {
                    return;
                }
                if let Ok(v) = serde_json::from_slice::<Value>(&buf) {
                    if tx.send(v).is_err() {
                        return;
                    }
                }
            }
        });
        Server { child, stdin, rx }
    }

    fn send(&mut self, v: Value) {
        let body = serde_json::to_vec(&v).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        self.stdin.write_all(&body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn wait<F: Fn(&Value) -> bool>(&self, pred: F) -> Value {
        let deadline = Duration::from_secs(15);
        loop {
            let v = self
                .rx
                .recv_timeout(deadline)
                .expect("timed out waiting for an LSP message");
            if pred(&v) {
                return v;
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn full_roundtrip_diagnostics_completion_hover() {
    let mut s = Server::start();

    s.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    let init = s.wait(|m| m["id"] == 1);
    let caps = &init["result"]["capabilities"];
    assert!(caps["completionProvider"].is_object());
    assert!(caps["hoverProvider"].as_bool().unwrap_or(false));
    assert!(caps["semanticTokensProvider"].is_object());
    s.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let uri = "file:///c/tmp/proto.sqf";
    let text = "_u serDamage 1;\nhint \"oops";
    s.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{
        "textDocument":{"uri":uri,"languageId":"sqf","version":1,"text":text}}}));
    let diag = s.wait(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri);
    let diags = diag["params"]["diagnostics"].as_array().unwrap();
    // unknown command `serDamage` + unterminated string
    assert!(diags.len() >= 2, "expected >=2 diagnostics, got {diags:?}");
    assert!(diags.iter().any(|d| d["message"].as_str().unwrap_or("").contains("serDamage")));

    // completion returns the engine command set
    s.send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{
        "textDocument":{"uri":uri},"position":{"line":0,"character":0}}}));
    let comp = s.wait(|m| m["id"] == 2);
    let items = comp["result"]["items"].as_array().or(comp["result"].as_array()).unwrap();
    assert!(items.len() > 400, "expected the full command set, got {}", items.len());

    // hover over `hint` (line 1, a registered command) renders its signature
    s.send(json!({"jsonrpc":"2.0","id":3,"method":"textDocument/hover","params":{
        "textDocument":{"uri":uri},"position":{"line":1,"character":1}}}));
    let hov = s.wait(|m| m["id"] == 3);
    let val = hov["result"]["contents"]["value"].as_str().unwrap_or("");
    assert!(val.contains("hint"), "hover over `hint` should show its signature, got {val:?}");

    s.send(json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":{}}));
    s.wait(|m| m["id"] == 99);
    s.send(json!({"jsonrpc":"2.0","method":"exit","params":{}}));
}
