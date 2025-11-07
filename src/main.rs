use anyhow::{Context, Result};
use clap::Parser;
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecursiveMode, Watcher};
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{File, read_to_string},
    io::ErrorKind,
    path::Path,
    sync::Arc,
};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, Stdout},
    process::Command,
    sync::Mutex,
};
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long,help="log file basename,or preferred log location like /tmp/clasangd",default_value_t=("/tmp/clasangd").to_string())]
    name: String,
    #[arg(short, long, help = "set verbose level", default_value_t = 0)]
    verbose: u8,
}
static mut IS_VERBOSE: u8 = 0;

#[derive(Default)]
struct DiagStore {
    // uri("file:///...") -> diagnostics(JSON array)
    clang: HashMap<String, Vec<Value>>,
    logs: HashMap<String, Vec<Value>>,
    saved_uri: String,
}

impl DiagStore {
    fn set_logs(&mut self, logs: HashMap<String, Vec<Value>>) {
        self.logs = logs;
    }
    fn set_clang(&mut self, uri: &str, diags: Vec<Value>) {
        self.clang.insert(uri.to_string(), diags);
    }
    // fn all_uris(&self) -> Vec<String> {
    //     let mut uris_tree: BTreeSet<String> = BTreeSet::new();
    //     for k in self.clang.keys() {
    //         uris_tree.insert(k.clone());
    //     }
    //     for k in self.logs.keys() {
    //         uris_tree.insert(k.clone());
    //     }
    //     uris_tree.into_iter().collect()
    // }
    fn merged_for(&self, uri: &str) -> Vec<Value> {
        // 同じ range+severity+source+message は1個にまとめる
        #[derive(Hash, Eq, PartialEq)]
        struct Key {
            sl: u64,
            sc: u64,
            el: u64,
            ec: u64,
            sev: i64,
            src: String,
            msg: String,
        }
        let mut seen: HashSet<Key> = HashSet::new();
        let mut out = Vec::new();

        let mut push_unique = |arr: &Vec<Value>| {
            for d in arr {
                let range = &d["range"];
                let s = &range["start"];
                let e = &range["end"];
                let key = Key {
                    sl: s["line"].as_u64().unwrap_or(0),
                    sc: s["character"].as_u64().unwrap_or(0),
                    el: e["line"].as_u64().unwrap_or(0),
                    ec: e["character"].as_u64().unwrap_or(0),
                    sev: d["severity"].as_i64().unwrap_or(0),
                    src: d["source"].as_str().unwrap_or("").to_string(),
                    msg: d["message"].as_str().unwrap_or("").to_string(),
                };
                if seen.insert(key) {
                    out.push(d.clone());
                }
            }
        };

        if let Some(c) = self.clang.get(uri) {
            push_unique(c);
        }
        if let Some(l) = self.logs.get(uri) {
            push_unique(l);
        }
        out
    }
}

