use crate::IS_VERBOSE;
use crate::SharedStore;
use crate::log_parser;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::read_to_string;
pub async fn create_publish_message(store: SharedStore, uri: &str) -> Result<Value> {
    let st = store.lock().await;
    let merged = st.merged_for(uri);

    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!(
                "[clasangd] Creating publish message for {}:{} log diags",
                uri,
                st.logs.get(uri).map(|v| v.len()).unwrap_or(0)
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
pub async fn update_logs_store(
    store: SharedStore,
    build_log: &str,
    run_log: &str,
) -> Result<Vec<String>> {
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
    let old_uris: HashSet<String> = st.logs.keys().cloned().collect(); //古いやつ消すためにカラパブリッシュする
    st.set_logs(logs_by_file);
    let new_uris: HashSet<String> = st.logs.keys().cloned().collect();
    let mut pub_uris: Vec<String> = new_uris.union(&old_uris).into_iter().cloned().collect();
    pub_uris.sort();
    Ok(pub_uris)
}
