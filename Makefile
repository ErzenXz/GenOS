.PHONY: build run test bench test-sdk clean

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