type SharedStore = Arc<Mutex<DiagStore>>;
type SharedClientWriter = Arc<Mutex<Stdout>>;
type SharedServerWriter = Arc<Mutex<tokio::process::ChildStdin>>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let is_verbose = args.verbose;
    let file_name = args.name;
    let build_log = file_name.to_string() + "_build.log";
    let run_log = file_name.to_string() + "_run.log";
    unsafe {
        IS_VERBOSE = is_verbose;
    }

    if let Err(e) = File::create(build_log.clone()) {
        eprintln!("[clasangd] failed to create {}  error: {:#}", &build_log, e);
    } else {
        unsafe {
            if 0 < IS_VERBOSE {
                eprintln!("[clasangd] succesed to create {}", &build_log);
            }
        }
    }
    if let Err(e) = File::create(run_log.clone()) {
        eprintln!("[clasangd] failed to create {}  error: {:#}", &run_log, e);
    } else {
        unsafe {
            if 0 < IS_VERBOSE {
                eprintln!("[clasangd] succesed to create {}", &run_log);
            }
        }
    }
    // clangd 起動
    let mut child = Command::new("clangd")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("failed to spawn clangd")?;

    let server_stdout = child.stdout.take().context("no clangd stdout")?;
    let server_reader = BufReader::new(server_stdout);

    let server_stdin = child.stdin.take().context("no clangd stdin")?;
    let server_writer: SharedServerWriter = Arc::new(Mutex::new(server_stdin));

    let client_reader = BufReader::new(io::stdin());
    let client_writer: SharedClientWriter = Arc::new(Mutex::new(io::stdout()));

    let store: SharedStore = Arc::new(Mutex::new(DiagStore::default()));
    // let (save_tx, _save_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let (change_tx, mut change_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // client -> server
    let client2server = {
        let server_writer = server_writer.clone();
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = client_to_server_loop(client_reader, server_writer, store).await {
                eprintln!("[clasangd] client_to_server error: {:#}", e);
            }
        })
    };
    let detect_change = {
        let change_tx = change_tx.clone();
        let build_log = build_log.clone();
        let run_log = run_log.clone();
        tokio::spawn(async move {
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
            while let Some(event) = rx.recv().await {
                match event.kind {
                    EventKind::Modify(ModifyKind::Data(_)) => {
                        let _ = change_tx.send(());
                    }
                    _ => {}
                }
            }
        })
    };
    let publish2server = {
        let client_writer = client_writer.clone();
        let store = store.clone();
        tokio::spawn(async move {
            while let Some(_) = change_rx.recv().await {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if let Err(e) = update_logs_store(store.clone(), &build_log, &run_log).await {
                    eprintln!("[clasangd] Failed to update logs: {:#}", e);
                    continue;
                }
                if let Err(e) = publish_all_diagnostics(store.clone(), client_writer.clone()).await
                {
                    eprintln!("[clasangd] Failed to publish diagnostics: {:#}", e);
                }
            }
        })
    };

    // server -> client
    let server2client = {
        let server_writer = server_writer.clone();
        let client_writer = client_writer.clone();
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) =
                server_to_client_loop(server_reader, server_writer, client_writer, store).await
            {
                eprintln!("[clasangd] server_to_client error: {:#}", e);
            }
        })
    };

    let _ = tokio::join!(server2client, detect_change, publish2server, client2server);
    Ok(())
}

// ====================== LSP IO ======================

async fn read_lsp_message<R>(r: &mut R) -> io::Result<Value>
where
    R: AsyncRead + Unpin,
{
    let mut header = Vec::new();
    let mut buf = [0u8; 1];

    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            if header.is_empty() {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "eof"));
            } else {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "eof in header"));
            }
        }
        header.push(buf[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8_lossy(&header);
    let len = header_str
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
    Ok(v)
}

async fn write_lsp_message<W>(w: &mut W, msg: &Value) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(msg)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(header.as_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

// ====================== メインループ ======================

async fn client_to_server_loop<R>(
    mut client_reader: R,
    server_writer: SharedServerWriter,
    store: SharedStore,
    // save_rx: tokio::sync::mpsc::UnboundedSender<()>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let msg = match read_lsp_message(&mut client_reader).await {
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
                        "[clasangd] full message: {}",
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
            write_lsp_message(&mut *w, &msg).await?;
        }
    }
    Ok(())
}

async fn server_to_client_loop<R>(
    mut server_reader: R,
    _server_writer: SharedServerWriter,
    client_writer: SharedClientWriter,
    store: SharedStore,
) -> Result<()>
where
    R: AsyncRead + Unpin,
{
    loop {
        let msg = match read_lsp_message(&mut server_reader).await {
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
            handle_publish_from_clangd(msg, client_writer.clone(), store.clone()).await?;
        } else {
            let mut w = client_writer.lock().await;
            write_lsp_message(&mut *w, &msg).await?;
        }
    }
    Ok(())
}

// ====================== 診断まわり ======================

