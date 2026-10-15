# Install `monitor-dhcp` on a remote host

This guide installs `monitor-dhcp` as a systemd service on one dedicated Ethernet
interface. The examples use:

| Setting | Example value |
| --- | --- |
| Remote host | `dhcp-host.example.com` |
| Dedicated Ethernet interface | `enp3s0` |
| Server address | `192.168.50.1/24` |
| Default client address | `192.168.50.2` |
| Known client address | `192.168.50.10` |

Replace these values for the target system. The dedicated interface must not be
the interface carrying the remote SSH session, must not be connected to a
production LAN, and must not have another DHCP server attached to it.

`monitor-dhcp` is intentionally not a general-purpose DHCP server. It assigns a
configured address to a known MAC address and the single `default_ip` to every
unknown client. It does not provide an address pool, DNS, a gateway, routing, or
conflict detection. Only one unknown client should therefore be active on the
link at a time.

## 1. Inspect the remote host

Connect to the host and identify the dedicated Ethernet interface:

```bash
ssh admin@dhcp-host.example.com
ip -br link
ip -br -4 address
```

Confirm the interface name physically, for example by checking its MAC address
or briefly disconnecting its cable. Do not continue until it is clear which
interface is used by SSH and which one will serve DHCP.

Check whether another process already listens on the DHCP server port:

```bash
sudo ss -ulpn 'sport = :67'
```

Stop or reconfigure any conflicting `dnsmasq`, `isc-dhcp-server`, Kea, or
NetworkManager connection-sharing service. A host firewall must also allow
inbound UDP port 67 and outbound UDP port 68 on the dedicated interface.

## 2. Build the executable

Choose either a native build on the remote host or a build on the development
machine followed by a copy.

### Option A: build on the remote host

Copy the repository to the host, install the native build prerequisites and a
current Rust toolchain, and build. The rustup command below is the standard
non-interactive installation from <https://rustup.rs>; review it or use the
host's existing current Rust installation if preferred.

```bash
# Run on the development machine.
rsync -a --exclude target ./ admin@dhcp-host.example.com:~/monitor-dhcp-rust/

# Run on the remote host.
sudo apt-get update
sudo apt-get install -y build-essential curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal
source "$HOME/.cargo/env"
cd ~/monitor-dhcp-rust
cargo build --release --locked
```

The resulting executable is `target/release/monitor-dhcp`.

### Option B: build locally and copy the executable

On the development machine, run the interactive build helper and select the
target host's architecture:

```bash
./quickstart.sh
```

The script prints the path of the completed executable. It supports x86_64,
ARMv6 hard-float, and ARMv7 hard-float, preferring a statically linked musl
build. Copy the printed executable and the deployment files to the remote host:

```bash
scp /path/printed/by/quickstart/monitor-dhcp \
    config.example.toml packaging/monitor-dhcp.service \
    admin@dhcp-host.example.com:/tmp/
```

When the helper falls back to a dynamically linked GNU build, the target must
have a compatible glibc. Check the executable before copying if necessary:

```bash
file /path/printed/by/quickstart/monitor-dhcp
```

## 3. Install the executable and configuration

On the remote host, install the executable selected in the previous step:

```bash
sudo install -Dm0755 /path/to/monitor-dhcp /usr/local/bin/monitor-dhcp
sudo install -d -m0755 /etc/monitor-dhcp
sudo install -m0644 /path/to/config.example.toml /etc/monitor-dhcp/config.toml
```

For a native build, `/path/to/monitor-dhcp` is
`~/monitor-dhcp-rust/target/release/monitor-dhcp` and the example configuration
is `~/monitor-dhcp-rust/config.example.toml`. For the copy example, both files
are under `/tmp`.

Edit the installed configuration:

```bash
sudoedit /etc/monitor-dhcp/config.toml
```

A complete configuration looks like this:

```toml
interface = "enp3s0"

server_ip = "192.168.50.1"
subnet_mask = "255.255.255.0"
default_ip = "192.168.50.2"
lease_seconds = 86400

[hosts]
"00:11:22:33:44:55" = "192.168.50.10"
"AA:BB:CC:DD:EE:FF" = "192.168.50.11"
```

The fields mean:

- `interface`: the dedicated Linux Ethernet interface.
- `server_ip`: the static address that will be configured on that interface.
- `subnet_mask`: the subnet mask sent to clients. It must match the interface
  address prefix.
- `default_ip`: the deterministic address offered to every MAC address not in
  `[hosts]`.
- `lease_seconds`: the lease duration; it must be at least 8 seconds and
  defaults to 86400 when omitted.
- `[hosts]`: optional MAC-to-address reservations. MAC addresses are
  case-insensitive.

All client addresses must be usable addresses in the server subnet and must
differ from `server_ip`. Explicit host addresses cannot be duplicated. A host
reservation may equal `default_ip`, but only one client using that address may
be active safely.

Validate the file before changing the network or starting the service:

```bash
/usr/local/bin/monitor-dhcp --config /etc/monitor-dhcp/config.toml --check
```

The success message notes that this static check does not inspect the network
interface.

## 4. Give the dedicated interface a static address

The application does not configure addresses or routes. Configure `server_ip`
and `subnet_mask` persistently with the remote host's network manager. Do not
set a gateway or DNS server on this dedicated link.

### Netplan example

