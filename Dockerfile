FROM scratch
LABEL maintainer="support@privatebin.org"

ARG GEOIP_MMDB
ENV GEOIP_MMDB $GEOIP_MMDB
ARG DATABASE
ENV DATABASE $DATABASE
ARG PORT
EXPOSE $PORT 8001
USER 1000:1000
WORKDIR /bin
VOLUME /var
CMD ["directory"]

COPY css /css
COPY img /img
COPY target/x86_64-unknown-linux-musl/release/directory /bin/
COPY templates /bin/templates
