use anyhow::Result;
use clap::Parser;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    sync::Arc,
};
use tokio::{
    io::{self, BufReader, Stdin, Stdout},
    sync::Mutex,
};

mod log_parser;
mod lsp_diagnosis;
mod lsp_io;
mod lsp_mainloop;
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long,help="set log file [basename] to make both of [basename]_build.log and [basename]_run.log,or set preferred log location like /tmp/[basename]",default_value_t=("/tmp/clasangd").to_string())]
    name: String,
    #[arg(short, long, help = "set verbose level", default_value_t = 0)]
    verbose: u8,
}
static mut IS_VERBOSE: u8 = 0;

#[derive(Default)]
struct DiagStore {
    logs: HashMap<String, Vec<Value>>,
    saved_uri: String,
    root_path: String,
}

impl DiagStore {
    fn set_logs(&mut self, logs: HashMap<String, Vec<Value>>) {
        self.logs = logs;
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

        if let Some(l) = self.logs.get(uri) {
            push_unique(l);
        }
        out
    }
}

type SharedStore = Arc<Mutex<DiagStore>>;
type SharedClientWriter = Arc<Mutex<Stdout>>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let is_verbose = args.verbose;
    let file_name = args.name;
    let build_log = file_name.to_string() + "_build.log";
    let run_log = file_name.to_string() + "_run.log";
    let store: SharedStore = Arc::new(Mutex::new(DiagStore::default()));
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
    let client_writer: SharedClientWriter = Arc::new(Mutex::new(io::stdout()));
    let client_reader: BufReader<Stdin> = BufReader::new(io::stdin());

    let t1 = {
        let build_log = build_log.clone();
        let run_log = run_log.clone();
        let client_writer = client_writer.clone();
        let store = store.clone();
        tokio::spawn(async move {
            let _ =
                lsp_mainloop::detect_change_publish(build_log, run_log, client_writer, store).await;
        })
    };
    let t2 = {
        let store = store.clone();
        let client_writer = client_writer.clone();
        tokio::spawn(async move {
            let _ = lsp_mainloop::client_to_server_loop(client_reader, client_writer, store).await;
        })
    };

    let _ = tokio::join!(t2, t1);
    Ok(())
}
