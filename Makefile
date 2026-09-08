.PHONY: build run test bench test-sdk test-release build-test build-release clean

build:
	cargo xtask build

run:
	cargo xtask run

test:
	cargo xtask test

bench:
	cargo xtask bench

test-sdk:
	cargo xtask test-sdk

clean:
	cargo xtask clean

build-test:
	cargo xtask build-test

build-release:
	cargo xtask build-release

test-release:
	cargo xtask test-release
