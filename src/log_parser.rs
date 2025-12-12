// use crate::prelude::*;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

pub fn parse_diagnostics(text: &str, uri: &str, root: &str) -> HashMap<String, Vec<Value>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    parse_oneline(text, uri, root, &mut out);
    parse_san_error(text, uri, root, &mut out);
    parse_stacktrace(text, uri, root, &mut out);
    parse_traceback(text, uri, root, &mut out);
    out
}
pub fn parse_oneline(
    text: &str,
    saved_uri: &str,
    root_path: &str,
    out: &mut HashMap<String, Vec<Value>>,
) {
    // ex. /path/to/a.c:12:34: error: message...
    // ex. test.c:12:34: ... it is relative form in build log
    let re = Regex::new(r"(?m)^(.+?):(\d+):(\d+):\s*(error|warning|runtime error|note):\s*(.*)$")
        .expect("invalid regex");

    for cap in re.captures_iter(text) {
        let uri = make_uri(&cap[1], saved_uri, root_path);
        let line = cap[2].parse::<u64>().unwrap_or(1).saturating_sub(1);
        let col = cap[3].parse::<u64>().unwrap_or(1).saturating_sub(1);
        let sev = match &cap[4] {
            "note" => 3,
            "warning" => 2,
            "error" | "runtime error" => 1,
            _ => 1,
        };
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
pub fn parse_san_error(
    text: &str,
    saved_uri: &str,
    root_path: &str,
    out: &mut HashMap<String, Vec<Value>>,
) {
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
            let uri = make_uri(&cap[2], saved_uri, root_path);
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

pub fn make_uri(p: &str, uri: &str, root: &str) -> String {
    if let Ok(path) = std::fs::canonicalize(p) {
        return format!("file://{}", path.display());
    }

    let root_relative = format!("{}/{}", root, p);
    if let Ok(path) = std::fs::canonicalize(&root_relative) {
        return format!("file://{}", path.display());
    }

    if let Some(filename) = Path::new(p).file_name().and_then(|f| f.to_str()) {
        if let Some(found) = find_file_bfs(root, filename) {
            return format!("file://{}", found.display());
        }
    }

    uri.to_string()
}

fn find_file_bfs(root: &str, filename: &str) -> Option<PathBuf> {
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return None;
    }

    let mut queue = VecDeque::new();
    queue.push_back(root_path.to_path_buf());
    let mut visited = HashSet::new();

    while let Some(dir) = queue.pop_front() {
        if !visited.insert(dir.clone()) {
            continue;
        }

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                if path.is_file() {
                    if let Some(name) = path.file_name() {
                        if name == filename {
                            return Some(path);
                        }
                    }
                } else if path.is_dir() && !is_ignore_dir(&path) {
                    queue.push_back(path);
                }
            }
        }
    }

    None
}

fn is_ignore_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n.starts_with('.')
                || n == "target"
                || n == "node_modules"
                || n == "build"
                || n == "dist"
        })
        .unwrap_or(false)
}

