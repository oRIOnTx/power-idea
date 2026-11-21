//! Manual probe of a language server against a real project:
//! `LSP_PROBE_ROOT=/path/to/app cargo test -p lsp --test project_probe -- --ignored --nocapture`

use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp::lsp_types::{self, notification, request};
use lsp::{LanguageServer, path_to_uri};

#[test]
#[ignore]
fn probe_ruby_project() {
    let Ok(root) = std::env::var("LSP_PROBE_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root).canonicalize().unwrap();
    let started = Instant::now();
    let env: Vec<(String, String)> = std::env::var("LSP_PROBE_GEMFILE")
        .map_or_default(|g| vec![("BUNDLE_GEMFILE".to_string(), g)]);
    let server = LanguageServer::start(
        "ruby",
        "ruby-lsp",
        &[],
        &env,
        &root,
        Arc::new(|method, params| {
            if method == "window/showMessage" || method == "window/logMessage" {
                eprintln!(
                    "[{method}] {}",
                    params["message"]
                        .as_str()
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or("")
                );
            }
        }),
    )
    .expect("start");
    eprintln!("initialized in {:?}", started.elapsed());
    for cap in [
        "definitionProvider",
        "hoverProvider",
        "completionProvider",
        "referencesProvider",
        "workspaceSymbolProvider",
    ] {
        eprintln!("{cap}: {}", server.supports(cap));
    }

    // First model file that subclasses ApplicationRecord.
    let models = std::fs::read_dir(root.join("app/models")).unwrap();
    let (path, text, line, col) = models
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "rb"))
        .find_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let (line, l) = text
                .lines()
                .enumerate()
                .find(|(_, l)| l.contains("< ApplicationRecord"))?;
            let col = l.find("ApplicationRecord")? as u32 + 2;
            Some((p, text, line as u32, col))
        })
        .expect("no model found");
    eprintln!("probing {} at {}:{}", path.display(), line + 1, col + 1);

    let uri = path_to_uri(&path).unwrap();
    server
        .notify::<notification::DidOpenTextDocument>(lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri: uri.clone(),
                language_id: "ruby".into(),
                version: 1,
                text,
            },
        })
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let response = server.request::<request::GotoDefinition>(lsp_types::GotoDefinitionParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: lsp_types::Position::new(line, col),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        });
        eprintln!(
            "[{:>5.1?}] definition -> {:?}",
            started.elapsed(),
            response.as_ref().map(|r| r.as_ref().map(|_| "found"))
        );
        if matches!(&response, Ok(Some(_))) || Instant::now() > deadline {
            eprintln!("{response:#?}");
            break;
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}
