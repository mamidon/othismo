# Object modules: every scripts/*.ts next to an image compiles to scripts/*.wasm.
SCRIPTS := $(wildcard examples/*/scripts/*.ts)
WASM    := $(SCRIPTS:.ts=.wasm)
# Any library or message change rebuilds every module; there are few of them.
AS_DEPS := as/index.ts as/env.ts $(wildcard examples/*/*.ts) node_modules/.installed

.PHONY: wasm hello greeter test clean

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

test: wasm
	cargo test

clean:
	rm -f $(WASM)
