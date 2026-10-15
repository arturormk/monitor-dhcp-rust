# Validate a monitor remotely before installation

This guide validates a directly connected monitor and `monitor-dhcp` from an
SSH session before making any persistent network or systemd changes. It is
intended for the delicate case where the host is administered over the network
and changing the wrong interface could end the only management session.

The procedure temporarily brings up the dedicated Ethernet interface, learns
the monitor's wired MAC address, adds the PC address with `ip addr add`, opens
interface-scoped DHCP firewall rules if UFW is active, and runs the executable
in the foreground. It does not edit or apply Netplan.

The examples use:

| Purpose | Example |
| --- | --- |
| SSH administrator | `admin` |
| Remote PC | `display-pc.example.net` |
| Dedicated monitor interface | `enp3s0` |
| PC/server address | `192.168.50.1/24` |
| Monitor fallback address | `192.168.50.2` |

Replace these values for the installation. The dedicated interface must connect
only to the monitor. It must not be the interface carrying SSH, and it must not
be connected to a production network or another DHCP server.

## 1. Protect the management connection

Log in using an administrator account with `sudo` access:

```bash
ssh admin@display-pc.example.net
sudo -v
```

Keep this session open and establish a second SSH session before changing
anything. Use one for `monitor-dhcp` or packet capture and the other for
inspection and recovery.

On the remote PC, identify the route back to the SSH client:

```bash
echo "$SSH_CONNECTION"
ip route get "$(printf '%s\n' "$SSH_CONNECTION" | awk '{print $1}')"
ip -br link
ip -br -4 address
ip route
```

The `ip route get` result names the management interface after `dev`. Record
that name. Stop if it is `enp3s0`, or if there is any doubt about which cable or
interface carries SSH. None of the commands below should be run against the
management interface.

Also verify that `192.168.50.0/24` does not overlap an existing network, VPN,
container network, or the SSH client route. Choose a different unused private
subnet if it does. In particular, this command must not select the proposed
monitor interface before the temporary address has been added:

```bash
ip route get 192.168.50.2
```

Inspect the proposed interface before using it:

```bash
ip -d link show dev enp3s0
ip -4 addr show dev enp3s0
nmcli device status 2>/dev/null || true
networkctl status enp3s0 2>/dev/null || true
sudo ss -ulpn 'sport = :67'
```

Do not continue if the interface already has an address, a default route, an
active NetworkManager connection that may configure it automatically, or an
unexpected DHCP server. Resolve that ownership first. Do not use `ip addr
flush`: on a remote host, a mistaken interface name could remove the management
address.

## 2. Discover the monitor's wired MAC address

No IP address is needed to observe a DHCP request. Bring up only the dedicated
link and start a capture:

```bash
sudo ip link set dev enp3s0 up
ip -br link show dev enp3s0
sudo tcpdump -eni enp3s0 'udp src port 68 and udp dst port 67'
```

Enable LAN in the monitor's on-screen settings, then reconnect its cable or
power-cycle it. A DHCP client normally sends a `DHCPDISCOVER`; with `tcpdump -e`,
the Ethernet source address before `>` is normally the monitor's wired MAC.
For example:

```text
00:11:22:33:44:55 > ff:ff:ff:ff:ff:ff, ethertype IPv4, ... DHCP-Message Option 53, length 1: Discover
```

Record the address and stop `tcpdump` with Ctrl-C. Compare it with the monitor's
menu or label if possible. A label may show the Wi-Fi MAC rather than the wired
one, and `ip link` shows the PC adapter's own MAC, not the monitor's.

If nothing appears, confirm that the interface reports carrier:

```bash
cat /sys/class/net/enp3s0/carrier
sudo tcpdump -eni enp3s0
```

`1` means that Ethernet carrier is present. An entirely silent capture usually
means the monitor's LAN interface is disabled, the monitor has not retried
DHCP, or there is a cable, adapter, or port problem. Recheck the on-screen LAN
setting and power-cycle the monitor. Do not proceed merely by guessing the MAC.

## 3. Build and prepare a temporary configuration

From the checked-out project on the remote PC, build without installing:

```bash
cargo build --release --locked
cp config.example.toml /tmp/monitor-dhcp-preflight.toml
```

Edit `/tmp/monitor-dhcp-preflight.toml` so it contains the dedicated interface,
the temporary PC address, and the MAC just observed. For example:

```toml
interface = "enp3s0"

server_ip = "192.168.50.1"
subnet_mask = "255.255.255.0"
default_ip = "192.168.50.2"
lease_seconds = 86400

[hosts]
"00:11:22:33:44:55" = "192.168.50.10"
```

