.DEFAULT_GOAL := build
.PHONY:build devel clean check fix

build:
	cargo build --release --workspace
	$(call move_bin,release,)

devel:
	cargo build -v --workspace
	$(call move_bin,debug,)

clean:
	cargo clean

check:
	cargo clippy --no-deps --all -- -Dwarnings -Aunused-variables -Adead-code

fix:
	cargo clippy --fix --allow-dirty --allow-staged --all
