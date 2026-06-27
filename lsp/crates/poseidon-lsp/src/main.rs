//! Poseidon language server — SQF/SQS scripting and ParamFile config
//! (`.sqm` / `.ext` / `.cfg` / `config.cpp`) for any LSP-capable editor.

mod line_index;

use dashmap::DashMap;
use line_index::LineIndex;
use once_cell::sync::Lazy;
use poseidon_catalog as cat;
use poseidon_syntax::common::{Severity, Tok};
use poseidon_syntax::{config, sqf};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Sqf,
    Sqs,
    Config,
}

fn lang_of(uri: &Url) -> Option<Lang> {
    let path = uri.path().to_ascii_lowercase();
    if path.ends_with(".sqf") {
        Some(Lang::Sqf)
    } else if path.ends_with(".sqs") {
        Some(Lang::Sqs)
    } else if path.ends_with(".sqm")
        || path.ends_with(".ext")
        || path.ends_with(".cfg")
        || path.ends_with("config.cpp")
    {
        Some(Lang::Config)
    } else {
        None
    }
}

struct Document {
    text: String,
    lang: Lang,
}

struct Backend {
    client: Client,
    docs: DashMap<Url, Document>,
}

// ---- semantic token legend -------------------------------------------------

const LEGEND_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,      // 0
    SemanticTokenType::FUNCTION,     // 1
    SemanticTokenType::VARIABLE,     // 2
    SemanticTokenType::PROPERTY,     // 3
    SemanticTokenType::STRING,       // 4
    SemanticTokenType::NUMBER,       // 5
    SemanticTokenType::COMMENT,      // 6
    SemanticTokenType::OPERATOR,     // 7
    SemanticTokenType::MACRO,        // 8
    SemanticTokenType::TYPE,         // 9
    SemanticTokenType::new("label"), // 10
];

fn tok_index(t: Tok) -> Option<u32> {
    Some(match t {
        Tok::Keyword | Tok::StructKeyword => 0,
        Tok::Command => 1,
        Tok::LocalVar | Tok::GlobalVar => 2,
        Tok::Property => 3,
        Tok::Str => 4,
        Tok::Number => 5,
        Tok::Comment => 6,
        Tok::Operator => 7,
        Tok::Macro => 8,
        Tok::TypeName => 9,
        Tok::Label => 10,
        Tok::Punct => return None,
    })
}

// ---- lexing helpers --------------------------------------------------------

fn lex(text: &str, lang: Lang) -> (Vec<poseidon_syntax::Token>, Vec<poseidon_syntax::Diagnostic>) {
    match lang {
        Lang::Sqf => sqf::lex(text, sqf::Dialect::Sqf),
        Lang::Sqs => sqf::lex(text, sqf::Dialect::Sqs),
        Lang::Config => config::lex(text),
    }
}

fn semantic_tokens(text: &str, toks: &[poseidon_syntax::Token]) -> Vec<SemanticToken> {
    let li = LineIndex::new(text);
    // (line, start_char_utf16, len_utf16, token_type) — single-line pieces
    let mut pieces: Vec<(u32, u32, u32, u32)> = Vec::new();
    for t in toks {
        let Some(ty) = tok_index(t.tok) else { continue };
        let seg = &text[t.span.start..t.span.end.min(text.len())];
        let mut seg_start = t.span.start;
        for line in seg.split('\n') {
            let visible = line.trim_end_matches('\r');
            let len16 = visible.encode_utf16().count() as u32;
            if len16 > 0 {
                let p = li.position(text, seg_start);
                pieces.push((p.line, p.character, len16, ty));
            }
            seg_start += line.len() + 1; // skip the '\n'
        }
    }
    pieces.sort_by_key(|p| (p.0, p.1));
    let mut data = Vec::with_capacity(pieces.len());
    let (mut pl, mut pc) = (0u32, 0u32);
    for (line, ch, len, ty) in pieces {
        let dl = line - pl;
        let dc = if dl == 0 { ch - pc } else { ch };
        data.push(SemanticToken {
            delta_line: dl,
            delta_start: dc,
            length: len,
            token_type: ty,
            token_modifiers_bitset: 0,
        });
        pl = line;
        pc = ch;
    }
    data
}

fn to_diagnostics(text: &str, diags: &[poseidon_syntax::Diagnostic]) -> Vec<Diagnostic> {
    let li = LineIndex::new(text);
    diags
        .iter()
        .map(|d| Diagnostic {
            range: Range {
                start: li.position(text, d.span.start),
                end: li.position(text, d.span.end),
            },
            severity: Some(match d.severity {
                Severity::Error => DiagnosticSeverity::ERROR,
                Severity::Warning => DiagnosticSeverity::WARNING,
                Severity::Information => DiagnosticSeverity::INFORMATION,
                Severity::Hint => DiagnosticSeverity::HINT,
            }),
            code: Some(NumberOrString::String(d.code.to_string())),
            source: Some("poseidon".into()),
            message: d.message.clone(),
            ..Default::default()
        })
        .collect()
}