On an Ubuntu system managed by Netplan, first inspect the existing files:

```bash
ls /etc/netplan
sudo netplan get
```

Add the dedicated interface without replacing the configuration for other
interfaces. For example, create `/etc/netplan/60-monitor-dhcp.yaml` containing:

```yaml
network:
  version: 2
  ethernets:
    enp3s0:
      dhcp4: false
      addresses:
        - 192.168.50.1/24
      optional: true
```

Because this is a remote change, use Netplan's automatic rollback facility:

```bash
sudo netplan generate
sudo netplan try
```

Accept the configuration only after confirming that the SSH session still
works. If `netplan try` is unsuitable for the host's network stack, arrange
console access before using `sudo netplan apply`.

### NetworkManager example

On a host managed directly by NetworkManager, create a manual connection:

```bash
sudo nmcli connection add type ethernet ifname enp3s0 \
  con-name monitor-dhcp-link \
  ipv4.method manual ipv4.addresses 192.168.50.1/24 \
  ipv4.never-default yes ipv4.gateway "" ipv4.dns "" \
  ipv6.method disabled
sudo nmcli connection up monitor-dhcp-link
```

Do not use NetworkManager's `shared` IPv4 method: it starts a competing DHCP/DNS
service.

Verify that the configured address and mask exactly match the TOML file:

```bash
ip -4 address show dev enp3s0
ip route show dev enp3s0
```

## 5. Install and start the systemd service

Create an unprivileged service account and install the supplied unit:

```bash
if ! getent group monitor-dhcp >/dev/null; then
  sudo groupadd --system monitor-dhcp
fi
if ! id monitor-dhcp >/dev/null 2>&1; then
  sudo useradd --system --gid monitor-dhcp --no-create-home \
    --shell /usr/sbin/nologin monitor-dhcp
fi
sudo install -Dm0644 /path/to/monitor-dhcp.service \
  /etc/systemd/system/monitor-dhcp.service
sudo systemctl daemon-reload
sudo systemctl enable --now monitor-dhcp.service
```

For a native repository copy, the unit source is
`~/monitor-dhcp-rust/packaging/monitor-dhcp.service`; for the copy example it is
`/tmp/monitor-dhcp.service`.

The unit runs without root privileges. It grants only `CAP_NET_BIND_SERVICE`,
which is needed for UDP port 67, and `CAP_NET_RAW`, which Linux requires here to
bind the socket to a specific interface. The program does not open a raw socket.

Confirm that startup succeeded:

```bash
systemctl status monitor-dhcp.service --no-pager
journalctl -u monitor-dhcp.service -n 50 --no-pager
sudo ss -ulpn 'sport = :67'
```

The journal should contain a line similar to:

```text
DHCP server started on enp3s0 (192.168.50.1)
```

## 6. Test a client

Connect the monitor or other single client directly to the dedicated Ethernet
interface and configure it to use DHCP. Follow the service log while the client
starts or renews its lease:

```bash
journalctl -u monitor-dhcp.service -f
```

A successful exchange produces `DISCOVER`, `OFFER`, `REQUEST`, and `ACK` log
entries. To inspect the packets themselves:

```bash
sudo tcpdump -ni enp3s0 'udp port 67 or udp port 68'
```

Test connectivity from the server with the address assigned to the client:

```bash
ping -c 3 192.168.50.10
```

## 7. Change configuration or upgrade

After editing the configuration, validate it and reload the service:

```bash
sudo /usr/local/bin/monitor-dhcp \
  --config /etc/monitor-dhcp/config.toml --check
sudo systemctl reload monitor-dhcp.service
journalctl -u monitor-dhcp.service -n 20 --no-pager
```

A reload atomically applies a valid change. An invalid replacement is rejected
and the previous configuration remains active. Changes to `interface` or
`server_ip` require a restart instead:

```bash
sudo systemctl restart monitor-dhcp.service
```

Changing an address assignment affects the client's next DHCP request. Reboot
the client or restart its network interface when immediate reassignment is
needed.

To upgrade the executable, build and copy the new file, then install it and
restart:

```bash
sudo install -m0755 /path/to/new/monitor-dhcp /usr/local/bin/monitor-dhcp
sudo systemctl restart monitor-dhcp.service
/usr/local/bin/monitor-dhcp --version
```

## Troubleshooting

- **`server_ip ... is not assigned to interface`**: configure the exact address
  and mask from `config.toml` on the named interface, then restart the service.
- **`interface ... does not exist`**: correct `interface` using the name shown by
  `ip -br link`. Interface names can differ between machines.
- **`Address already in use`**: another DHCP server owns UDP port 67. Locate it
  with `sudo ss -ulpn 'sport = :67'`, then stop or reconfigure it.
- **The service starts but the client receives nothing**: verify cabling and link
  state with `ip link show dev enp3s0`, check the firewall, ensure connection
  sharing is disabled, and capture traffic with `tcpdump`.
- **The client receives the wrong address**: check its MAC address and the
  `[hosts]` entry. The log records the MAC used for each request.
- **More detail is needed**: add `Environment=RUST_LOG=debug` under `[Service]`
  with `sudo systemctl edit monitor-dhcp.service`, then run
  `sudo systemctl daemon-reload && sudo systemctl restart monitor-dhcp.service`.

This service is suitable only for the isolated, single-client style of link
described above. Do not attach it to a shared or production network.
