RUST_TOOLCHAIN := 1.81.0
OPTIMIZER_IMAGE := cosmwasm/workspace-optimizer:0.16.1@sha256:b9c92b2900b7ebaab3499203615c1b8589592bc557355ed3432e48851ffde69e

.PHONY: ci test clippy security optimize reproducible fee-wasm test-keeper local-setup local-test local-grid local-e2e local-fee-e2e local-soak local-all local-stop local-reset

TEST_CANONICAL_FEE_COLLECTOR := terra1x46rqay4d3cssq8gxxvqz8xt6nwlz4td20k38v
TEST_CANONICAL_FEE_REGISTRY := terra1mpys68uwajmcjn6lwctan39v5sf4yk3agxp8pns02kqfnym3racsunwpqy
TEST_CANONICAL_SWAP_PROXY := terra16j5u6ey7a84g40sr3gd94nzg5w5fm45046k9s2347qhfpwm5fr6sem3lr2
MAINNET_TEST_ENV := CL8Y_CANONICAL_FEE_COLLECTOR=$(TEST_CANONICAL_FEE_COLLECTOR) CL8Y_CANONICAL_FEE_REGISTRY=$(TEST_CANONICAL_FEE_REGISTRY) CL8Y_CANONICAL_SWAP_PROXY=$(TEST_CANONICAL_SWAP_PROXY)

ci: test clippy

test:
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path rebalancer-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path rebalancer-system/Cargo.toml --all-targets
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path rebalancer-system/Cargo.toml --lib --features mainnet
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path market-grid-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path market-grid-system/Cargo.toml --all-targets
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path market-grid-system/Cargo.toml --lib --features mainnet
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path limit-grid-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path limit-grid-system/Cargo.toml --all-targets
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path limit-grid-system/Cargo.toml --lib --features mainnet
	cargo +$(RUST_TOOLCHAIN) fmt --manifest-path fee-system/Cargo.toml --all -- --check
	cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path fee-system/Cargo.toml --all-targets
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) test --locked --manifest-path fee-system/Cargo.toml --all-targets --features mainnet
	python3 -m unittest discover -s rebalancer-system/examples/keeper -p 'test_*.py'
	PYTHONPATH=grid-operator-system/services/grid-operator python3 -m unittest discover -s grid-operator-system/services/grid-operator/tests -p 'test_*.py'

clippy:
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path rebalancer-system/Cargo.toml --all-targets --features mainnet -- -D warnings
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path market-grid-system/Cargo.toml --all-targets --features mainnet -- -D warnings
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path limit-grid-system/Cargo.toml --all-targets --features mainnet -- -D warnings
	$(MAINNET_TEST_ENV) cargo +$(RUST_TOOLCHAIN) clippy --locked --manifest-path fee-system/Cargo.toml --all-targets --features mainnet -- -D warnings

security:
	.github/scripts/tests/release-security-policy-test.sh
	.github/scripts/install-security-tools.sh
	.github/scripts/release-security-policy.sh audit
	cargo +stable deny --manifest-path rebalancer-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path market-grid-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path limit-grid-system/Cargo.toml --all-features check
	cargo +stable deny --manifest-path fee-system/Cargo.toml --all-features check

optimize:
	docker run --rm -v "$(CURDIR)/rebalancer-system:/code" --mount type=volume,source=cl8y_bot_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)
	docker run --rm -v "$(CURDIR)/market-grid-system:/code" --mount type=volume,source=cl8y_grid_swap_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)
	docker run --rm -v "$(CURDIR)/limit-grid-system:/code" --mount type=volume,source=cl8y_grid_limit_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry $(OPTIMIZER_IMAGE)

reproducible:
	mkdir -p artifacts/release
	.github/scripts/release-security-policy.sh inventory
	@for workspace in `./.github/scripts/release-security-policy.sh workspaces`; do \
		OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh $$workspace artifacts/release default; \
		OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh $$workspace artifacts/release mainnet; \
	done

fee-wasm:
	mkdir -p artifacts/fee-system
ifeq ($(MAINNET),1)
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh fee-system artifacts/fee-system mainnet
else
	OPTIMIZER_IMAGE=$(OPTIMIZER_IMAGE) .github/scripts/reproducible-build.sh fee-system artifacts/fee-system default
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

local-fee-e2e:
	./test-area/run-fee-e2e.sh

local-soak:
	./test-area/run-soak.sh

local-all:
	./test-area/run-all.sh

local-stop:
	./test-area/stop.sh

local-reset:
	./test-area/reset.sh
