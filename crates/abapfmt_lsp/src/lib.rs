//! The `abapfmt lsp` language server: document and range formatting over stdio, nothing else.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use abapfmt_config::Config;
use abapfmt_formatter::format_checked;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
};
use lsp_types::request::{Formatting, RangeFormatting, Request as _};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentRangeFormattingParams, OneOf, Position, Range,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
};
use similar::{DiffOp, TextDiff};

/// LSP error code `RequestFailed`.
const REQUEST_FAILED: i32 = -32803;

/// Runs the server on stdin/stdout until the client shuts it down.
pub fn run() -> Result<(), String> {
    let (connection, io_threads) = Connection::stdio();
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    connection
        .initialize(serde_json::json!(capabilities))
        .map_err(|e| format!("initialize: {e}"))?;
    main_loop(&connection)?;
    drop(connection);
    io_threads.join().map_err(|e| format!("io: {e}"))?;
    Ok(())
}

fn main_loop(connection: &Connection) -> Result<(), String> {
    let mut documents: HashMap<Uri, String> = HashMap::new();
    for message in &connection.receiver {
        match message {
            Message::Request(req) => {
                if connection
                    .handle_shutdown(&req)
                    .map_err(|e| format!("shutdown: {e}"))?
                {
                    return Ok(());
                }
                let response = handle_request(&documents, req);
                connection
                    .sender
                    .send(Message::Response(response))
                    .map_err(|e| e.to_string())?;
            }
            Message::Notification(note) => handle_notification(&mut documents, note),
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_notification(documents: &mut HashMap<Uri, String>, note: Notification) {
    match note.method.as_str() {
        DidOpenTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) {
                documents.insert(p.text_document.uri, p.text_document.text);
            }
        }
        DidChangeTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(note.params)
                && let Some(change) = p.content_changes.into_iter().last()
            {
                // the server announces full synchronization
                documents.insert(p.text_document.uri, change.text);
            }
        }
        DidCloseTextDocument::METHOD => {
            if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) {
                documents.remove(&p.text_document.uri);
            }
        }
        _ => {}
    }
}

fn handle_request(documents: &HashMap<Uri, String>, req: Request) -> Response {
    let id = req.id.clone();
    let result = match req.method.as_str() {
        Formatting::METHOD => serde_json::from_value::<DocumentFormattingParams>(req.params)
            .map_err(|e| e.to_string())
            .and_then(|p| format_document(documents, &p.text_document.uri, None)),
        RangeFormatting::METHOD => {
            serde_json::from_value::<DocumentRangeFormattingParams>(req.params)
                .map_err(|e| e.to_string())
                .and_then(|p| format_document(documents, &p.text_document.uri, Some(p.range)))
        }
        other => {
            return Response::new_err(id, -32601, format!("method not supported: {other}"));
        }
    };
    match result {
        Ok(edits) => Response::new_ok(id, edits),
        Err(message) => Response::new_err(id, REQUEST_FAILED, message),
    }
}

fn format_document(
    documents: &HashMap<Uri, String>,
    uri: &Uri,
    range: Option<Range>,
) -> Result<Vec<TextEdit>, String> {
    let src = documents
        .get(uri)
        .ok_or_else(|| format!("document is not open: {}", uri.as_str()))?;
    let config = load_config(uri)?;
    let formatted = format_checked(src, &config.rules)?;
    let lines = range.map(|r| (r.start.line, r.end.line));
    Ok(line_edits(src, &formatted, lines))
}

/// The configuration for a document: the nearest `abapfmt.json`, or the defaults.
fn load_config(uri: &Uri) -> Result<Config, String> {
    let Some(dir) = uri_to_path(uri).and_then(|p| p.parent().map(PathBuf::from)) else {
        return Ok(Config::default());
    };
    match abapfmt_config::find_config(&dir) {
        Some(path) => abapfmt_config::load_file(&path).map(|l| l.config),
        None => Ok(Config::default()),
    }
}

fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = rest.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            decoded.push(b);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    let text = String::from_utf8(decoded).ok()?;
    // "/c:/dir" on Windows
    let b = text.as_bytes();
    let text = if b.len() > 2 && b[0] == b'/' && b[2] == b':' {
        text[1..].to_string()
    } else {
        text
    };
    Some(PathBuf::from(text))
}

/// Line-based edits that turn `old` into `new`. With `lines` (first and last line of the
/// selection), only the changes that touch these lines are returned.
fn line_edits(old: &str, new: &str, lines: Option<(u32, u32)>) -> Vec<TextEdit> {
    if old == new {
        return Vec::new();
    }
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let diff = TextDiff::from_slices(&old_lines, &new_lines);
    let position = |line: usize| -> Position {
        match old_lines.get(line) {
            Some(_) => Position::new(line as u32, 0),
            None => match old_lines.last() {
                Some(last) if !last.ends_with('\n') => Position::new(
                    old_lines.len() as u32 - 1,
                    last.encode_utf16().count() as u32,
                ),
                _ => Position::new(old_lines.len() as u32, 0),
            },
        }
    };
    let mut edits = Vec::new();
    for op in diff.ops() {
        let (old_range, new_range) = match *op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index..old_index + old_len,
                new_index..new_index + new_len,
            ),
        };
        if let Some((first, last)) = lines
            && old_range.len() == new_range.len()
            && !old_range.is_empty()
        {
            // line by line, so that lines outside the selection stay as they are
            for (o, n) in old_range.clone().zip(new_range.clone()) {
                if old_lines[o] != new_lines[n] && (first..=last).contains(&(o as u32)) {
                    edits.push(TextEdit {
                        range: Range::new(position(o), position(o + 1)),
                        new_text: new_lines[n].to_string(),
                    });
                }
            }
            continue;
        }
        if let Some((first, last)) = lines {
            // an insertion belongs to the selection if it lies between selected lines
            let (start, end) = (old_range.start as u32, old_range.end as u32);
            let touches = if old_range.is_empty() {
                start > first && start <= last + 1
            } else {
                start <= last && end > first
            };
            if !touches {
                continue;
            }
        }
        edits.push(TextEdit {
            range: Range::new(position(old_range.start), position(old_range.end)),
            new_text: new_lines[new_range].concat(),
        });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &str, edits: &[TextEdit]) -> String {
        // edits are whole-line and sorted: rebuild by lines
        let lines: Vec<&str> = old.split_inclusive('\n').collect();
        let mut out = String::new();
        let mut at = 0usize;
        for e in edits {
            let (s, t) = (e.range.start.line as usize, e.range.end.line as usize);
            let end_col = e.range.end.character;
            let t = if end_col > 0 { t + 1 } else { t };
            out.extend(lines[at..s].iter().copied());
            out.push_str(&e.new_text);
            at = t.min(lines.len());
        }
        out.extend(lines[at..].iter().copied());
        out
    }

    #[test]
    fn whole_document_edits_reproduce_the_result() {
        let old = "a  b\nkeep\nc   d";
        let new = "a b\nkeep\nc d\nextra\n";
        let edits = line_edits(old, new, None);
        assert_eq!(apply(old, &edits), new);
    }

    #[test]
    fn range_edits_only_touch_selected_lines() {
        let old = "a  b\nkeep\nc   d\n";
        let new = "a b\nkeep\nc d\n";
        let edits = line_edits(old, new, Some((2, 2)));
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 2);
        assert_eq!(apply(old, &edits), "a  b\nkeep\nc d\n");
    }

    #[test]
    fn file_uris_become_paths() {
        let uri: Uri = "file:///c%3A/dir/a%20b.abap".parse().unwrap();
        assert_eq!(uri_to_path(&uri), Some(PathBuf::from("c:/dir/a b.abap")));
    }
}
