# Object modules: every scripts/*.ts next to an image compiles to scripts/*.wasm.
SCRIPTS := $(wildcard examples/*/scripts/*.ts)
WASM    := $(SCRIPTS:.ts=.wasm)
# Any library or message change rebuilds every module; there are few of them.
AS_DEPS := as/index.ts as/ns.ts as/env.ts $(wildcard examples/*/*.ts) node_modules/.installed

.PHONY: wasm hello greeter static static-built site test clean

wasm: $(WASM)

%.wasm: %.ts $(AS_DEPS)
	npx asc $< -o $@ --optimize

node_modules/.installed: package.json as/package.json
	npm install
	touch $@

hello: wasm
	cargo run -- examples/hello/hello.img

greeter: wasm
	cargo run -- examples/greeter/greeter.img

static: wasm
	cargo run -- examples/static/static.img

# The static example sealed into one file, modules and site included. It is
# rebuilt when othismo changes too, since the file format may have.
examples/static/static.rti: examples/static/static.img $(WASM) $(shell find examples/static/public -type f) $(wildcard src/*.rs) Cargo.toml
	cargo run -- build $< -o $@

static-built: examples/static/static.rti
	cargo run -- $<

# Re-renders docs/*.md into the static example's site.
site: node_modules/.installed
	node examples/static/site.mjs

test: wasm
	cargo test

clean:
	rm -f $(WASM) examples/static/static.rti
