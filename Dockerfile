ARG build_image
ARG base_image
FROM ${build_image} AS build
# Install system dependencies ('cmake' is a dependency of Rust crate 'prost-build').
RUN apt update && apt install -y cmake && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
RUN cargo build --release

FROM ${base_image}
COPY --from=build /build/target/release/transaction-logger /usr/local/bin/
ENTRYPOINT [ "transaction-logger" ]
