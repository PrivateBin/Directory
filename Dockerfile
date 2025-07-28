FROM rust:1.80-alpine3.20
RUN apk --no-cache update && \
    apk add --no-cache \
        musl-dev \
        sqlite-static && \
    adduser -D rust
USER rust
WORKDIR /home/rust
# these avoid the overhead of compiling our own static library
ENV SQLITE3_STATIC=1 \
    SQLITE3_LIB_DIR=/usr/lib
COPY . /home/rust/
RUN cargo build --release

FROM scratch
ARG RELEASE=0.18.2
LABEL org.opencontainers.image.authors=support@privatebin.org \
      org.opencontainers.image.vendor=PrivateBin \
      org.opencontainers.image.documentation=https://github.com/PrivateBin/Directory/blob/master/README.md \
      org.opencontainers.image.source=https://github.com/PrivateBin/Directory \
      org.opencontainers.image.licenses=AGPL-3.0 \
      org.opencontainers.image.version=${RELEASE}

ENV GEOIP_MMDB=/var/geoip-country.mmdb \
    ROCKET_ADDRESS="::" \
    ROCKET_DATABASES={directory={url="/var/directory.sqlite"}}
EXPOSE 8000
USER 1000:1000
WORKDIR /
VOLUME /var
CMD ["directory"]

COPY css /css
COPY img /img
COPY --from=0 /home/rust/target/release/directory /bin/
COPY templates /templates
