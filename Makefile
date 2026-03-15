# Coral Language — Build & Distribution Targets

.PHONY: build release runtime test test-runtime check clean install dist help

# Default target
help:
	@echo "Coral Language Build System"
	@echo ""
	@echo "  make build        — Build compiler and runtime (debug)"
	@echo "  make release      — Build compiler and runtime (release)"
	@echo "  make runtime      — Build runtime library only"
	@echo "  make test         — Run all tests"
	@echo "  make test-runtime — Run runtime tests only"
	@echo "  make check        — Quick build check (no tests)"
	@echo "  make clean        — Remove build artifacts"
	@echo "  make install      — Install coralc to ~/.cargo/bin"
	@echo "  make dist         — Build release binary for distribution"
	@echo ""

# ─── Build ────────────────────────────────────────────────────
build:
	cargo build
	cargo build -p runtime

release:
	cargo build --release
	cargo build -p runtime --release

runtime:
	cargo build -p runtime

check:
	cargo check

# ─── Test ─────────────────────────────────────────────────────
test:
	cargo test --all
	cargo test -p runtime --lib

test-runtime:
	cargo test -p runtime --lib

# ─── Install & Distribute ────────────────────────────────────
install: release
	@mkdir -p ~/.cargo/bin
	cp target/release/coralc ~/.cargo/bin/coralc
	@echo "Installed coralc to ~/.cargo/bin/coralc"
	@echo "Ensure ~/.cargo/bin is in your PATH"

DIST_DIR := dist
PLATFORM := $(shell uname -s | tr '[:upper:]' '[:lower:]')-$(shell uname -m)

dist: release
	@mkdir -p $(DIST_DIR)
	cp target/release/coralc $(DIST_DIR)/coralc
	@# Bundle runtime library
	@if [ -f target/release/libruntime.so ]; then \
		cp target/release/libruntime.so $(DIST_DIR)/; \
	elif [ -f target/release/libruntime.dylib ]; then \
		cp target/release/libruntime.dylib $(DIST_DIR)/; \
	fi
	@# Bundle standard library
	cp -r std $(DIST_DIR)/std
	@# Create tarball
	tar -czf $(DIST_DIR)/coral-$(PLATFORM).tar.gz -C $(DIST_DIR) coralc libruntime.* std/
	@echo "Distribution package: $(DIST_DIR)/coral-$(PLATFORM).tar.gz"

# ─── Clean ────────────────────────────────────────────────────
clean:
	cargo clean
	rm -rf $(DIST_DIR)
