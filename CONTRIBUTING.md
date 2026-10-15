# Contributing

Bug reports and focused pull requests are welcome. Please describe the network
topology and the client involved when reporting protocol or interoperability
problems.

Before opening a pull request, run:

```console
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

The privileged network-namespace suite is Linux-only and requires the tools
listed in the README:

```console
sudo ./tests/netns.sh
```

Do not include real MAC addresses, private network details, or packet captures
containing sensitive data in a public issue.
