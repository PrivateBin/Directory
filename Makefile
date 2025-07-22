.PHONY: all release test build pack image run check lint coverage clean help

NAME = directory
IMAGE = privatebin/$(NAME)
PORT = 8000
DATABASE = var/directory.sqlite
ROCKET_DATABASES = "{directory={url=\"$(DATABASE)\"}}"
GEOIP_MMDB = var/geoip-country.mmdb

all: test build image run check clean ## Equivalent to "make test build image run check clean" (default).

release: test build pack license image run check clean ## Equivalent to "make test build pack image run check clean".

test: .cargo/registry $(DATABASE) ## Build and run the unit tests.
	rm -f $(DATABASE)-*
	git checkout $(DATABASE)
	GEOIP_MMDB="$(GEOIP_MMDB)" \
	ROCKET_DATABASES=$(ROCKET_DATABASES) \
	RUST_BACKTRACE=1 \
	cargo test --release -- --test-threads=1
	git checkout $(DATABASE)

build: .cargo/registry ## Build the binary for release.
	rm -f $(DATABASE)-*
	git checkout $(DATABASE)
	cargo build --release

pack: ## Compresses the binary to reduce it's size, only intended for the release.
	upx --ultra-brute target/x86_64-unknown-linux-musl/release/directory

license: ## Generates the LICENSE.md file
	cargo about init
	cargo about generate --fail about.hbs > LICENSE.md

LICENSE.md: license

image: ## Build the container image.
	docker build --build-arg PORT=$(PORT) -t $(IMAGE) --load .

run: ## Run a container from the image.
	docker run -d --rm --init --name $(NAME) -p=$(PORT):$(PORT) \
		--read-only -v "$(CURDIR)/var":/var -u=$$(id -u):$$(id -g) $(IMAGE)

check: ## Launch tests to verify that the service works as expected, requires a running container.
	@sleep 2
	nc -z localhost $(PORT)
	curl -s http://localhost:$(PORT)/ | grep "Welcome!"
	curl -s http://localhost:$(PORT)/about | grep "About"
	curl -s http://localhost:$(PORT)/add | grep "Add instance"
	docker exec -t -e CRON=POLL directory directory | grep "cleaned up checks stored before"
	@echo "Checks: \033[92mOK\033[0m"

.cargo/registry:
	mkdir -p .cargo/registry

lint: ## Run fmt & clippy on the code to come up with improvements.
	cargo fmt
	cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
	git checkout $(DATABASE)

coverage: ## Run tarpaulin on the code to report on the tests code coverage.
	git checkout $(DATABASE)
	GEOIP_MMDB="$(GEOIP_MMDB)" \
	ROCKET_DATABASES=$(ROCKET_DATABASES) \
	cargo tarpaulin --release -o Html -- --test-threads=1
	git checkout $(DATABASE)

clean: $(DATABASE) ## Stops and removes the running container.
	rm -f $(DATABASE)-*
	git checkout $(DATABASE)
	docker ps -q --filter "name=$(NAME)" | grep -q . && \
	docker stop $(NAME) || true

help: ## Displays these usage instructions.
	@echo "Usage: make <target(s)>"
	@echo
	@echo "Specify one or multiple of the following targets and they will be processed in the given order:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "%-16s%s\n", $$1, $$2}' $(MAKEFILE_LIST)