async fn handle_publish_from_clangd(
    msg: Value,
    client_writer: SharedClientWriter,
    store: SharedStore,
) -> Result<()> {
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

    // 合成して publish
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

    let mut w = client_writer.lock().await;
    write_lsp_message(&mut *w, &out_msg).await?;
    Ok(())
}
async fn publish_all_diagnostics(
    store: SharedStore,
    client_writer: SharedClientWriter,
) -> Result<()> {
    let st = store.lock().await;
    let uri = st.saved_uri.clone();
    let merged = { st.merged_for(&uri) };
    let out_msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "version": null,
            "diagnostics": merged
        }
    });
    let mut w = client_writer.lock().await;
    write_lsp_message(&mut *w, &out_msg).await?;
    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!(
                "[clasangd] Publishing at didSave for {}: {} clang diags, {} log diags",
                uri,
                st.clang.get(&uri).map(|v| v.len()).unwrap_or(0),
                st.logs.get(&uri).map(|v| v.len()).unwrap_or(0)
            );
        }
    }

    Ok(())
}
async fn update_logs_store(store: SharedStore, build_log: &str, run_log: &str) -> Result<()> {
    let txt = read_to_string(build_log).unwrap_or_default()
        + &read_to_string(run_log).unwrap_or_default();
    unsafe {
        if 0 < IS_VERBOSE {
            // eprintln!("[clasangd] {}", txt);
            eprintln!("[clasangd] Reading logs, total size: {} bytes", txt.len());
        }
    }
    let mut st = store.lock().await;
    let logs_by_file = parse_diagnostics(&txt, &st.saved_uri); // uri -> Vec<diag>

    unsafe {
        if 0 < IS_VERBOSE {
            eprintln!("[clasangd] Parsed logs for {} files", logs_by_file.len());
        }
    }
    for (uri, diags) in &logs_by_file {
        eprintln!("[clasangd]   {}: {} diagnostics", uri, diags.len());
    }

    st.set_logs(logs_by_file);

    Ok(())
}

// ====================== ログパーサ ======================

