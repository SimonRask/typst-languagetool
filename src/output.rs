use std::{collections::HashMap, io::stdout, io::Write, path::Path, str::Chars};

use annotate_snippets::{
	display_list::{DisplayList, FormatOptions},
	snippet::{Annotation, AnnotationType, Slice, Snippet, SourceAnnotation},
};
use languagetool_rust::{check::Match, CheckResponse};
use tower_lsp::lsp_types::{
	self, CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic, TextEdit, WorkspaceEdit,
};

pub fn output_diagnostics(
	start: &mut Position,
	response: &CheckResponse,
	total: usize,
	url: lsp_types::Url,
) -> Vec<(Diagnostic, Vec<CodeActionOrCommand>)> {
	let mut last = 0;
	let mut diagnostics = Vec::new();
	for info in &response.matches {
		start.advance(info.offset - last);
		let mut end = start.clone();
		end.advance(info.length);

		let range = lsp_types::Range {
			start: lsp_types::Position {
				line: start.line as u32 - 1,
				character: start.column as u32 - 1,
			},
			end: lsp_types::Position {
				line: end.line as u32 - 1,
				character: end.column as u32 - 1,
			},
		};
		let actions: Vec<CodeActionOrCommand> = info
			.replacements
			.iter()
			.map(|replacement| {
				CodeActionOrCommand::CodeAction(CodeAction {
					title: format!(
						"Replace with '{replacement}'",
						replacement = replacement.value
					),
					kind: Some(CodeActionKind::QUICKFIX),
					edit: Some(WorkspaceEdit {
						changes: Some(HashMap::from_iter(
							[(
								url.clone(),
								vec![TextEdit::new(range, replacement.value.clone())],
							)]
							.into_iter(),
						)),
						..Default::default()
					}),
					is_preferred: Some(true),
					..Default::default()
				})
			})
			.collect();

		let diagnostic = Diagnostic {
			range,
			message: info.message.clone(),
			// data: Some(serde_json::to_value(actions).unwrap()),
			..Default::default()
		};
		diagnostics.push((diagnostic, actions));

		last = info.offset;
	}
	start.advance(total - last);

	diagnostics
}

pub fn output_plain(file: &Path, start: &mut Position, response: &CheckResponse, total: usize) {
	let mut last = 0;
	let mut out = stdout().lock();
	for info in &response.matches {
		start.advance(info.offset - last);
		let mut end = start.clone();
		end.advance(info.length);
		writeln!(
			out,
			"{} {}:{}-{}:{} info {}",
			file.display(),
			start.line,
			start.column,
			end.line,
			end.column,
			info.message,
		)
		.unwrap();
		last = info.offset;
	}
	start.advance(total - last);
}

const PRETTY_RANGE: usize = 20;

pub fn output_pretty(file: &Path, start: &mut Position, response: &CheckResponse, total: usize) {
	let mut last = 0;
	let file_name = format!("{}", file.display());
	for info in &response.matches {
		if info.offset > PRETTY_RANGE {
			start.advance(info.offset - PRETTY_RANGE - last);
			last = info.offset - PRETTY_RANGE;
		}
		print_pretty(&file_name, start, info);
	}
	start.advance(total - last);
}

fn print_pretty(file_name: &str, start: &Position, info: &Match) {
	let start_buffer = info.offset.min(PRETTY_RANGE);

	let context = start
		.clone()
		.content
		.take(start_buffer + info.length + PRETTY_RANGE)
		.collect::<String>();

	let mut annotations = Vec::new();
	annotations.push(SourceAnnotation {
		label: &info.message,
		annotation_type: AnnotationType::Info,
		range: (start_buffer, start_buffer + info.length),
	});
	for replacement in &info.replacements {
		let pos = start_buffer + info.length + 2;
		annotations.push(SourceAnnotation {
			label: &replacement.value,
			annotation_type: AnnotationType::Help,
			range: (pos, pos),
		})
	}

	if let Some(urls) = &info.rule.urls {
		for url in urls {
			annotations.push(SourceAnnotation {
				label: &url.value,
				annotation_type: AnnotationType::Note,
				range: (2, 2),
			})
		}
	}

	let snippet = Snippet {
		title: Some(Annotation {
			label: Some(&info.rule.description),
			annotation_type: AnnotationType::Info,
			id: Some(&info.rule.id),
		}),
		footer: Vec::new(),
		slices: vec![Slice {
			source: &context,
			line_start: start.line,
			origin: Some(file_name),
			fold: true,
			annotations,
		}],
		opt: FormatOptions {
			color: true,
			anonymized_line_numbers: false,
			margin: None,
		},
	};
	println!("{}", DisplayList::from(snippet));
}

#[derive(Clone)]
pub struct Position<'a> {
	line: usize,
	column: usize,
	content: Chars<'a>,
}

impl<'a> Position<'a> {
	pub fn new(content: &'a str) -> Self {
		Self {
			line: 1,
			column: 1,
			content: content.chars(),
		}
	}

	fn advance(&mut self, amount: usize) {
		for _ in 0..amount {
			match self.content.next().unwrap() {
				'\n' => {
					self.line += 1;
					self.column = 1;
				},
				_ => {
					self.column += 1;
				},
			}
		}
	}
}