The explicit reservation is useful for confirming that the captured MAC is
the client receiving the expected address. Validate the file without opening
a socket or inspecting the interface:

```bash
./target/release/monitor-dhcp \
  --config /tmp/monitor-dhcp-preflight.toml --check
```

## 4. Add the PC address without changing Netplan

Add, rather than replace or flush, the temporary address:

```bash
sudo ip addr add 192.168.50.1/24 dev enp3s0
sudo ip link set dev enp3s0 up
ip -br -4 addr show dev enp3s0
ip route show dev enp3s0
```

The connected route should cover only `192.168.50.0/24`; there must be no
default route through `enp3s0`. Re-run the earlier `ip route get` command for
the SSH client and confirm that its management interface and source address
have not changed. Also confirm that the second SSH session still works before
continuing.

This address is a runtime kernel change and normally disappears at reboot. It
does not modify Netplan, but a network manager could still react to link-state
changes; that is why the ownership checks in step 1 matter.

## 5. Allow DHCP through UFW temporarily

Check UFW rather than enabling it as part of this test:

```bash
sudo ufw status verbose
```

If UFW is inactive, leave it inactive and skip this step. Enabling a firewall
for the first time during a remote network test can block SSH.

If UFW is active, first confirm that its existing policy permits the SSH
management connection. Then add narrowly scoped DHCP rules on the dedicated
interface:

```bash
sudo ufw allow in on enp3s0 proto udp \
  from any port 68 to any port 67 comment 'monitor-dhcp preflight'
sudo ufw allow out on enp3s0 proto udp \
  from any port 67 to any port 68 comment 'monitor-dhcp preflight'
sudo ufw status numbered
```

DHCP clients send from UDP port 68 to server port 67; the server replies from
67 to 68. The outbound rule is redundant with UFW's usual default-allow
outgoing policy, but makes the required test traffic explicit and also covers
a host with a restrictive outgoing policy. Interface scoping prevents these
rules from exposing a DHCP server on the management network.

## 6. Run the server in the foreground

Make sure no process has claimed DHCP server port 67:

```bash
sudo ss -ulpn 'sport = :67'
```

If this reports a listener, identify why it is running. Do not start a second
DHCP server or stop an unfamiliar network service from an SSH session.

In the first SSH session, run the build directly from the checkout:

```bash
sudo env RUST_LOG=debug ./target/release/monitor-dhcp \
  --config /tmp/monitor-dhcp-preflight.toml
```

Reconnect or power-cycle the monitor so that it requests a new lease. A
successful exchange includes lines like:

```text
DISCOVER mac=00:11:22:33:44:55
OFFER mac=00:11:22:33:44:55 ip=192.168.50.10
REQUEST mac=00:11:22:33:44:55
ACK mac=00:11:22:33:44:55 ip=192.168.50.10
```

The final `ACK` confirms that `monitor-dhcp` granted the address. From the
second session, optionally observe the complete exchange and test reachability:

```bash
sudo tcpdump -ni enp3s0 -vv 'udp port 67 or udp port 68'
ping -c 3 192.168.50.10
ip neigh show dev enp3s0
```

Some monitors do not answer ICMP echo, so a failed `ping` alone does not undo a
successful DHCP exchange. An `ACK` plus subsequent ARP or application traffic
from `192.168.50.10` is stronger evidence that the monitor configured the
address. If the logs stop at `OFFER`, use the packet capture to determine
whether the offer left the PC and whether the monitor sent a request.

Stop the foreground server with Ctrl-C when validation is complete.

## 7. Remove the temporary changes

Remove only the address added by this guide:

```bash
sudo ip addr del 192.168.50.1/24 dev enp3s0
```

If UFW rules were added, list them and delete only the entries whose comment is
`monitor-dhcp preflight`:

```bash
sudo ufw status numbered
sudo ufw delete NUMBER
```

UFW renumbers its rules after every deletion, so list them again before
deleting the second rule. If `enp3s0` was down before the test, restore that
state after the monitor is disconnected:

```bash
sudo ip link set dev enp3s0 down
```

Finally, verify the management route and current state:

```bash
ip -br -4 address
ip route
sudo ss -ulpn 'sport = :67'
```

After this preflight succeeds, use the project installation instructions to
make the interface address, firewall policy, configuration, and service
persistent. For a single interface, continue with the
[README](../README.md#configure-the-dedicated-interface) and the
[systemd guide](HOWTO-systemd.md). For two monitor links, use the
[network bridge guide](HOWTO-NetworkBridge.md).
