use crate::IS_VERBOSE;
use crate::SharedStore;
use crate::log_parser;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs::read_to_string;
pub async fn create_publish_message(store: SharedStore) -> Result<Value> {
    let st = store.lock().await;
    let uri = st.saved_uri.clone();
    let merged = st.merged_for(&uri);

    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!(
                "[clasangd] Creating publish message for {}:{} log diags",
                uri,
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
    let root_uri = {
        let st = store.lock().await;
        st.root_path.clone()
    };

    let logs_by_file = log_parser::parse_diagnostics(&txt, &saved_uri, &root_uri);

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
