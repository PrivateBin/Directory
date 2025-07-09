FROM scratch
ARG RELEASE=0.17.2
LABEL org.opencontainers.image.authors=support@privatebin.org \
      org.opencontainers.image.vendor=PrivateBin \
      org.opencontainers.image.documentation=https://github.com/PrivateBin/Directory/blob/master/README.md \
      org.opencontainers.image.source=https://github.com/PrivateBin/Directory \
      org.opencontainers.image.licenses=AGPL-3.0 \
      org.opencontainers.image.version=${RELEASE}

ARG GEOIP_MMDB
ARG ROCKET_DATABASES
ENV GEOIP_MMDB=$GEOIP_MMDB \
    ROCKET_ADDRESS="::" \
    ROCKET_DATABASES=$ROCKET_DATABASES
ARG PORT
EXPOSE $PORT
USER 1000:1000
WORKDIR /
VOLUME /var
CMD ["directory"]

COPY css /css
COPY img /img
COPY target/release/directory /bin/
COPY templates /templates
