
default:
    just --list


build:
    RUSTFLAGS="-Ctarget-feature=+multivalue -Clink-args=-zstack-size=64000" cargo build --release --package prototype --target wasm32-unknown-unknown
    cargo build --package othismo


package: build
    # Building & Placing Artifacts in ./playground 
    rm -r ./playground/*
    mkdir -p ./playground
    cp ./target/debug/othismo ./playground
    cp ./target/wasm32-unknown-unknown/release/prototype.wasm ./playground

[working-directory: 'playground']
dev: package
    # Building dev image 'image'
    ./othismo new-image image
    ./othismo image import-module ./prototype.wasm
    ./othismo image instantiate-instance prototype instance
    # test
    ./othismo image list-objects

[working-directory: 'playground']
hello:
    ./othismo image send-message /


# --- glue ---

# Build the glue language server
lsp:
    cargo build --package lsp

# Build the VS Code extension (run once after checkout, then on TS changes)
[working-directory: 'glue/vscode']
extension:
    npm install
    npm run compile

# Everything the editor integration needs
editor: lsp extension
    @echo "Open glue/vscode in VS Code and press F5 to launch the extension host."
