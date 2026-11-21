//! Live check against a real `ruby-lsp`. Ignored by default because it needs
//! the server installed: `cargo test -p lsp -- --ignored`.

use std::sync::Arc;

use lsp::lsp_types::{self, notification, request};
use lsp::{LanguageServer, path_to_uri};

#[test]
#[ignore]
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
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
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
