# Quick start:
#   make test          # pure-Rust unit+integration, default features
#   make check         # the full gate: fmt-check + clippy + test
#   make help          # list every target

# Recipes use `source` (a bashism) to load ~/.cargo/env. make defaults to
# /bin/sh, which is dash on Ubuntu (CI) where `source` doesn't exist — force
# bash so recipes behave the same on macOS and Linux.
SHELL := /bin/bash

# cargo isn't on PATH in this environment; source it in each recipe.
CARGO := source "$$HOME/.cargo/env" && cargo

PROPSIM_MANIFEST := Cargo.toml

## help: list every target with its description
.PHONY: help
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed -e 's/## //' | awk -F': ' '{ printf "  \033[1m%-18s\033[0m %s\n", $$1, $$2 }'

## test: alias for test-propsim (default-feature unit + integration tests)
.PHONY: test
test: test-propsim

## test-unit: library unit tests only (src/ #[test]s), both workspaces, default features
.PHONY: test-unit
test-unit:
	$(CARGO) test --manifest-path $(PROPSIM_MANIFEST) --lib

## test-propsim: all default-feature tests
.PHONY: test-propsim
test-propsim:
	$(CARGO) test --manifest-path $(PROPSIM_MANIFEST)

## test-all-features: every feature closure, eg propsim's serde closure (still offline / pure-Rust)
.PHONY: test-all-features
test-all-features:
	$(CARGO) test --manifest-path $(PROPSIM_MANIFEST) --all-features

## fmt: format the whole workspace in place
.PHONY: fmt
fmt:
	$(CARGO) fmt --all

## fmt-check: assert the workspace is formatted (used by `check`)
.PHONY: fmt-check
fmt-check:
	$(CARGO) fmt --all -- --check

## clippy: lint all targets, warnings denied
.PHONY: clippy
clippy:
	$(CARGO) clippy --manifest-path $(PROPSIM_MANIFEST) --all-targets -- -D warnings

## check: the full gate — fmt-check + clippy + test
.PHONY: check
check: fmt-check clippy test-propsim
