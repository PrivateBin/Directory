FROM scratch
LABEL maintainer="support@privatebin.org"

ARG GEOIP_MMDB
ENV GEOIP_MMDB $GEOIP_MMDB
ENV ROCKET_ADDRESS "::"
ARG ROCKET_DATABASES
ENV ROCKET_DATABASES $ROCKET_DATABASES
ARG PORT
EXPOSE $PORT
USER 1000:1000
WORKDIR /
VOLUME /var
CMD ["directory"]

COPY css /css
COPY img /img
COPY target/x86_64-unknown-linux-musl/release/directory /bin/
COPY templates /templates
