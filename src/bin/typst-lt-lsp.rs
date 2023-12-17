use dashmap::DashMap;
use languagetool_rust::check::Data;
use languagetool_rust::{CheckRequest, ServerClient};
use serde_json::Value;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use typst_lt::rules::Rules;
use typst_lt::{convert, output};

#[derive(Debug)]
struct Backend {
	client: Client,
	lt_client: languagetool_rust::ServerClient,
	diagnostics_map: DashMap<Url, Vec<(Diagnostic, Vec<CodeActionOrCommand>)>>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
	async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
		Ok(InitializeResult {
			server_info: None,
			capabilities: ServerCapabilities {
				text_document_sync: Some(TextDocumentSyncCapability::Kind(
					TextDocumentSyncKind::FULL,
				)),
				workspace: Some(WorkspaceServerCapabilities {
					workspace_folders: Some(WorkspaceFoldersServerCapabilities {
						supported: Some(true),
						change_notifications: Some(OneOf::Left(true)),
					}),
					file_operations: None,
				}),
				code_action_provider: Some(CodeActionProviderCapability::Options(
					CodeActionOptions {
						resolve_provider: Some(true),
						..Default::default()
					},
				)),
				..ServerCapabilities::default()
			},
		})
	}
	async fn initialized(&self, _: InitializedParams) {
		self.client
			.log_message(MessageType::INFO, "initialized!")
			.await;
	}

	async fn shutdown(&self) -> Result<()> {
		Ok(())
	}

	async fn did_open(&self, params: DidOpenTextDocumentParams) {
		self.client
			.log_message(MessageType::INFO, "file opened!")
			.await;
		self.on_change(TextDocumentItem {
			uri: params.text_document.uri,
			text: params.text_document.text,
			version: params.text_document.version,
		})
		.await
	}

	async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
		self.on_change(TextDocumentItem {
			uri: params.text_document.uri,
			text: std::mem::take(&mut params.content_changes[0].text),
			version: params.text_document.version,
		})
		.await
	}

	async fn did_save(&self, _: DidSaveTextDocumentParams) {
		self.client
			.log_message(MessageType::INFO, "file saved!")
			.await;
	}

	async fn did_close(&self, _: DidCloseTextDocumentParams) {
		self.client
			.log_message(MessageType::INFO, "file closed!")
			.await;
	}

	async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {
		self.client
			.log_message(MessageType::INFO, "configuration changed!")
			.await;
	}

	async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {
		self.client
			.log_message(MessageType::INFO, "workspace folders changed!")
			.await;
	}

	async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
		self.client
			.log_message(MessageType::INFO, "watched files have changed!")
			.await;
	}

	async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
		let uri = params.text_document.uri;
		let range = params.range;
		if let Some(diagnostics_for_file) = self.diagnostics_map.get(&uri) {
			let diagnostics_in_range: Vec<_> = diagnostics_for_file
				.iter()
				.filter(|(d, _)| range.start >= d.range.start && range.start <= d.range.end)
				.collect();
			let actions_from_diagnostics: Vec<CodeActionOrCommand> = diagnostics_in_range
				.iter()
				.flat_map(|(_, ca)| ca.clone())
				.collect();

			// actions_from_diagnostics.push(CodeActionOrCommand::CodeAction(CodeAction {
			// 	title: format!(
			// 		"{:?} - {:?} - {}",
			// 		diagnostics_for_file.len(),
			// 		diagnostics_in_range.len(),
			// 		actions_from_diagnostics.len()
			// 	),
			// 	kind: Some(CodeActionKind::QUICKFIX),
			// 	diagnostics: None,
			// 	// edit: Some(WorkspaceEdit { changes: None, ..Default::default() }),
			// 	edit: None,
			// 	command: None,
			// 	is_preferred: Some(true),
			// 	disabled: None,
			// 	data: None,
			// }));

			Ok(Some(actions_from_diagnostics))
		} else {
			Ok(Some(vec![
			// 	CodeActionOrCommand::CodeAction(CodeAction {
			// 	title: "Not found in map".to_string(),
			// 	kind: Some(CodeActionKind::QUICKFIX),
			// 	diagnostics: None,
			// 	// edit: Some(WorkspaceEdit { changes: None, ..Default::default() }),
			// 	edit: None,
			// 	command: None,
			// 	is_preferred: Some(true),
			// 	disabled: None,
			// 	data: None,
			// })
			]))
		}
	}

	async fn execute_command(&self, _: ExecuteCommandParams) -> Result<Option<Value>> {
		self.client
			.log_message(MessageType::INFO, "command executed!")
			.await;

		match self.client.apply_edit(WorkspaceEdit::default()).await {
			Ok(res) if res.applied => self.client.log_message(MessageType::INFO, "applied").await,
			Ok(_) => self.client.log_message(MessageType::INFO, "rejected").await,
			Err(err) => self.client.log_message(MessageType::ERROR, err).await,
		}

		Ok(None)
	}
}

// #[derive(Debug, Deserialize, Serialize)]
// struct InlayHintParams {
// 	path: String,
// }

// enum CustomNotification {}
// impl Notification for CustomNotification {
// 	type Params = InlayHintParams;
// 	const METHOD: &'static str = "custom/notification";
// }
struct TextDocumentItem {
	uri: Url,
	text: String,
	version: i32,
}

impl Backend {
	async fn on_change(&self, params: TextDocumentItem) {
		let rules = Rules::new();
		let root = typst_syntax::parse(&params.text);
		let data = convert::convert(&root, &rules, 10000);
		let language = "auto".to_string();
		let mut diagnostics: Vec<(Diagnostic, Vec<CodeActionOrCommand>)> = vec![];
		let mut position = output::Position::new(&params.text);
		for items in data {
			let req = CheckRequest::default()
				.with_language(language.clone())
				.with_data(Data::from_iter(items.0));

			let response = &self.lt_client.check(&req).await;
			match response {
				Ok(response) => diagnostics.extend(output::output_diagnostics(
					&mut position,
					response,
					items.1,
					params.uri.clone(),
				)),
				Err(err) => {
					self.client
						.log_message(MessageType::ERROR, &err.to_string())
						.await;
				},
			}
		}

		self.diagnostics_map
			.insert(params.uri.clone(), diagnostics.clone());

		self.client
			.publish_diagnostics(
				params.uri.clone(),
				diagnostics.iter().map(|(d, _)| d.clone()).collect(),
				Some(params.version),
			)
			.await;
	}
}

#[tokio::main]
async fn main() {
	let stdin = tokio::io::stdin();
	let stdout = tokio::io::stdout();

	let (service, socket) = LspService::build(|client| Backend {
		client,
		lt_client: ServerClient::new("http://127.0.0.1", "8081"),
		diagnostics_map: DashMap::new(),
	})
	.finish();

	Server::new(stdin, stdout, socket).serve(service).await;
}
