.PHONY: all build dev install status test fmt lint clean

all: build

build:
	cargo build --release

dev:
	@./scripts/install.sh dev

install:
	@./scripts/install.sh release

status:
	@./scripts/install.sh status

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets --all-features

install-mcp:
	@cargo run --offline -- install-mcp

clean:
	cargo clean
