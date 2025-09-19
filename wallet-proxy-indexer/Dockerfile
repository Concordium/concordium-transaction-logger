ARG build_image=rust:1.89-bookworm
ARG base_image=debian:bookworm
FROM ${build_image} AS build

WORKDIR /build
COPY . .
RUN cargo build -p wallet-proxy-indexer --release

FROM ${base_image}
COPY --from=build /build/target/release/wallet-proxy-indexer /usr/local/bin/
ENTRYPOINT [ "wallet-proxy-indexer" ]
