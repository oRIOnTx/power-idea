//! Live check against a real `ruby-lsp`. Ignored by default because it needs
//! the server installed: `cargo test -p lsp -- --ignored`.

use std::sync::Arc;

use lsp::lsp_types::{self, notification, request};
use lsp::{LanguageServer, path_to_uri};

#[test]
#[ignore = "needs a live ruby-lsp installed"]
fn ruby_lsp_goes_to_definition_across_files() {
    let dir = std::env::temp_dir().join(format!("power-idea-ruby-lsp-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/greeter.rb"),
        "class Greeter\n  def hello\n    \"hi\"\n  end\nend\n",
    )
    .unwrap();
    let main = "require_relative \"lib/greeter\"\nGreeter.new.hello\n";
    std::fs::write(dir.join("main.rb"), main).unwrap();
    let dir = dir.canonicalize().unwrap();

    let server = LanguageServer::start("ruby", "ruby-lsp", &[], &[], &dir, Arc::new(|_, _| {}))
        .expect("start ruby-lsp");
    let uri = path_to_uri(&dir.join("main.rb")).unwrap();
    server
        .notify::<notification::DidOpenTextDocument>(lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri: uri.clone(),
                language_id: "ruby".into(),
                version: 1,
                text: main.into(),
            },
        })
        .unwrap();

    // ruby-lsp indexes the project in the background; retry briefly.
    let mut links = Vec::new();
    for _ in 0..40 {
        let response = server
            .request::<request::GotoDefinition>(lsp_types::GotoDefinitionParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                    position: lsp_types::Position::new(1, 2), // on `Greeter`
                },
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                partial_result_params: lsp_types::PartialResultParams::default(),
            })
            .unwrap();
        links = match response {
            Some(lsp_types::GotoDefinitionResponse::Array(l)) => l.into_iter().map(|l| l.uri).collect(),
            Some(lsp_types::GotoDefinitionResponse::Scalar(l)) => vec![l.uri],
            Some(lsp_types::GotoDefinitionResponse::Link(l)) => l.into_iter().map(|l| l.target_uri).collect(),
            None => Vec::new(),
        };
        if !links.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        links.iter().any(|u| u.as_str().ends_with("lib/greeter.rb")),
        "definition not found: {links:?}"
    );
}

#[test]
#[ignore = "needs a live ruby-lsp installed"]
fn ruby_lsp_navigation_and_refactoring() {
    use lsp::nav;

    let dir = std::env::temp_dir().join(format!("power-idea-ruby-nav-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "class Greeter\n  def hello\n    name = \"x\"\n    name\n  end\nend\n";
    std::fs::write(dir.join("greeter.rb"), source).unwrap();
    let dir = dir.canonicalize().unwrap();

    let server = LanguageServer::start("ruby", "ruby-lsp", &[], &[], &dir, Arc::new(|_, _| {}))
        .expect("start ruby-lsp");
    let uri = path_to_uri(&dir.join("greeter.rb")).unwrap();
    let doc = lsp_types::TextDocumentIdentifier { uri: uri.clone() };
    server
        .notify::<notification::DidOpenTextDocument>(lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri,
                language_id: "ruby".into(),
                version: 1,
                text: source.into(),
            },
        })
        .unwrap();

    let symbols = server
        .request::<request::DocumentSymbolRequest>(lsp_types::DocumentSymbolParams {
            text_document: doc.clone(),
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        })
        .unwrap();
    let outline: Vec<_> = nav::document_symbols(symbols)
        .into_iter()
        .map(|s| (s.name, s.depth))
        .collect();
    assert!(outline.contains(&("Greeter".into(), 0)), "{outline:?}");
    assert!(outline.contains(&("hello".into(), 1)), "{outline:?}");

    // Workspace symbols need the index; retry briefly.
    let mut found = Vec::new();
    for _ in 0..40 {
        let response = server
            .request::<request::WorkspaceSymbolRequest>(lsp_types::WorkspaceSymbolParams {
                query: "Greeter".into(),
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
                partial_result_params: lsp_types::PartialResultParams::default(),
            })
            .unwrap();
        found = nav::workspace_symbols(response);
        if found.iter().any(|s| s.name == "Greeter") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(
        found
            .iter()
            .any(|s| s.name == "Greeter" && nav::is_type_like(s.kind))
    );

    let on_name = lsp_types::TextDocumentPositionParams {
        text_document: doc.clone(),
        position: lsp_types::Position::new(3, 5),
    };
    let references = server
        .request::<request::References>(lsp_types::ReferenceParams {
            text_document_position: on_name,
            context: lsp_types::ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            partial_result_params: lsp_types::PartialResultParams::default(),
        })
        .unwrap()
        .unwrap_or_default();
    println!("references: {}", references.len());

    if server.supports("renameProvider") {
        let edit = server
            .request::<request::Rename>(lsp_types::RenameParams {
                text_document_position: lsp_types::TextDocumentPositionParams {
                    text_document: doc.clone(),
                    position: lsp_types::Position::new(0, 7),
                },
                new_name: "Welcomer".into(),
                work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
            })
            .unwrap();
        if let Some(edit) = edit {
            let files = nav::workspace_edit_files(edit).unwrap();
            let (_, edits) = files.iter().find(|(p, _)| p.ends_with("greeter.rb")).unwrap();
            let renamed = nav::apply_text_edits(source, edits).unwrap();
            assert!(renamed.starts_with("class Welcomer\n"), "{renamed}");
        }
    }

    let formatting = server.request::<request::Formatting>(lsp_types::DocumentFormattingParams {
        text_document: doc,
        options: lsp_types::FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            ..Default::default()
        },
        work_done_progress_params: lsp_types::WorkDoneProgressParams::default(),
    });
    println!("formatting: {formatting:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
