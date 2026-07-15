.PHONY: build run test clean

build:
	cargo xtask build

run:
	cargo xtask run

test:
	cargo xtask test

clean:
	cargo xtask clean
