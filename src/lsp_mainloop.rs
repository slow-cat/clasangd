use crate::IS_VERBOSE;
use crate::SharedClientWriter;
use crate::SharedStore;
use crate::lsp_diagnosis;
use crate::lsp_io;
use anyhow::Result;
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecursiveMode, Watcher};
use serde_json::json;
use std::io::ErrorKind;
use std::path::Path;
use tokio::io::AsyncRead;
pub async fn client_to_server_loop<R>(
    mut client_reader: R,
    client_writer: SharedClientWriter,
    store: SharedStore,
) where
    R: AsyncRead + Unpin,
{
    loop {
        let msg = match lsp_io::read_lsp_message(&mut client_reader).await {
            Ok(v) => v,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => {
                eprintln!("[clasangd] read from client failed: {:#}", e);
                break;
            }
        };
        unsafe {
            if 0 < IS_VERBOSE {
                eprintln!("[clasangd] receive: {}", msg.get("method").unwrap());
                if 1 < IS_VERBOSE {
                    eprintln!(
                        "[clasangd] json: {}",
                        serde_json::to_string(&msg).unwrap_or_default()
                    );
                }
            }
        }

        if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
            if method == "textDocument/didOpen"
                || method == "textDocument/didChange"
                || method == "textDocument/didSave"
            {
                let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
                let uri = params
                    .get("textDocument")
                    .and_then(|u| u.get("uri"))
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_string();

                if !uri.is_empty() {
                    unsafe {
                        if 0 < IS_VERBOSE {
                            eprintln!("[clasangd] Setting saved_uri to: {}", uri);
                        }
                    }
                    let mut st = store.lock().await;
                    st.saved_uri = uri;
                }
            }
        }

        if msg.get("method").and_then(|m| m.as_str()) == Some("initialize") {
            let path = msg
                .get("params")
                .and_then(|p| p.get("rootPath"))
                .and_then(|u| u.as_str())
                .unwrap_or_default()
                .to_string();
            let mut st = store.lock().await;
            st.root_path = path;
            let reply = json!({
                "jsonrpc": "2.0",
                "id": msg.get("id"),
                "result": {
                    "capabilities": {
                        "textDocumentSync": 1
                    }
                }
            });
            let mut w = client_writer.lock().await;
            if let Err(e) = lsp_io::write_lsp_message(&mut *w, &reply).await {
                eprintln!("[clasangd] Failed to publish diagnostics: {:#}", e);
            }
        }
        if msg.get("method").and_then(|m| m.as_str()) == Some("shutdown") {
            let reply = json!({
            "jsonrpc": "2.0",
            "id": msg.get("id"),
            "result": null
             });
            let mut w = client_writer.lock().await;
            if let Err(e) = lsp_io::write_lsp_message(&mut *w, &reply).await {
                eprintln!("[clasangd] shutdown: {:#}", e);
            }
        }
        if msg.get("method").and_then(|m| m.as_str()) == Some("exit") {
            std::process::exit(0);
        }
    }
}
pub async fn detect_change_publish(
    build_log: String,
    run_log: String,
    client_writer: SharedClientWriter,
    store: SharedStore,
) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.blocking_send(event);
        }
    })
    .unwrap();
    watcher
        .watch(Path::new(&build_log), RecursiveMode::NonRecursive)
        .unwrap();
    watcher
        .watch(Path::new(&run_log), RecursiveMode::NonRecursive)
        .unwrap();
    let mut pending = false;
    loop {
        tokio::select! {
             Some(event) = rx.recv() => {
                 // Modifyイベントの時だけpendingをセット
                 if matches!(event.kind, EventKind::Modify(ModifyKind::Data(_))) {
                     pending = true;
                 }
             }
             _ = tokio::time::sleep(tokio::time::Duration::from_millis(300)), if pending => {
                 pending = false;

                 if let Ok(uris) = lsp_diagnosis::update_logs_store(store.clone(), &build_log, &run_log).await {
                     for uri in uris{
                     match lsp_diagnosis::create_publish_message(store.clone(),&uri).await {
                         Ok(msg) => {
                             unsafe {
                                 if 0 < IS_VERBOSE {
                                     eprintln!("[clasangd] start to publish");
                                     if 1 < IS_VERBOSE {
                                         eprintln!(
                                             "[clasangd] JSON: {}",
                                             serde_json::to_string_pretty(&msg).unwrap_or_default()
                                         );
                                     }
                                 }
                             }
                             let mut w = client_writer.lock().await;
                             if let Err(e) = lsp_io::write_lsp_message(&mut *w, &msg).await {
                                 eprintln!("[clasangd] Failed to publish diagnostics: {:#}", e);
                             }
                         }
                         Err(e) => {
                             eprintln!("[clasangd] Failed to create publish message: {:#}", e);
                         }
                     }

                     }
                 } else {
                     eprintln!("[clasangd] Failed to update logs");
                 }
             }
        };
    }
}
