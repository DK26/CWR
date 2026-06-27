// Minimal VS Code client: launches poseidon-lsp over stdio and binds it to the
// three Poseidon languages. The server is the editor-independent part; this
// file only exists so VS Code can spawn it. Neovim/Helix/Emacs point at the
// `poseidon-lsp` binary directly (see lsp/README.md).
const { workspace } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const command = workspace.getConfiguration("poseidon").get("server.path") || "poseidon-lsp";
  const serverOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };
  const clientOptions = {
    documentSelector: [
      { scheme: "file", language: "sqf" },
      { scheme: "file", language: "sqs" },
      { scheme: "file", language: "paramfile" },
    ],
  };
  client = new LanguageClient("poseidon", "Poseidon Language Server", serverOptions, clientOptions);
  client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
