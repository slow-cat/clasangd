# Usage
## build
```bash
  cargo build --release
```
make it accessible
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

# Issue
~~It can handle only initial Asan log.
Maybe, lsp cannot  receive same comment in short span.~~ solved
