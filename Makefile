.PHONY: test clippy optimize test-keeper local-setup local-test local-grid local-e2e local-soak local-all local-stop local-reset

test:
	cargo test --manifest-path rebalancer-system/Cargo.toml
	cargo test --manifest-path grid-contract-system/Cargo.toml

clippy:
	cargo clippy --manifest-path rebalancer-system/Cargo.toml --all-targets -- -D warnings
	cargo clippy --manifest-path grid-contract-system/Cargo.toml --all-targets -- -D warnings

optimize:
	docker run --rm -v "$(CURDIR)/rebalancer-system:/code" --mount type=volume,source=cl8y_bot_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry cosmwasm/workspace-optimizer:0.16.1
	docker run --rm -v "$(CURDIR)/grid-contract-system:/code" --mount type=volume,source=cl8y_grid_target_cache,target=/code/target --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry cosmwasm/workspace-optimizer:0.16.1

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
