ARG build_image
ARG base_image
FROM ${build_image} AS build
# 'rustfmt' is needed by run custom build command for 'concordium-rust-sdk'.
RUN rustup component add rustfmt
WORKDIR /build
COPY . .
RUN cargo build --release

FROM ${base_image}
COPY --from=build /build/target/release/transaction-logger /usr/local/bin/
ENTRYPOINT [ "transaction-logger" ]
