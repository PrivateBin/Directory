FROM scratch
LABEL maintainer="support@privatebin.org"
WORKDIR /bin
ARG PORT
EXPOSE $PORT
USER 1000:1000
CMD ["directory"]
COPY css /css
COPY img /img
COPY target/x86_64-unknown-linux-musl/release/directory /bin/
COPY templates /bin/templates
