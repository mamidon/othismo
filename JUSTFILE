
default:
    just --list


build-code:
    cargo build --package prototype --target wasm32-unknown-unknown
    cargo build --package othismo


package: build-code
    # Building & Placing Artifacts in ./playground 
    rm -r ./playground/*
    mkdir -p ./playground
    cp ./target/debug/othismo ./playground
    cp ./target/wasm32-unknown-unknown/debug/prototype.wasm ./playground

[working-directory: 'playground']
dev: package
    # Building dev image 'image'
    ./othismo new-image image
    ./othismo image import-module ./prototype.wasm 
    ./othismo image instantiate-instance prototype instance
    # test
    ./othismo image list-objects