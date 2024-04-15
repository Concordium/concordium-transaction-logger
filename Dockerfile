ARG build_image=rust:1.74-bookworm
ARG base_image=debian:bookworm
FROM ${build_image} AS build

WORKDIR /build
COPY . .
RUN cargo build --release

FROM ${base_image}
COPY --from=build /build/target/release/transaction-logger /usr/local/bin/
ENTRYPOINT [ "transaction-logger" ]