pub fn parse_stacktrace(
    text: &str,
    saved_uri: &str,
    root_path: &str,
    out: &mut HashMap<String, Vec<Value>>,
) {
    // Exception in thread "main" java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0
    // 	at crash.main(crash.java:4)
    let re_exception = Regex::new(r"(?m)^(?:Exception in thread .+?|Traceback|.*?Error):\s*(.+)")
        .expect("invalid regex");

    let re_at = Regex::new(r"^\s+at\s+.+?\((.+?):(\d+)\)").expect("invalid regex");

    let mut current_exception: Option<String> = None;

    for line in text.lines() {
        if let Some(cap) = re_exception.captures(line) {
            let message = cap.get(1).map(|m| m.as_str()).unwrap_or("Runtime error");
            current_exception = Some(message.to_string());
            continue;
        }

        if let Some(ref exc_msg) = current_exception {
            if let Some(cap) = re_at.captures(line) {
                let file = &cap[1];
                let line_num = cap[2].parse::<u64>().unwrap_or(1).saturating_sub(1);
                let uri = make_uri(file, saved_uri, root_path);

                let diag = json!({
                    "range": {
                        "start": { "line": line_num, "character": 0 },
                        "end": { "line": line_num, "character": 1 }
                    },
                    "severity": 1,
                    "source": "runtime",
                    "message": exc_msg
                });

                out.entry(uri).or_default().push(diag);
                current_exception = None;
                continue;
            }
        }
    }
}
pub fn parse_traceback(
    text: &str,
    saved_uri: &str,
    root_path: &str,
    out: &mut HashMap<String, Vec<Value>>,
) {
    // Traceback (most recent call last):
    //   File "/home/moamoa/report/b_tree.py", line 66, in <module>
    //     tree.b_tree_insert(i)
    //     ~~~~~~~~~~~~~~~~~~^^^
    //   File "/home/moamoa/report/b_tree.py", line 63, in b_tree_insert
    //     self.b_tree_insert_nonfull(r,k)
    //     ~~~~~~~~~~~~~~~~~~~~~~~~~~^^^^^
    //   File "/home/moamoa/report/b_tree.py", line 52, in b_tree_insert_nonfull
    //     if x.c[i].n==2*self.t-1:
    //        ~~~^^^
    // IndexError: list index out of range
    let re_error = Regex::new(r"^(?P<msg>(?:\w+Error|Exception)[^\r\n]*)").expect("invalid regex");

    let re_at = Regex::new(r#"^\s*File\s+"([^"]+)",\s+line\s+(\d+),(?:\s+in\s+(.+))?"#)
        .expect("invalid regex");
    // let re_line=Regex::new(r"\^+").expect("invalid regex");

    let mut current_exception: Option<String> = None;
    let mut sev = 1;

    for line in text.lines().rev() {
        if let Some(cap) = re_error.captures(line) {
            let message = cap
                .name("msg")
                .map(|m| m.as_str())
                .unwrap_or("Runtime error");
            current_exception = Some(message.to_string());
            continue;
        }
        if let Some(ref exc_msg) = current_exception {
            if let Some(cap) = re_at.captures(line) {
                let file = &cap[1];
                let line_num = cap[2].parse::<u64>().unwrap_or(1).saturating_sub(1);
                let uri = make_uri(file, saved_uri, root_path);
                let location = cap
                    .get(3)
                    .map(|m| format!(" in {}", m.as_str()))
                    .unwrap_or_default();

                let diag = json!({
                    "range": {
                        "start": { "line": line_num, "character": 0 },
                        "end": { "line": line_num, "character": 1 }
                    },
                    "severity": sev,
                    "source": "runtime",
                    "message": format!("{exc_msg}{location}")
                });
                sev = 2;
                out.entry(uri).or_default().push(diag);
                continue;
            }
        } else {
            sev = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_traceback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("b_tree.py");
        std::fs::write(&file_path, "# python test file").unwrap();

        let log = format!(
            r#"Traceback (most recent call last):
  File "{file}", line 66, in <module>
    tree.b_tree_insert(i)
  File "{file}", line 63, in b_tree_insert
    self.b_tree_insert_nonfull(r,k)
  File "{file}", line 52, in b_tree_insert_nonfull
    if x.c[i].n==2*self.t-1:
       ~~~^^^
IndexError: list index out of range
"#,
            file = file_path.display()
        );

        let mut out = HashMap::new();
        parse_traceback(
            &log,
            "file:///tmp/dummy.py",
            temp_dir.path().to_str().unwrap(),
            &mut out,
        );

        let uri = format!("file://{}", file_path.canonicalize().unwrap().display());
        let diags = out
            .get(&uri)
            .expect("diagnostic missing for traceback file");

        assert_eq!(diags.len(), 1);
        let diag = &diags[0];
        assert_eq!(diag["range"]["start"]["line"], json!(51));
        assert_eq!(diag["range"]["start"]["character"], json!(0));
        assert_eq!(
            diag["message"],
            json!("IndexError: list index out of range in b_tree_insert_nonfull")
        );
    }
}
