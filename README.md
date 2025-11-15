# Overview
This program can merge diagnoses from clangd and logs at build or runtime and send them to your editor, as if from a single LSP.

# Usage
## Build
```bash
  cargo build --release
```
make it available
```bash
  ln -s "${PWD}/target/release/clasangd" ~/.local/bin
```
## lsp-setup
* for helix editor
    see example/language.toml
## Options
```
Options:
  -n, --name <NAME>  log file basename,or preferred log location like /tmp/clasangd [default: /tmp/clasangd]
  -v, --verbose      eprint lsp-log
  -h, --help         Print help
  -V, --version      Print version
```
# ToDo
 - handle set of files  like header and source files
 - wrap not only clangd, like jdtls
 - splite read_lsp into reading client_stdout part and throwing at lsp_stdin
# Issue
~~It can handle only initial Asan log.
Maybe, lsp cannot  receive same comment in short span.~~ solved