fn parse_diagnostics(text: &str, uri: &str) -> HashMap<String, Vec<Value>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    parse_oneline(text, uri, &mut out);
    parse_san_error(text, uri, &mut out);
    out
}
fn parse_oneline(text: &str, saved_uri: &str, out: &mut HashMap<String, Vec<Value>>) {
    // ex. /path/to/a.c:12:34: error: message...
    // ex. test.c:12:34: ... it is relative form in build log
    let re = Regex::new(r"(?m)^(.+?):(\d+):(\d+):\s*(error|warning|runtime error):\s*(.*)$")
        .expect("invalid regex");

    for cap in re.captures_iter(text) {
        let uri = make_uri(&cap[1], saved_uri);
        let line = cap[2].parse::<u64>().unwrap_or(1).saturating_sub(1);
        let col = cap[3].parse::<u64>().unwrap_or(1).saturating_sub(1);
        let sev = if &cap[4] == "warning" { 2 } else { 1 }; // 1=Error,2=Warning
        let msg = cap[5].to_string();

        let diag = json!({
            "range": {
                "start": { "line": line, "character": col },
                "end":   { "line": line, "character": col + 1 }
            },
            "severity": sev,
            "source": "ubsan/asan",
            "message": msg
        });

        out.entry(uri).or_default().push(diag);
    }
}
fn parse_san_error(text: &str, saved_uri: &str, out: &mut HashMap<String, Vec<Value>>) {
    // ex.
    // =================================================================
    // ==2481858==ERROR: AddressSanitizer: attempting double-free on 0x7b8f5e9e0010 in thread T0:
    //     #0 0x5602fa52ec1d in free.part.0 asan_malloc_linux.cpp.o
    //     #1 0x5602fa581047 in main /home/moamoa/clasangd/test.c:12:3
    //     #2 0x7f6f5f627674 in __libc_start_call_main /usr/src/debug/glibc/glibc/csu/../sysdeps/nptl/libc_start_call_main.h:58:16
    //     #3 0x7f6f5f627728 in __libc_start_main /usr/src/debug/glibc/glibc/csu/../csu/libc-start.c:360:3
    //     #4 0x5602fa42c094 in _start (/tmp/c_test+0x2c094) (BuildId: 874c5f694bce2854ed46b36ec29f1e765611eb8a)

    // 0x7b8f5e9e0010 is located 0 bytes inside of 5-byte region [0x7b8f5e9e0010,0x7b8f5e9e0015)
    // freed by thread T0 here:
    //     #0 0x5602fa52ec1d in free.part.0 asan_malloc_linux.cpp.o
    //     #1 0x5602fa58103b in main /home/moamoa/clasangd/test.c:11:3
    //     #2 0x7f6f5f627674 in __libc_start_call_main /usr/src/debug/glibc/glibc/csu/../sysdeps/nptl/libc_start_call_main.h:58:16
    //     #3 0x7f6f5f627728 in __libc_start_main /usr/src/debug/glibc/glibc/csu/../csu/libc-start.c:360:3
    //     #4 0x5602fa42c094 in _start (/tmp/c_test+0x2c094) (BuildId: 874c5f694bce2854ed46b36ec29f1e765611eb8a)

    // previously allocated by thread T0 here:
    //     #0 0x5602fa52fb85 in malloc (/tmp/c_test+0x12fb85) (BuildId: 874c5f694bce2854ed46b36ec29f1e765611eb8a)
    //     #1 0x5602fa580fba in main /home/moamoa/clasangd/test.c:9:5
    //     #2 0x7f6f5f627674 in __libc_start_call_main /usr/src/debug/glibc/glibc/csu/../sysdeps/nptl/libc_start_call_main.h:58:16
    //     #3 0x7f6f5f627728 in __libc_start_main /usr/src/debug/glibc/glibc/csu/../csu/libc-start.c:360:3
    //     #4 0x5602fa42c094 in _start (/tmp/c_test+0x2c094) (BuildId: 874c5f694bce2854ed46b36ec29f1e765611eb8a)

    // SUMMARY: AddressSanitizer: double-free asan_malloc_linux.cpp.o in free.part.0
    // ==2481858==ABORTING
    let re_san =
        Regex::new(r"^==\d+==ERROR:\s+([A-Za-z]+Sanitizer):\s*(.*)$").expect("invalid regex");
    let re_frame =
        Regex::new(r"^\s*#(\d+)\s+\S+\s+in\s+\S+\s+(/[^:]+):(\d+):(\d+)").expect("invalid regex");
    let mut current_kind: Option<String> = None;
    let mut current_msg: Option<String> = None;

    for line in text.lines() {
        if let Some(cap) = re_san.captures(line) {
            current_kind = Some(cap[1].to_string());
            current_msg = Some(cap[2].to_string());
            continue;
        }
        if let (Some(kind), Some(msg), Some(cap)) = (
            current_kind.as_ref(),
            current_msg.as_ref(),
            re_frame.captures(line),
        ) {
            let frame_num: u32 = cap[1].parse().unwrap_or(999);
            if frame_num != 1 {
                continue;
            }
            let uri = make_uri(&cap[2], saved_uri);
            let line = cap[3].parse::<u64>().unwrap_or(1).saturating_sub(1);
            let col = cap[4].parse::<u64>().unwrap_or(1).saturating_sub(1);
            let sev = 1; // 1=Error,2=Warning

            let diag = json!({
                "range": {
                    "start": { "line": line, "character": col },
                    "end":   { "line": line, "character": col + 1 }
                },
                "severity": sev,
                "source": format!("sanitizer/{}",kind),
                "message": msg
            });

            out.entry(uri).or_default().push(diag);
        };
    }
}

fn make_uri(p: &str, uri: &str) -> String {
    match std::fs::canonicalize(p) {
        Ok(path) => format!("file://{}", path.display().to_string()),
        Err(e) => {
            unsafe {
                if 0 < IS_VERBOSE {
                    eprintln!(
                        "[clasangd] failed to canonicalize {} use {} instead... error: {:#}",
                        p, uri, e
                    );
                }
            }
            uri.to_string()
        }
    }
}
