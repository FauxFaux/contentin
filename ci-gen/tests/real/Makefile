all: smokes

BIN=../../../target/debug

cargo:
	cargo build --all

smokes: cargo $(patsubst %,.%.yml,$(wildcard *))

.%.yml: %
	$(BIN)/ci-gen -h capnp $^ | $(BIN)/ci-dump > $@
