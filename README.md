# monitor-dhcp

`monitor-dhcp` is a deliberately small DHCPv4 server for one dedicated Ethernet link. It gives a configured address to known MAC addresses and one deterministic default address to every unknown client. It has no address pool, lease database, DNS, routing, or management API.

The configuration file is the source of truth. This design fits a PC directly connected to one commercial monitor that requires DHCP but does not support static configuration.

## Download a release

GitHub [releases](https://github.com/arturormk/monitor-dhcp-rust/releases)
contain a statically linked Linux x86_64 executable, the example configuration,
and the systemd unit. Download the archive and its checksum, then verify and
install it:

```bash
sha256sum --check monitor-dhcp-v1.0-x86_64-unknown-linux-musl.tar.gz.sha256
tar -xzf monitor-dhcp-v1.0-x86_64-unknown-linux-musl.tar.gz
cd monitor-dhcp-v1.0-x86_64-unknown-linux-musl
sudo install -Dm0755 monitor-dhcp /usr/local/bin/monitor-dhcp
sudo install -Dm0644 config.example.toml /etc/monitor-dhcp/config.toml
```

Review the configuration and systemd instructions below before starting the
server. Release binaries are built by GitHub Actions from version tags; no
prebuilt binary is committed to this repository.

## Build and install

Ubuntu 24.04 and Ubuntu 26.04 are supported. Install the Rust toolchain and build:

```bash
cargo build --release
sudo install -Dm0755 target/release/monitor-dhcp /usr/local/bin/monitor-dhcp
sudo install -Dm0644 config.example.toml /etc/monitor-dhcp/config.toml
```

Alternatively, run `./quickstart.sh` and choose x86_64, ARMv6, or ARMv7. It prefers a statically linked musl executable, falls back to the corresponding GNU target only when the static toolchain is unavailable, and prints the resulting executable path.

Edit `/etc/monitor-dhcp/config.toml`; see [config.example.toml](config.example.toml). A reserved address may equal `default_ip`, but because there is no collision avoidance only one such client may be active safely.

Check static configuration without requiring the interface or opening UDP port 67:

```bash
monitor-dhcp --config /etc/monitor-dhcp/config.toml --check
```

## Configure the dedicated interface

Find the wired interface with `ip link`. The interface must be isolated from production networks and must not have another DHCP server. `monitor-dhcp` never changes interface addresses or routes.

When the PC is administered over SSH, validate the monitor, its wired MAC, the
temporary interface address, UFW rules, and a complete DHCP exchange before
making persistent network changes. See
[Validate a monitor remotely before installation](docs/HOWTO-RemotePreflight.md).

For a temporary setup matching the example:

```bash
sudo ip addr add 192.168.50.1/24 dev enp3s0
sudo ip link set enp3s0 up
sudo target/release/monitor-dhcp --config ./config.example.toml
```

For persistent configuration on Ubuntu, either create a NetworkManager connection with IPv4 method `manual`, address `192.168.50.1/24`, no gateway, and no DNS, or define the same static address in the machine's Netplan YAML and run `sudo netplan apply`. NetworkManager connection sharing must be disabled because it starts its own DHCP/DNS service. Also disable or reconfigure any `dnsmasq`, `isc-dhcp-server`, or Kea instance bound to this interface. Confirm the result with `ip -4 addr show dev enp3s0` and `sudo ss -ulpn 'sport = :67'`.

To serve two directly connected monitors through two USB Ethernet adapters with
one service, configure a Linux bridge and bind `monitor-dhcp` to the bridge.
See [Serve two monitors through a Linux network bridge](docs/HOWTO-NetworkBridge.md),
which also covers pre-IP MAC discovery and Wake-on-LAN with `etherwake`.

## systemd

For a hardened system-level installation with a selectable configuration file,
service account permissions, Ubuntu 20.04–26.04 compatibility notes, and
troubleshooting guidance, see [Run monitor-dhcp as a system service](docs/HOWTO-systemd.md).

Create the service account and install the example unit:

```bash
sudo groupadd --system monitor-dhcp
sudo useradd --system --gid monitor-dhcp --no-create-home \
  --shell /usr/sbin/nologin monitor-dhcp
sudo install -Dm0644 packaging/monitor-dhcp.service /etc/systemd/system/monitor-dhcp.service
sudo systemctl daemon-reload
sudo systemctl enable --now monitor-dhcp
```

`CAP_NET_BIND_SERVICE` permits UDP port 67. Linux also requires `CAP_NET_RAW` for `SO_BINDTODEVICE` in the supported deployment; the application does not open a raw socket. The unit restricts address families to IPv4, interface discovery through Netlink, and local sockets.

Operate and inspect it with:

```bash
sudo systemctl restart monitor-dhcp
sudo systemctl reload monitor-dhcp
journalctl -u monitor-dhcp -f
```

SIGHUP atomically applies a valid replacement configuration. Changes to `interface` or `server_ip` are rejected until restart, and an invalid reload leaves the last valid configuration active. Address changes take effect on the client's next request; immediate reassignment may require rebooting the monitor or restarting its network interface.

Set `RUST_LOG=debug` for transaction-level diagnostics. Capture traffic with:

```bash
sudo tcpdump -ni enp3s0 'udp port 67 or udp port 68'
```

## Tests

Run all unprivileged checks:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

The namespace integration suite requires root, Linux network namespaces, `iproute2`, `dhclient`, `ping`, and Python 3:

```bash
sudo ./tests/netns.sh
```

It creates disposable veth networks and verifies a real client's acquisition, subnet mask and connectivity, deterministic reacquisition, renewal, valid and invalid SIGHUP reloads, malformed-packet survival, a client with the broadcast flag clear, and isolation from a second interface. It cleans up its namespaces on exit.

## Versioning and releases

The version in `Cargo.toml` is the manually maintained source version. The
executable displays even minor versions in `major.minor` form, so `1.2.0`
becomes `v1.2` and never changes on its own. For odd minor versions, the build
adds the Git commit count: `1.3.0` at commit 212 becomes `v1.3.212`. Odd minor
versions are therefore suitable for development builds between stable even
minor releases.

To publish a release, tag the commit with the exact version reported by the
binary and push the tag. The release workflow rejects a mismatched tag, builds
a static x86_64 Linux archive, generates its SHA-256 checksum, and attaches
both files to a GitHub release:

```bash
cargo run --release -- --version
git tag v1.0
git push origin v1.0
```

## Limitations

- Unknown clients all receive the same `default_ip`; only one should be active on the dedicated link.
- There is no persistent or in-memory lease state and no conflict detection or alternate allocation after DECLINE.
- Initial replies with `ciaddr = 0` are sent as IPv4 broadcasts, even when the client clears its broadcast flag. The namespace test verifies this works with the tested Linux client and wire probe. Devices that require pre-address link-layer unicast may still be incompatible; no `AF_PACKET` path is claimed or included.
- Relay traffic, BOOTP-only requests, IPv6, pools, DNS, gateways, PXE, routing, and DHCPFORCERENEW are intentionally unsupported.
