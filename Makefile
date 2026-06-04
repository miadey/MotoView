# MotoView — convenience targets (no Node, no npm).
#
#   make client    build the Rust->WASM client and embed it in the runtime
#   make compiler  build the `motoview` compiler (Rust)
#   make example   compile + deploy the counter to the local replica (needs dfx)
#   make site      build the documentation site into site/docs (needs python3)
#   make check     type-check the Motoko runtime with moc
#   make all       client + compiler

MOC := $(HOME)/.cache/dfinity/versions/0.28.0/moc
BASE := $(HOME)/.cache/dfinity/versions/0.28.0/base

.PHONY: all client compiler example site check clean

all: client compiler

client:
	./tools/build-client.sh

compiler:
	cargo build --release --manifest-path compiler/Cargo.toml
	@echo "built: compiler/target/release/motoview"

example: compiler
	compiler/target/release/motoview build examples/counter --name counter
	cd examples/counter && dfx deploy

site:
	cd site && python3 build.py

check:
	./tools/check.sh runtime/src/App.mo runtime/src/lib.mo

test:
	cargo test --manifest-path compiler/Cargo.toml

clean:
	cargo clean --manifest-path compiler/Cargo.toml || true
	cargo clean --manifest-path client/Cargo.toml || true
	rm -rf site/docs/*.html site/docs/assets
