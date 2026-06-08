VERSION := $(shell jq -r '.version' package.json)
TAG := v$(VERSION)

.PHONY: release
release:
	@if [ -z "$(VERSION)" ] || [ "$(VERSION)" = "null" ]; then \
		echo "Error: could not read version from package.json"; exit 1; fi
	@if git rev-parse "$(TAG)" >/dev/null 2>&1; then \
		echo "Error: tag $(TAG) already exists"; exit 1; fi
	@CURRENT_CARGO=$$(grep '^version = ' src-tauri/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	if [ "$$CURRENT_CARGO" != "$(VERSION)" ]; then \
		sed -i "s/^version = \"$$CURRENT_CARGO\"/version = \"$(VERSION)\"/" src-tauri/Cargo.toml; \
		echo "Updated src-tauri/Cargo.toml to $(VERSION)"; fi
	@CURRENT_TAURI=$$(jq -r '.version' src-tauri/tauri.conf.json); \
	if [ "$$CURRENT_TAURI" != "$(VERSION)" ]; then \
		sed -i 's/"version": "'"$$CURRENT_TAURI"'"/"version": "'"$(VERSION)"'"/' src-tauri/tauri.conf.json; \
		echo "Updated src-tauri/tauri.conf.json to $(VERSION)"; fi
	@if git diff --quiet && git diff --cached --quiet && test -z "$$(git status --porcelain)"; then \
		echo "Nothing to commit"; exit 1; fi
	git add -A
	git commit -m "release $(TAG)"
	git tag "$(TAG)"
	git push origin "$(TAG)"
	git push
	@echo "Released $(TAG)"