// ---- completion ------------------------------------------------------------

static SQF_COMPLETIONS: Lazy<Vec<CompletionItem>> = Lazy::new(|| {
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for c in cat::COMMANDS.iter().filter(|c| c.is_identifier()) {
        if !seen.insert(c.name.to_ascii_lowercase()) {
            continue;
        }
        let kind = if cat::is_keyword(&c.name) {
            CompletionItemKind::KEYWORD
        } else {
            CompletionItemKind::FUNCTION
        };
        items.push(CompletionItem {
            label: c.name.clone(),
            kind: Some(kind),
            detail: Some(c.signature()),
            documentation: Some(Documentation::String(c.detail())),
            ..Default::default()
        });
    }
    items
});

static CONFIG_COMPLETIONS: Lazy<Vec<CompletionItem>> = Lazy::new(|| {
    ["class", "enum"]
        .iter()
        .map(|k| CompletionItem {
            label: k.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        })
        .collect()
});

// ---- hover -----------------------------------------------------------------

fn hover_markdown(name: &str) -> Option<String> {
    let overloads = cat::lookup(name);
    if overloads.is_empty() {
        return None;
    }
    let mut sigs: Vec<String> = overloads.iter().map(|c| c.signature()).collect();
    sigs.dedup();
    let mut seen = std::collections::HashSet::new();
    sigs.retain(|s| seen.insert(s.clone()));
    let mut s = String::new();
    s.push_str("```sqf\n");
    for sig in &sigs {
        s.push_str(sig);
        s.push('\n');
    }
    s.push_str("```\n");
    let origin = overloads[0].origin.as_str();
    s.push_str(&format!(
        "\n*classic Operation Flashpoint / Cold War Assault {} command*",
        if origin.is_empty() { "engine" } else { origin }
    ));
    if sigs.len() > 1 {
        s.push_str(&format!(" · {} overloads", sigs.len()));
    }
    // Authored docs (description + runnable examples), when the corpus covers it.
    // Signatures above always come from the catalog; the corpus only adds prose.
    if let Some(doc) = cat::docs(name) {
        s.push_str("\n\n");
        s.push_str(&doc.description);
        if !doc.examples.is_empty() {
            s.push_str("\n\n```sqf\n");
            for ex in &doc.examples {
                s.push_str(ex);
                s.push('\n');
            }
            s.push_str("```");
        }
    }
    Some(s)
}

fn config_symbols(text: &str) -> Vec<DocumentSymbol> {
    let li = LineIndex::new(text);
    fn build(li: &LineIndex, text: &str, c: &config::ClassSymbol) -> DocumentSymbol {
        let range = Range {
            start: li.position(text, c.full_span.start),
            end: li.position(text, c.full_span.end),
        };
        let selection_range = Range {
            start: li.position(text, c.name_span.start),
            end: li.position(text, c.name_span.end),
        };
        #[allow(deprecated)]
        DocumentSymbol {
            name: c.name.clone(),
            detail: c.base.as_ref().map(|b| format!(": {b}")),
            kind: SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: Some(c.children.iter().map(|ch| build(li, text, ch)).collect()),
        }
    }
    config::symbols(text)
        .iter()
        .map(|c| build(&li, text, c))
        .collect()
}

impl Backend {
    async fn refresh(&self, uri: Url) {
        let Some(doc) = self.docs.get(&uri) else { return };
        let mut diags = lex(&doc.text, doc.lang).1; // lexical
        match doc.lang {
            Lang::Sqf => diags.extend(sqf::semantic_diagnostics(&doc.text, sqf::Dialect::Sqf)),
            Lang::Sqs => diags.extend(sqf::semantic_diagnostics(&doc.text, sqf::Dialect::Sqs)),
            Lang::Config => {}
        }
        let lsp_diags = to_diagnostics(&doc.text, &diags);
        self.client.publish_diagnostics(uri, lsp_diags, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "poseidon-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["_".into()]),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: SemanticTokensLegend {
                                token_types: LEGEND_TYPES.to_vec(),
                                token_modifiers: vec![],
                            },
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "poseidon-lsp ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let td = params.text_document;
        let Some(lang) = lang_of(&td.uri) else { return };
        self.docs.insert(td.uri.clone(), Document { text: td.text, lang });
        self.refresh(td.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let Some(lang) = lang_of(&uri) else { return };
        if let Some(change) = params.content_changes.into_iter().last() {
            self.docs.insert(uri.clone(), Document { text: change.text, lang });
            self.refresh(uri).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.docs.remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let items = match doc.lang {
            Lang::Sqf | Lang::Sqs => SQF_COMPLETIONS.clone(),
            Lang::Config => CONFIG_COMPLETIONS.clone(),
        };
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        if doc.lang == Lang::Config {
            return Ok(None);
        }
        let li = LineIndex::new(&doc.text);
        let offset = li.offset(&doc.text, pos);
        let (toks, _) = lex(&doc.text, doc.lang);
        let hit = toks.iter().find(|t| {
            offset >= t.span.start
                && offset < t.span.end
                && matches!(t.tok, Tok::Command | Tok::Keyword)
        });
        let Some(t) = hit else { return Ok(None) };
        let name = &doc.text[t.span.start..t.span.end];
        let Some(md) = hover_markdown(name) else { return Ok(None) };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
            range: Some(Range {
                start: li.position(&doc.text, t.span.start),
                end: li.position(&doc.text, t.span.end),
            }),
        }))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        let (toks, _) = lex(&doc.text, doc.lang);
        let data = semantic_tokens(&doc.text, &toks);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(doc) = self.docs.get(&uri) else { return Ok(None) };
        if doc.lang != Lang::Config {
            return Ok(None);
        }
        Ok(Some(DocumentSymbolResponse::Nested(config_symbols(&doc.text))))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: DashMap::new(),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(p: &str) -> Url {
        Url::parse(p).unwrap()
    }

