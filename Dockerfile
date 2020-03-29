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

COPY css /css
COPY img /img
COPY migrations /bin/migrations
COPY target/x86_64-unknown-linux-musl/release/directory /bin/
COPY templates /bin/templates
