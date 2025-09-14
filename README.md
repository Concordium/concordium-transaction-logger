# Wallet Proxy

[![Contributor Covenant](https://img.shields.io/badge/Contributor%20Covenant-2.0-4baaaa.svg)](https://github.com/Concordium/.github/blob/main/.github/CODE_OF_CONDUCT.md)
![Build and test](https://github.com/Concordium/concordium-node/actions/workflows/build-test.yaml/badge.svg)

This repository contains the Wallet Proxy, which works as middleware for the wallets maintained by 
Concordium. The Wallet Proxy acts as a proxy for accessing the node and additionally provides access to indexed chain data. 
Part of the Wallet Proxy is an indexer that indexes the chain data that is relevant for the wallets.

- [wallet-proxy-indexer](./wallet-proxy-indexer/) 
  Indexer of chain data. The indexed data is written to a Postgres database.
- [wallet-proxy-service](./wallet-proxy-service/) 
  Service that exposes the Wallet Proxy API. Reads data from the Postgres database
  or directly from the node. The crate is still WIP

## Component Interaction Diagram

![Component Interaction Diagram](docs/diagrams/wallet-proxy.drawio.png)

## Submodules

The submodules in `deps` can be checked out using `git submodule update --init --recursive`.
