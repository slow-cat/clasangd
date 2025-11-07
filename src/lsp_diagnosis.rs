use crate::IS_VERBOSE;
use crate::SharedStore;
use crate::log_parser;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs::read_to_string;
pub async fn handle_publish_from_clangd(msg: Value, store: SharedStore) -> Result<Option<Value>> {
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

    let uri = params
        .get("uri")
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();

    let version = params.get("version").cloned().unwrap_or(json!(0));
    let clang_diags = params
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!(
                "[clasangd] Received from clangd for {}: {} diagnostics",
                uri,
                clang_diags.len()
            );
        }
    }

    {
        let mut st = store.lock().await;
        st.set_clang(&uri, clang_diags);
    }

    let merged = {
        let st = store.lock().await;
        unsafe {
            if 0 < IS_VERBOSE {
                eprintln!(
                    "[clasangd] Publishing {}.ver for {}: {} clang diags, {} log diags",
                    version,
                    uri,
                    st.clang.get(&uri).map(|v| v.len()).unwrap_or(0),
                    st.logs.get(&uri).map(|v| v.len()).unwrap_or(0)
                );
            }
        }
        st.merged_for(&uri)
    };

    let out_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "version": version,
            "diagnostics": merged
        }
    });

    Ok(Some(out_msg))
}
pub async fn create_publish_message(store: SharedStore) -> Result<Value> {
    let st = store.lock().await;
    let uri = st.saved_uri.clone();
    let merged = st.merged_for(&uri);

    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!(
                "[clasangd] Creating publish message for {}: {} clang diags, {} log diags",
                uri,
                st.clang.get(&uri).map(|v| v.len()).unwrap_or(0),
                st.logs.get(&uri).map(|v| v.len()).unwrap_or(0)
            );
        }
    }

    Ok(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "version": null,
            "diagnostics": merged
        }
    }))
}
pub async fn update_logs_store(store: SharedStore, build_log: &str, run_log: &str) -> Result<()> {
    let txt = read_to_string(build_log).unwrap_or_default()
        + &read_to_string(run_log).unwrap_or_default();
    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!("[clasangd] Reading logs, total size: {} bytes", txt.len());
        }
    }
    let saved_uri = {
        let st = store.lock().await;
        st.saved_uri.clone()
    };

    let logs_by_file = log_parser::parse_diagnostics(&txt, &saved_uri);

    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!("[clasangd] Parsed logs for {} files", logs_by_file.len());
        }
    }
    for (uri, diags) in &logs_by_file {
        eprintln!("[clasangd]   {}: {} diagnostics", uri, diags.len());
    }

    let mut st = store.lock().await;
    st.set_logs(logs_by_file);

    Ok(())
}
