# Overview
This program can send compile error and runtime error to editors by LSP.
clang python java are allowed.
# Usage
## Build
```bash
  cargo install --path . --root ~/.cargo 
```
## Options
```
Options:
  -n, --name <NAME>  log file basename,or preferred log location like /tmp/clasangd [default: /tmp/clasangd]
  -v, --verbose      eprint lsp-log
  -h, --help         Print help
  -V, --version      Print version
```
# Todo
- python's underline
