use crate::IS_VERBOSE;
// use crate::SharedClientReader;
use crate::SharedClientWriter;
use crate::SharedServerWriter;
use crate::SharedStore;
use crate::lsp_diagnosis;
use crate::lsp_io;
use anyhow::Result;
use serde_json::json;
use std::io::ErrorKind;
use tokio::io::AsyncRead;
pub async fn client_to_server_loop<R>(
    mut client_reader: R,
    server_writer: SharedServerWriter,
    store: SharedStore,
) -> Result<()>
where
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

        if msg.get("method").and_then(|m| m.as_str()) == Some("textDocument/didSave") {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

            let uri = params
                .get("textDocument")
                .cloned()
                .unwrap_or_else(|| json!({}))
                .get("uri")
                .and_then(|u| u.as_str())
                .unwrap_or_default()
                .to_string();
            let mut st = store.lock().await;
            st.saved_uri = uri;
            // let _ = save_rx.send(());
        }
        // clangd へ転送
        {
            let mut w = server_writer.lock().await;
            lsp_io::write_lsp_message(&mut *w, &msg).await?;
        }
    }
    Ok(())
}

pub async fn server_to_client_loop<R>(
    mut server_reader: R,
    _server_writer: SharedServerWriter,
    client_writer: SharedClientWriter,
    store: SharedStore,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let msg = match lsp_io::read_lsp_message(&mut server_reader).await {
            Ok(v) => v,
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => {
                eprintln!("[clasangd] read from server failed: {:#}", e);
                break;
            }
        };

        // publishDiagnostics だけ横取りして合成、それ以外はそのまま転送
        let method = msg.get("method").and_then(|m| m.as_str());

        if method == Some("textDocument/publishDiagnostics") {
            if let Some(publish_msg) =
                lsp_diagnosis::handle_publish_from_clangd(msg, store.clone()).await?
            {
                let mut w = client_writer.lock().await;
                lsp_io::write_lsp_message(&mut *w, &publish_msg).await?;
            }
        } else {
            let mut w = client_writer.lock().await;
            lsp_io::write_lsp_message(&mut *w, &msg).await?;
        }
    }
    Ok(())
}
// pub async fn init(client_reader: SharedClientReader, store: SharedStore) {
//     loop {
//         let mut reader = client_reader.lock().await;
//         let msg = match lsp_io::read_lsp_message(&mut *reader).await {
//             Ok(v) => v,
//             Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
//             Err(e) => {
//                 eprintln!("[clasangd] read from client failed: {:#}", e);
//                 break;
//             }
//         };
//         unsafe {
//             if 0 < IS_VERBOSE {
//                 eprintln!("[clasangd] receive: {}", msg.get("method").unwrap());
//                 if 1 < IS_VERBOSE {
//                     eprintln!(
//                         "[clasangd] json: {}",
//                         serde_json::to_string(&msg).unwrap_or_default()
//                     );
//                 }
//             }
//         }

//         if msg.get("method").and_then(|m| m.as_str()) == Some("initialize") {
//             let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

//             let root_uri = params
//                 .get("rootUri")
//                 .and_then(|u| u.as_str())
//                 .unwrap_or_default()
//                 .to_string();
//             let mut st = store.lock().await;
//             st.root_uri = root_uri;
//             drop(reader); // Mutexを解放
//             break;
//         }
//         drop(reader); // ループ内でも解放
//     }
// }
