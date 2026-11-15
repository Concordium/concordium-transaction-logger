ARG build_image=rust:1.89-bookworm
ARG base_image=debian:bookworm
FROM ${build_image} AS build

WORKDIR /build
COPY . .
RUN cargo build -p wallet-proxy-indexer --release

FROM ${base_image}
# The indexer verifies the certificate of an https node endpoint against the system trust store.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/wallet-proxy-indexer /usr/local/bin/
ENTRYPOINT [ "wallet-proxy-indexer" ]
