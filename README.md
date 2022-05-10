# Transaction logger

Log affected accounts and smart contracts into a postgres database.

# Supported configuration options

- `TRANSACTION_LOGGER_NODES`
  List of nodes to query. They are used in order, and the next one is only used
  if querying preceding one failed. Must be non-empty. For example
  `http://localhost:10000,http://localhost:13000`

- `TRANSACTION_LOGGER_RPC_TOKEN`
  GRPC access token for all the nodes.

- `TRANSACTION_LOGGER_DB_STRING`
  Database connection string for the postgres database.
  For example `host=localhost dbname=transaction-outcome user=postgres password=password port=5432`

- `TRANSACTION_LOGGER_LOG_LEVEL`
  Log level. One of `off`, `error`, `warn`, `info`, `debug`. Default is `off`.

- `TRANSACTION_LOGGER_NUM_PARALLEL_QUERIES`
  Maximum number of parallel queries to make to the node. Usually 1 is the
  correct number, but during initial catchup it is useful to increase this to,
  say 8 to take advantage of parallelism in queries which are typically IO bound.

- `TRANSACTION_LOGGER_MAX_BEHIND_SECONDS` Maximum number of seconds since last
  finalization before a node is deemed behind is dropped upon a failed query. In
  such a case a new node is attempted.

# Failure handling

The service handles nodes disappearing or getting behind, as well as the
database connection being lost. The key design features of the service are

- Each block is written in a single database transaction. Thus any failure of
  the service leaves the database in an easily recoverable state. If a
  connection to the database is lost, the service will attempt to reconnect,
  with exponential backoff, up to 6 times.
- A list of nodes and their endpoints can be given. If querying a node fails,
  the next one in the list is attempted, in a round-robin fashion.

  The next node in the list is attempted in any of following cases
  - the node query fails due to network issues, or because the node failed to
    respond with the expected response
  - if the node is too far behind. This is currently determined by using the
    node's last finalized time as an indicator which is not perfect, although it
    should be sufficient. If this proves to be an unreliable test we could
    instead revise it to take into account
    - block arrival latency
    - time of the last finalized block

# Contributing

[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.0-4baaaa.svg)](https://github.com/Concordium/.github/blob/main/.github/CODE_OF_CONDUCT.md)

This repository's CI automatically checks formatting and common problems in rust.
Changes to any of the packages must be such that
- ```cargo clippy --all``` produces no warnings
- ```rust fmt``` makes no changes.

Everything in this repository should build with stable rust at the moment (at least version 1.53 and up), however the fmt tool must be from a nightly release since some of the configuration options are not stable. One way to run the `fmt` tool is
```
 cargo +nightly-2019-11-13 fmt
```
(the exact version used by the CI can be found in [.github/workflows/ci.yaml](.github/workflows/ci.yaml) file).
You will need to have a recent enough nightly version installed, which can be done via
```
rustup toolchain install nightly-2019-11-13
```
or similar, using the [rustup](https://rustup.rs/) tool. See the documentation of the tool for more details.

In order to contribute you should make a pull request and ask a person familiar with the codebase for a review.

## Building

The project is a pure Rust project, and can be build by running

```shell
cargo build --release
```

This produces a single binary `target/release/transaction-logger`.
