RUST_TOOLCHAIN := 1.81.0
OPTIMIZER_IMAGE := cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e
CARGO_AUDIT_VERSION := 0.22.2
CARGO_DENY_VERSION := 0.20.2

.PHONY: ci test clippy security optimize reproducible test-keeper local-setup local-test local-grid local-e2e local-soak local-all local-stop local-reset

ci: test clippy

test:
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path rebalancer-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path rebalancer-system/Cargo.toml --all-targets
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path grid-contract-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path grid-contract-system/Cargo.toml --all-targets
	python3 -m unittest discover -s rebalancer-system/examples/keeper -p 'test_*.py'

clippy:
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings

security:
	cargo +stable install cargo-audit --version $(CARGO_AUDIT_VERSION) --locked
	cargo +stable install cargo-deny --version $(CARGO_DENY_VERSION) --locked
	cargo +stable audit --file rebalancer-system/Cargo.lock
	cargo +stable audit --file grid-contract-system/Cargo.lock
	cargo +stable deny --manifest-path rebalancer-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path grid-contract-system/Cargo.toml --all-features check

optimize:
	docker run --rm -v "$(CURDIR)/rebalancer-system:/code" --mount type=volume,source=cl8y_bot_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)
	docker run --rm -v "$(CURDIR)/grid-contract-system:/code" --mount type=volume,source=cl8y_grid_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)

reproducible:
	mkdir -p artifacts/release
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh rebalancer-system artifacts/release
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh grid-contract-system artifacts/release

test-keeper:
	python3 -m unittest discover -s rebalancer-system/examples/keeper -p 'test_*.py'

local-setup:
	./test-area/setup.sh

local-test:
	./test-area/integration-test.sh

local-grid:
	./test-area/grid-integration-test.sh

local-e2e:
	./test-area/run-e2e.sh

local-soak:
	./test-area/run-soak.sh

local-all:
	./test-area/run-all.sh

local-stop:
	./test-area/stop.sh

local-reset:
	./test-area/reset.sh
