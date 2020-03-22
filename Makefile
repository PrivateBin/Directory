.PHONY: build run test clean help all

NAME = directory
IMAGE = privatebin/$(NAME)
PORT = 8000

build: ## Build the container image (default).
	mkdir -p .cargo/registry
	docker run --rm -t \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		ekidd/rust-musl-builder:nightly-2020-03-12 cargo test
	docker run --rm -t \
		-v "$(CURDIR)":/home/rust/src \
		-v "$(CURDIR)"/.cargo/registry:/home/rust/.cargo/registry \
		ekidd/rust-musl-builder:nightly-2020-03-12 cargo build --release
	docker build --build-arg PORT=$(PORT) -t $(IMAGE) .

run: ## Run a container from the image.
	docker run -d --init --name $(NAME) -p=$(PORT):$(PORT) --read-only --restart=always $(IMAGE)

test: ## Launch tests to verify that the service works as expected, requires a running container.
	@sleep 1
	nc -z localhost $(PORT)
	curl -s http://localhost:$(PORT)/ | grep "Welcome!"
	curl -s http://localhost:$(PORT)/add | grep "Add instance"

clean: ## Stops and removes the running container.
	docker stop $(NAME)
	docker rm $(NAME)

help: ## Displays these usage instructions.
	@echo "Usage: make <target(s)>"
	@echo
	@echo "Specify one or multiple of the following targets and they will be processed in the given order:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "%-16s%s\n", $$1, $$2}' $(MAKEFILE_LIST)

all: build run test clean ## Equivalent to "make build run test clean"
