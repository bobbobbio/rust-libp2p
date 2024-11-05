#!/bin/bash

export RUST_LOG=info

if [[ "$ACTION" == 1 ]]; then
    target/release/dcutr-example --secret-key-seed 1 --mode listen --relay-address /ip4/107.170.193.89/tcp/4001/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN
else
    target/release/dcutr-example --secret-key-seed 2 --mode dial --relay-address /ip4/107.170.193.89/tcp/4001/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN --remote-peer-id 12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X
fi
