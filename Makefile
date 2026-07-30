.PHONY: test test-keeper local-setup local-test local-grid local-e2e local-soak local-all local-stop local-reset

test:
	cargo test --workspace

test-keeper:
	python3 -m unittest discover -s examples/keeper -p 'test_*.py'

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
