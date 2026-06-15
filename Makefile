.PHONY: release benchly clean

release: benchly

benchly:
	cd src/benchly && cargo build --release

clean:
	cd src/benchly && cargo clean