# Changelog for the transaction logger service

## Unreleased

## 0.7.2

- Fix bug in CIS2 event parsing. Events emitted by contract init were not
  logged.

## 0.7.1

- Fix bug in parameter size limit parsing for protocol 5.

## 0.7.0

- Add support for protocol 5.

## 0.6.0

- Add an extra table to record the list of CIS2 tokens on each smart contract.

## 0.5.0

- Use Rust SDK V2.
- The logger now requires the V2 GRPC API.
- Minimum Rust version is bumped to 1.61
- `TRANSACTION_LOGGER_RPC_TOKEN` is no longer supported

## 0.4.0

- Bump Node SDK.
- Revise log levels.

## 0.3.1

- Fix parsing of level1 and root key updates in block summaries.

## 0.3.0
- Support for node version 4 API.
- Support delegation and new contract events.
