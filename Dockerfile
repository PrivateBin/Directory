FROM ekidd/rust-musl-builder:nightly-2020-03-12
RUN sudo apt-get update && \
    sudo apt-get install -y \
        libsqlite3-dev \
        upx-ucl \
        zlib1g-dev \
    && \
    sudo rm -rf /var/lib/apt/lists/*
RUN curl -Ls https://github.com/PrivateBin/Directory/archive/0.1.0.tar.gz | tar -xz --strip 1 && \
    cargo build --release && \
    mv target/x86_64-unknown-linux-musl/release/directory directory && \
    strip directory && \
    upx --ultra-brute directory



FROM scratch
LABEL maintainer="support@privatebin.org"

ARG GEOIP_MMDB
ENV GEOIP_MMDB $GEOIP_MMDB
ARG ROCKET_DATABASES
ENV ROCKET_DATABASES $ROCKET_DATABASES
ARG PORT
EXPOSE $PORT
USER 1000:1000
WORKDIR /bin
VOLUME /var
CMD ["directory"]

COPY --from=0 /home/rust/src/css /css
COPY --from=0 /home/rust/src/img /img
COPY --from=0 /home/rust/src/migrations /bin/migrations
COPY --from=0 /home/rust/src/directory /bin/
COPY --from=0 /home/rust/src/templates /bin/templates
