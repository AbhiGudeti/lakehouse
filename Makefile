.PHONY: fmt clippy test check

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all

check: fmt clippy test
