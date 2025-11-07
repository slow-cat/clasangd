use anyhow::{Context, Result};
use clap::Parser;
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecursiveMode, Watcher};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    path::Path,
    sync::Arc,
};
use tokio::{
    io::{self, BufReader, Stdout},
    process::Command,
    sync::Mutex,
};
mod log_parser;
mod lsp_diagnosis;
mod lsp_io;
mod lsp_mainloop;
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
    fn merged_for(&self, uri: &str) -> Vec<Value> {
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
            if let Err(e) =
                lsp_mainloop::client_to_server_loop(client_reader, server_writer, store).await
            {
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
                if let Err(e) =
                    lsp_diagnosis::update_logs_store(store.clone(), &build_log, &run_log).await
                {
                    eprintln!("[clasangd] Failed to update logs: {:#}", e);
                    continue;
                }
                match lsp_diagnosis::create_publish_message(store.clone()).await {
                    Ok(msg) => {
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
        })
    };

    // server -> client
    let server2client = {
        let server_writer = server_writer.clone();
        let client_writer = client_writer.clone();
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = lsp_mainloop::server_to_client_loop(
                server_reader,
                server_writer,
                client_writer,
                store,
            )
            .await
            {
                eprintln!("[clasangd] server_to_client error: {:#}", e);
            }
        })
    };

    let _ = tokio::join!(server2client, detect_change, publish2server, client2server);
    Ok(())
}
