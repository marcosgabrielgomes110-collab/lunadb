# LunaDB - development shortcuts
#
# Quick usage:
#   make            # shows this help
#   make build      # compiles in debug mode
#   make test       # runs Rust + Python SDK tests
#   make serve      # starts the API (cargo run -- serve)
#
# Useful variables:
#   make test-py PYTHON=$(VENV)/bin/python   # uses the venv from make venv
#   make migrate TO=json FORCE=1             # lunadb migrate --to json --force
#   make serve ARGS="--config ~/.lunadb/config.toml"

.DEFAULT_GOAL := help

CARGO   ?= cargo
PYTHON  ?= python3
VENV    ?= .venv
TO      ?= json
ARGS    ?=
FORCE   ?=
PREFIX  ?= /usr/local
BINDIR  ?= $(PREFIX)/bin
DESTDIR ?=

.PHONY: help build release check fmt fmt-check clippy lint test test-rust test-py \
        venv run serve init shell migrate apikey install uninstall clean distclean \
        luna-setup

help:
	@echo "LunaDB - available targets:"
	@echo ""
	@echo "  build       Compiles the binary (debug) to target/debug/lunadb"
	@echo "  release     Compiles optimized (target/release/lunadb)"
	@echo "  check       cargo check on all targets"
	@echo "  fmt         Formats the code (cargo fmt)"
	@echo "  fmt-check   Checks formatting without changing"
	@echo "  clippy      Lint with clippy (-D warnings)"
	@echo "  lint        fmt-check + clippy"
	@echo "  test        Rust + Python SDK tests"
	@echo "  test-rust   Only cargo test --all"
	@echo "  test-py     Only the SDK pytest (depends on build)"
	@echo "  venv        Creates .venv with pytest and the SDK installed (-e)"
	@echo "  run         Compiles and runs (default: serve)"
	@echo "  serve       Starts the API"
	@echo "  init        Initializes ~/.lunadb (config, roles, root, master key)"
	@echo "  shell       Opens the interactive shell"
	@echo "  migrate     Converts tables (TO=json|msgpack, FORCE=1)"
	@echo "  apikey      API key commands (ARGS=\"list\" etc.)"
	@echo "  luna-setup  Interactive config.toml generator (Enter = default)"
	@echo "  install     Installs ONLY the binary to PREFIX/bin (default /usr/local)"
	@echo "  uninstall   Removes the installed binary"
	@echo "  clean       Removes Cargo artifacts"
	@echo "  distclean   clean + removes .venv and Python caches"
	@echo ""
	@echo "Examples:"
	@echo "  make build && make test"
	@echo "  make venv && make test-py PYTHON=$(VENV)/bin/python"
	@echo "  make migrate TO=json FORCE=1"
	@echo "  make install PREFIX=\$$HOME/.local        # without sudo"
	@echo "  sudo make install PREFIX=/usr/local       # system-wide"
	@echo "  make install DESTDIR=/tmp/pkgroot PREFIX=/usr   # packaging"

build:
	$(CARGO) build

release:
	$(CARGO) build --release

check:
	$(CARGO) check --all-targets

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

lint: fmt-check clippy

test: test-rust test-py

test-rust:
	$(CARGO) test --all

# Requires pytest (install with: make venv, or pip install pytest).
# Optional symbol: PYTHON=$(VENV)/bin/python make test-py
test-py: build
	PYTHONPATH=lunapy/src $(PYTHON) -m pytest lunapy/tests -q

venv:
	$(PYTHON) -m venv $(VENV)
	$(VENV)/bin/pip install --upgrade pip
	$(VENV)/bin/pip install -e "lunapy[dev]"

run:
	$(CARGO) run -- $(ARGS)

serve:
	$(CARGO) run -- serve $(ARGS)

init:
	$(CARGO) run -- init $(ARGS)

shell:
	$(CARGO) run -- shell $(ARGS)

migrate:
	$(CARGO) run -- migrate --to $(TO) $(if $(FORCE),--force,)

apikey:
	$(CARGO) run -- apikey $(ARGS)

# Interactive config.toml generator. ARGS is forwarded to the script, e.g.
#   make luna-setup ARGS="--config /tmp/luna/config.toml --force"
luna-setup:
	sh scripts/luna-setup.sh $(ARGS)

# Installs only the compiled binary (release) to $(DESTDIR)$(BINDIR).
install: release
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 0755 target/release/lunadb "$(DESTDIR)$(BINDIR)/lunadb"
	@echo "installed: $(DESTDIR)$(BINDIR)/lunadb"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/lunadb"
	@echo "removed: $(DESTDIR)$(BINDIR)/lunadb"

clean:
	$(CARGO) clean

distclean: clean
	rm -rf $(VENV)
	find . -type d -name '__pycache__' -prune -exec rm -rf {} +
	find . -type d -name '*.egg-info' -prune -exec rm -rf {} +
	rm -rf .pytest_cache
