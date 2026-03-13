.DEFAULT_GOAL := build
.PHONY: build devel clean check fix test

build:
	cargo build --release --workspace

devel:
	cargo build -v --workspace

clean:
	cargo clean

check:
	cargo clippy --no-deps --all -- -Dwarnings -Aunused-variables -Adead-code

fix:
	cargo clippy --fix --allow-dirty --allow-staged --all

test:
	cargo test --workspace
