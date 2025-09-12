# Changelog for the transaction logger service

## Unreleased changes

- Update the Rust SDK for better forwards compatibility with future node versions and revised error handling or reporting for unknown transaction and event types. New type `IndexingError` created in lib. Error handling in fn `get_cis2_events`, `insert_transaction` and `on_finalized_block`.

## [0.14.0] - 2025-08-07

### Changed

- Update Rust SDK submodule in order to fix JSON serialization of `RejectReason`

## [0.13.0] - 2025-07-15

### Changed

- Update base submodule in order to support PLT pause 

## [0.12.0] - 2025-06-30

Database schema version: 1

### Changed

- Update `rust-sdk` submodule link dependency.

## [0.11.0] - 2025-06-25

Database schema version: 1

### Added

- Affected accounts now include accounts that have their plt balance changed as part of block item summary outcomes.
- Added database migration logic.

## [0.10.0]

Database schema version: 1

- Added support for suspension related transaction events and special outcomes.

## [0.9.0]

- Moved the postgres feature from the Rust SDK into this crate as its own code.
- Updated the CI Rust version to 1.73

## [0.8.0]

- Add support for node version 6. This is a breaking change and this version
  of the logger only supports node version 6.

## [0.7.4]

- Add support for TLS connection to the node. If the node URL starts with
  `https` then a TLS connection will be attempted. The service takes certificate
  roots from the host on which it is running.

## [0.7.3]

- Add options `TRANSACTION_LOGGER_CONNECT_TIMEOUT` (default 10s) and
  `TRANSACTION_LOGGER_REQUEST_TIMEOUT` (default 60s) for timing out the initial
  connection to the node, and each request to the node.

## [0.7.2]

- Fix bug in CIS2 event parsing. Events emitted by contract init were not
  logged.

## [0.7.1]

- Fix bug in parameter size limit parsing for protocol 5.

## [0.7.0]

- Add support for protocol 5.

## [0.6.0]

- Add an extra table to record the list of CIS2 tokens on each smart contract.

## [0.5.0]

- Use Rust SDK V2.
- The logger now requires the V2 GRPC API.
- Minimum Rust version is bumped to 1.61
- `TRANSACTION_LOGGER_RPC_TOKEN` is no longer supported

## [0.4.0]

- Bump Node SDK.
- Revise log levels.

## [0.3.1]

- Fix parsing of level1 and root key updates in block summaries.

## [0.3.0]
- Support for node version 4 API.
- Support delegation and new contract events.
