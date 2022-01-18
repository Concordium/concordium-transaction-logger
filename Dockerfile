ARG build_image
ARG ubuntu_image_tag
FROM ${build_image} AS build
# 'rustup' is needed by run custom build command for 'concordium-rust-sdk'.
RUN rustup component add rustfmt
WORKDIR /build
COPY . .
RUN cargo build --release

FROM ubuntu:${ubuntu_image_tag}
COPY --from=build /build/target/release/transaction-logger /usr/local/bin/
ENTRYPOINT [ "transaction-logger" ]
