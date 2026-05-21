BIN      := regolo
INSTALL  := /usr/local/bin

.PHONY: all build release install uninstall clean test fmt lint

all: build

build:
	cargo build

release:
	cargo build --release

install: release
	install -m 755 target/release/$(BIN) $(INSTALL)/$(BIN)

uninstall:
	rm -f $(INSTALL)/$(BIN)

clean:
	cargo clean

test:
	cargo test

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings
