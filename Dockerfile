FROM scratch
LABEL maintainer="support@privatebin.org"
WORKDIR /bin
COPY target/x86_64-unknown-linux-musl/release/directory /bin/
COPY img /img
ARG PORT
EXPOSE $PORT
USER 1000:1000
CMD ["directory"]
