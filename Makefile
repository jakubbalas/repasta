# Repasta developer tasks. Run `make help` for the list.

CARGO ?= cargo
CARGO_FLAGS := --locked --offline
DESKTOP := --features desktop --bin repasta-desktop

.DEFAULT_GOAL := help
.PHONY: help build run app install uninstall fmt check clean

help: ## List available targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  make %-10s %s\n", $$1, $$2}'

build: ## Build the desktop preview (release)
	$(CARGO) build $(CARGO_FLAGS) --release $(DESKTOP)

run: ## Run the desktop preview from source
	$(CARGO) run $(CARGO_FLAGS) --release $(DESKTOP)

app: ## Build target/Repasta.app (macOS)
	./scripts/bundle-macos.sh

install: ## Rebuild Repasta.app and install it to ~/Applications (macOS)
	./scripts/bundle-macos.sh --install

uninstall: ## Remove Repasta.app from ~/Applications (macOS)
	rm -rf "$(HOME)/Applications/Repasta.app"

fmt: ## Format the code
	$(CARGO) fmt --all

check: ## Run the hand-off checks: fmt, Clippy, and tests
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets -- -D warnings
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets --features desktop -- -D warnings
	$(CARGO) test $(CARGO_FLAGS)
	$(CARGO) test $(CARGO_FLAGS) --features desktop

clean: ## Remove build output, including target/Repasta.app
	$(CARGO) clean
