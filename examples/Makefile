# Makefile for building Go (fat) and TinyGo (lean) WASI binaries

# Source file
SRC = hello.go

# Output names
TINY = hello-tinygo.wasm
FAT  = hello-go.wasm
JS   = hello-js.wasm
GR   = hello-grain.wasm

# Default target: build both
.PHONY: build build-tinygo build-go build-js build-grain clean fix

build: build-tinygo build-go build-js build-grain fix

build-tinygo:
	@echo "==> Building TinyGo WASI module"
	tinygo build -o $(TINY) -target=wasi $(SRC)

build-go:
	@echo "==> Building Go (fat) WASI module"
	GOOS=wasip1 GOARCH=wasm CGO_ENABLED=0 go build -o $(FAT) $(SRC)

build-js:
	@echo "==> Building JavaScript WASI module"
	javy build hello.js -o $(JS)

build-grain:
	@echo "==> Building Grain WASI module"
	grain hello.gr -o $(GR)
	rm -rf target
	rm hello.gro

fix:
	@echo "==> Fixing attributes"
	chmod -x *wasm

clean:
	@echo "==> Cleaning up"
	rm -f $(TINY) $(FAT) $(JS) $(GR)
	rm -rf target
