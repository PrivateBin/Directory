.PHONY: all release test build pack image run check clean help

NAME = directory
IMAGE = privatebin/$(NAME)
PORT = 8000
GEOIP_MMDB=var/geoip-country.mmdb

all: test build image run check clean ## Equivalent to "make test build image run check clean" (default).

release: test build pack image run check clean ## Equivalent to "make test build pack image run check clean".

test: .cargo/registry ## Build and run the unit tests.
	docker run --rm -t --init -e GEOIP_MMDB="/home/rust/src/$(GEOIP_MMDB)" \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		ekidd/rust-musl-builder:nightly-2020-03-12 \
		cargo test --release # -- --nocapture

build: .cargo/registry ## Build the binary for release.
	docker run --rm -t --init -e GEOIP_MMDB="/home/rust/src/$(GEOIP_MMDB)" \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		ekidd/rust-musl-builder:nightly-2020-03-12 \
		cargo build --release

pack: ## Strips and compresses the binary to reduce it's size, only intended for the release.
	strip target/x86_64-unknown-linux-musl/release/directory
	upx --ultra-brute target/x86_64-unknown-linux-musl/release/directory

image: ## Build the container image.
	docker build --build-arg PORT=$(PORT) --build-arg GEOIP_MMDB="/$(GEOIP_MMDB)" -t $(IMAGE) .

run: ## Run a container from the image.
	docker run -d --init --name $(NAME) -p=$(PORT):$(PORT) \
	   --read-only -v "$(CURDIR)":/var:ro --restart=always $(IMAGE)

check: ## Launch tests to verify that the service works as expected, requires a running container.
	@sleep 1
	nc -z localhost $(PORT)
	curl -s http://localhost:$(PORT)/ | grep "Welcome!"
	curl -s http://localhost:$(PORT)/add | grep "Add instance"

.cargo/registry:
	mkdir -p .cargo/registry

clean: ## Stops and removes the running container.
	docker stop $(NAME)
	docker rm $(NAME)

help: ## Displays these usage instructions.
	@echo "Usage: make <target(s)>"
	@echo
	@echo "Specify one or multiple of the following targets and they will be processed in the given order:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "%-16s%s\n", $$1, $$2}' $(MAKEFILE_LIST)

