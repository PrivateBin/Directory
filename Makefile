.PHONY: all release test build pack image run check clean help

NAME = directory
IMAGE = privatebin/$(NAME)
PORT = 8000
BUILD_IMAGE = ekidd/rust-musl-builder:nightly-2020-03-12-sqlite
GEOIP_MMDB = var/geoip-country.mmdb
DATABASE = var/directory.sqlite
ROCKET_DATABASES = "{directory={url=\"$(DATABASE)\"}}"
ROCKET_CRON_KEY = $(shell openssl rand -base64 32)

all: test build image run check clean ## Equivalent to "make test build image run check clean" (default).

release: test build pack license image run check clean ## Equivalent to "make test build pack image run check clean".

test: .cargo/registry var/directory.sqlite ## Build and run the unit tests.
	git checkout $(DATABASE)
	docker run --rm -t --init \
		-e GEOIP_MMDB="$(GEOIP_MMDB)" \
		-e ROCKET_DATABASES=$(ROCKET_DATABASES) \
		-e ROCKET_CRON_KEY=$(ROCKET_CRON_KEY) \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		$(BUILD_IMAGE) \
		cargo test --release # -- --nocapture

build: .cargo/registry ## Build the binary for release.
	docker run --rm -t --init \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		$(BUILD_IMAGE) \
		cargo build --release

pack: ## Strips and compresses the binary to reduce it's size, only intended for the release.
	strip target/x86_64-unknown-linux-musl/release/directory
	upx --ultra-brute target/x86_64-unknown-linux-musl/release/directory

license: ## Generates the LICENSE.md file
	docker run --rm -t --init \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		$(BUILD_IMAGE) \
		sh -c "cargo install cargo-about && cargo about init && cargo about generate about.hbs > /home/rust/src/LICENSE.md"

image: ## Build the container image.
	docker build --build-arg PORT=$(PORT) \
		--build-arg GEOIP_MMDB="/$(GEOIP_MMDB)" \
		--build-arg ROCKET_DATABASES='{directory={url="/'$(DATABASE)'"}}' \
		-t $(IMAGE) .

run: ## Run a container from the image.
	docker run -d --init --name $(NAME) -p=$(PORT):$(PORT) \
		-e ROCKET_CRON_KEY=$(ROCKET_CRON_KEY) \
		--read-only -v "$(CURDIR)/var":/var --restart=always $(IMAGE)

check: ## Launch tests to verify that the service works as expected, requires a running container.
	@sleep 1
	nc -z localhost $(PORT)
	curl -s http://localhost:$(PORT)/ | grep "Welcome!"
	curl -s http://localhost:$(PORT)/about | grep "About"
	curl -s http://localhost:$(PORT)/add | grep "Add instance"

.cargo/registry:
	mkdir -p .cargo/registry

clean: var/directory.sqlite ## Stops and removes the running container.
	docker stop $(NAME)
	docker rm $(NAME)
	git checkout $(DATABASE)

help: ## Displays these usage instructions.
	@echo "Usage: make <target(s)>"
	@echo
	@echo "Specify one or multiple of the following targets and they will be processed in the given order:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "%-16s%s\n", $$1, $$2}' $(MAKEFILE_LIST)
