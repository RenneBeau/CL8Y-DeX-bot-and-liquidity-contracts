RUST_TOOLCHAIN := 1.81.0
OPTIMIZER_IMAGE := cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e

.PHONY: ci test clippy security optimize reproducible fee-wasm test-keeper local-setup local-test local-grid local-e2e local-soak local-all local-stop local-reset

ci: test clippy

test:
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path rebalancer-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path rebalancer-system/Cargo.toml --all-targets
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path market-grid-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path market-grid-system/Cargo.toml --all-targets
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path limit-grid-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path limit-grid-system/Cargo.toml --all-targets
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path fee-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path fee-system/Cargo.toml --all-targets
	python3 -m unittest discover -s rebalancer-system/examples/keeper -p 'test_*.py'
	python3 -m unittest discover -s grid-operator-system/services/grid-operator/tests -p 'test_*.py'

clippy:
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path market-grid-system/Cargo.toml --all-targets -- -D warnings
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path limit-grid-system/Cargo.toml --all-targets -- -D warnings
	cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path fee-system/Cargo.toml --all-targets --features mainnet -- -D warnings

security:
	.github/scripts/install-security-tools.sh
	cargo +stable audit --file rebalancer-system/Cargo.lock --ignore RUSTSEC-2024-0344
	cargo +stable audit --file market-grid-system/Cargo.lock --ignore RUSTSEC-2024-0344
	cargo +stable audit --file limit-grid-system/Cargo.lock --ignore RUSTSEC-2024-0344
	cargo +stable deny --manifest-path rebalancer-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path market-grid-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path limit-grid-system/Cargo.toml --all-features check

optimize:
	docker run --rm -v "$(CURDIR)/rebalancer-system:/code" --mount type=volume,source=cl8y_bot_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)
	docker run --rm -v "$(CURDIR)/market-grid-system:/code" --mount type=volume,source=cl8y_grid_swap_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)
	docker run --rm -v "$(CURDIR)/limit-grid-system:/code" --mount type=volume,source=cl8y_grid_limit_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)

reproducible:
	mkdir -p artifacts/release
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh rebalancer-system artifacts/release
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh market-grid-system artifacts/release
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh limit-grid-system artifacts/release

## Build the fee-system wasm artifacts. Default (no `mainnet` feature): local
## test-a-net / E2E binaries with dummy addresses. Mainnet releases MUST be built
## with `make fee-wasm MAINNET=1` (or `--features mainnet`) so the canonical CL8Y
## and CMM treasury addresses are pinned inside the binary and re-pointer rejections.
fee-wasm:
	mkdir -p artifacts/fee-system
ifeq ($(MAINNET),1)
	cargo +$(RUST_TOOLCHAIN) build --locked --manifest-path fee-system/Cargo.toml --release --target wasm32-unknown-unknown --features mainnet
	cp fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_registry.wasm artifacts/fee-system/
	cp fee-system/target/wasm32-unknown-unknown/release/cl8y_fee_collector.wasm artifacts/fee-system/
else
	cargo +$(RUST_TOOLCHAIN) build --locked --manifest-path fee-system/Cargo.toml --release --target wasm32-unknown-unknown
endif

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