    #[test]
    fn language_detection_by_extension() {
        assert!(matches!(lang_of(&uri("file:///m/init.sqf")), Some(Lang::Sqf)));
        assert!(matches!(lang_of(&uri("file:///m/init.sqs")), Some(Lang::Sqs)));
        assert!(matches!(lang_of(&uri("file:///m/mission.sqm")), Some(Lang::Config)));
        assert!(matches!(lang_of(&uri("file:///m/description.ext")), Some(Lang::Config)));
        assert!(matches!(lang_of(&uri("file:///m/user.cfg")), Some(Lang::Config)));
        assert!(matches!(lang_of(&uri("file:///m/config.cpp")), Some(Lang::Config)));
        // not a Poseidon resource, and a generic .cpp must NOT be treated as config
        assert!(lang_of(&uri("file:///m/notes.txt")).is_none());
        assert!(lang_of(&uri("file:///m/engine.cpp")).is_none());
    }

    #[test]
    fn semantic_tokens_classify_and_delta_encode() {
        // `player` is a registered command → FUNCTION (legend index 1), at 0,0 len 6
        let (toks, _) = sqf::lex("player", sqf::Dialect::Sqf);
        let st = semantic_tokens("player", &toks);
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].delta_line, 0);
        assert_eq!(st[0].delta_start, 0);
        assert_eq!(st[0].length, 6);
        assert_eq!(st[0].token_type, 1); // FUNCTION
        assert_eq!(LEGEND_TYPES[st[0].token_type as usize], SemanticTokenType::FUNCTION);
    }

    #[test]
    fn semantic_tokens_split_multiline_token() {
        // a block comment spanning two lines must become two single-line pieces
        let src = "/* a\n b */";
        let (toks, _) = config::lex(src);
        let st = semantic_tokens(src, &toks);
        let comments: Vec<_> = st.iter().filter(|t| t.token_type == 6).collect(); // COMMENT
        assert_eq!(comments.len(), 2, "multi-line comment should split per line");
    }

    #[test]
    fn diagnostics_map_spans_to_ranges() {
        let src = "hint \"oops"; // unterminated string starting at col 5
        let lex_diags = sqf::lex(src, sqf::Dialect::Sqf).1;
        let lsp = to_diagnostics(src, &lex_diags);
        assert_eq!(lsp.len(), 1);
        assert_eq!(lsp[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp[0].range.start.line, 0);
        assert_eq!(lsp[0].range.start.character, 5);
        assert_eq!(lsp[0].source.as_deref(), Some("poseidon"));
    }

    #[test]
    fn hover_renders_signature_for_known_command() {
        let md = hover_markdown("setpos").expect("setPos is a command");
        assert!(md.contains("setPos"));
        assert!(md.contains("```sqf"));
        assert!(hover_markdown("definitely_not_a_command").is_none());
    }

    #[test]
    fn hover_includes_authored_docs_when_present() {
        // setPos is in the doc corpus → description + example prose appear,
        // in addition to the catalog-derived signature.
        let md = hover_markdown("setpos").expect("setPos is a command");
        assert!(md.contains("Teleports"), "expected description, got:\n{md}");
        assert!(md.contains("player setPos"), "expected example, got:\n{md}");
        // an undocumented-but-real command still hovers (signature only)
        let acos = hover_markdown("acos").expect("acos is a command");
        assert!(acos.contains("acos"));
    }

    #[test]
    fn completion_includes_real_commands() {
        let labels: Vec<&str> = SQF_COMPLETIONS.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"player"));
        assert!(labels.contains(&"setDamage"));
        // symbolic operators are not identifier completions
        assert!(!labels.contains(&"+"));
    }

    #[test]
    fn document_symbols_from_config_tree() {
        let src = "class Mission { class Groups { class Item0 : Base {}; }; };";
        let syms = config_symbols(src);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Mission");
        assert_eq!(syms[0].kind, SymbolKind::CLASS);
        let groups = &syms[0].children.as_ref().unwrap()[0];
        assert_eq!(groups.name, "Groups");
        assert_eq!(groups.children.as_ref().unwrap()[0].detail.as_deref(), Some(": Base"));
    }
}
