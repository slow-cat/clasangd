// use crate::prelude::*;
use crate::IS_VERBOSE;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
pub fn parse_diagnostics(text: &str, uri: &str) -> HashMap<String, Vec<Value>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    parse_oneline(text, uri, &mut out);
    parse_san_error(text, uri, &mut out);
    out
}
pub fn parse_oneline(text: &str, saved_uri: &str, out: &mut HashMap<String, Vec<Value>>) {
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
pub fn parse_san_error(text: &str, saved_uri: &str, out: &mut HashMap<String, Vec<Value>>) {
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

pub fn make_uri(p: &str, uri: &str) -> String {
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
