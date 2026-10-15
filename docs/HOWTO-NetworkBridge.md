# Serve two monitors through a Linux network bridge

This guide connects two monitor LAN ports to two dedicated USB-to-Ethernet
adapters and serves both monitors with one `monitor-dhcp` process. Linux makes
the adapters ports of a software Ethernet switch called `br-monitor`:

```text
monitor A --- USB Ethernet A ---\
                               br-monitor --- monitor-dhcp / control software
monitor B --- USB Ethernet B ---/
```

`monitor-dhcp` still binds to one interface. That interface is the bridge
rather than either physical adapter. The bridge has the PC's static IPv4
address; its member adapters do not have addresses of their own.

This is preferable to two service instances when both monitors may share one
Ethernet broadcast domain. It needs no application changes, routing, NAT,
external switch, or additional power supply. Do not use a Linux bond for this:
a bond treats its member links as redundant or parallel paths to the same
network, not as independent ports for two endpoint devices.

Use two isolated interfaces and two `monitor-dhcp` instances instead when:

- the monitors use the same or unstable MAC address;
- the monitors must not be able to exchange Ethernet traffic;
- more than one unrecognized device may be connected at once; or
- a failure or reconfiguration of one link must not affect the other.

## Important address limitation

Every unknown client receives the configured `default_ip`. On a bridge, two
unknown monitors would therefore be offered the same address. Give the two
monitors distinct entries in `[hosts]` before operating them together:

```toml
interface = "br-monitor"

server_ip = "192.168.50.1"
subnet_mask = "255.255.255.0"
default_ip = "192.168.50.2"
lease_seconds = 86400

[hosts]
"00:11:22:33:44:55" = "192.168.50.10"
"AA:BB:CC:DD:EE:FF" = "192.168.50.11"
```

Keep `default_ip` different from both reservations. It can then support one
replacement monitor temporarily while its MAC address is being identified.
Two simultaneous unknown monitors remain unsafe because both receive the same
fallback address.

The remainder of this guide uses these example values:

| Setting | Example |
| --- | --- |
| First USB Ethernet adapter | `enp4s0` |
| Second USB Ethernet adapter | `enp5s0` |
| Bridge | `br-monitor` |
| PC/server address | `192.168.50.1/24` |
| First monitor | `192.168.50.10` |
| Second monitor | `192.168.50.11` |
| Unknown-client fallback | `192.168.50.2` |

Replace the interface names, addresses, and MAC addresses for the installation.
Both adapters must be dedicated to the monitors. In particular, neither may be
the interface carrying an SSH session or other production network traffic.

## Find the monitor MAC addresses before assigning IP addresses

A DHCP client sends its Ethernet MAC address before it has an IPv4 address.
Connect and identify one monitor at a time so that the observed MAC can be
associated with the correct physical display.

### Capture DHCP on the physical adapter

No IP address is required for this method. Bring up the dedicated adapter and
capture DHCP frames, then power or reconnect the monitor:

```bash
sudo ip link set enp4s0 up
sudo tcpdump -eni enp4s0 'udp src port 68 and udp dst port 67'
```

The `-e` option prints the Ethernet header. The source address before the `>`
on a DHCPDISCOVER frame is normally the monitor MAC. Repeat on `enp5s0` with
only the second monitor connected. Stop `tcpdump` with Ctrl-C.

This also works after the bridge is configured: capture on `enp4s0` or
`enp5s0`, rather than only on `br-monitor`, to identify the physical port on
which the frame arrived.

### Read the monitor-dhcp journal

`monitor-dhcp` logs the client MAC as soon as it accepts a DHCPDISCOVER, before
the offered address is usable:

```bash
journalctl -u monitor-dhcp.service -f
```

A transaction begins with a line similar to:

```text
DISCOVER mac=00:11:22:33:44:55
```

When using this discovery method, connect only one unknown monitor at a time.
Record its MAC and disconnect it before discovering the next monitor. After
both MACs are known, add both reservations and reconnect the monitors. If the
first monitor remains connected, force it to reacquire and verify its reserved
address before connecting another unknown client. A change to `[hosts]` can be
reloaded; changing the configured interface requires a service restart.

### Inspect the bridge forwarding database

After a monitor has transmitted any Ethernet frame, the bridge learns its
source MAC and the port on which it was seen:

```bash
bridge fdb show br br-monitor
```

Entries associated with `enp4s0` or `enp5s0` help map a MAC to a cable. Dynamic
entries age out, so wake or reconnect the monitor if it is not listed.

`ip link` shows the MAC address of the PC's USB adapter, not the monitor at the
other end of the cable. `ip neigh` is also not the right first tool here: its
IPv4 neighbor entries normally appear only after IP and ARP activity.

Labels, menus, or vendor documentation may also show the monitor MAC. Compare
them with a capture when possible; some products label a Wi-Fi MAC or another
internal interface instead of the wired LAN port.

## Try the bridge temporarily with `ip`

Stop the service before moving its current interface into a bridge:

```bash
sudo systemctl stop monitor-dhcp.service
```

The following commands remove any existing addresses from the two dedicated
adapters. Do not run them against a management or production interface.

```bash
sudo ip link add name br-monitor type bridge
sudo ip link set dev br-monitor type bridge stp_state 0

sudo ip addr flush dev enp4s0
sudo ip addr flush dev enp5s0
sudo ip link set dev enp4s0 master br-monitor
sudo ip link set dev enp5s0 master br-monitor

sudo ip link set dev enp4s0 up
sudo ip link set dev enp5s0 up
sudo ip address add 192.168.50.1/24 dev br-monitor
sudo ip link set dev br-monitor up
```

STP is disabled because this exact topology has two direct endpoint links and
cannot contain a Layer-2 loop. Leave STP enabled if either port might instead
reach a switch, a monitor with an Ethernet pass-through port, or any topology
that could loop traffic back into the bridge.

Inspect the result:

```bash
ip -br link show dev enp4s0
ip -br link show dev enp5s0
ip -br link show dev br-monitor
ip -br -4 address show dev br-monitor
bridge link show
ip route show dev br-monitor
```

The IPv4 address must appear on `br-monitor`, not on either member. Both members
should report `master br-monitor`. There should be a connected route for
`192.168.50.0/24` and no default route through the bridge.

Change `/etc/monitor-dhcp/config.toml` to use `interface = "br-monitor"`, add
the two reservations, validate it, and restart the service:

```bash
sudo /usr/local/bin/monitor-dhcp \
  --config /etc/monitor-dhcp/config.toml --check
sudo systemctl restart monitor-dhcp.service
journalctl -u monitor-dhcp.service -n 30 --no-pager
```

An interface change cannot be applied with `systemctl reload`; a restart is
required because the DHCP socket must be rebound to the new interface.

The temporary bridge disappears at reboot. To remove it manually:

```bash
sudo systemctl stop monitor-dhcp.service
sudo ip link set dev enp4s0 nomaster
sudo ip link set dev enp5s0 nomaster
sudo ip link set dev br-monitor down
sudo ip link delete dev br-monitor type bridge
```

Restore the previous addresses or apply the previous Netplan configuration
before restarting any service that used the independent adapters.

## Configure the bridge persistently with Netplan

First inspect the existing configuration:

```bash
ls -l /etc/netplan
sudo netplan get
```

Add this configuration without replacing definitions for unrelated interfaces.
For example, create `/etc/netplan/60-monitor-bridge.yaml`:

```yaml
network:
  version: 2
  renderer: networkd
  ethernets:
    enp4s0:
      dhcp4: false
      dhcp6: false
      accept-ra: false
      link-local: []
      optional: true
    enp5s0:
      dhcp4: false
      dhcp6: false
      accept-ra: false
      link-local: []
      optional: true
  bridges:
    br-monitor:
      interfaces:
        - enp4s0
        - enp5s0
      addresses:
        - 192.168.50.1/24
      dhcp4: false
      dhcp6: false
      accept-ra: false
      link-local: []
      optional: true
      parameters:
        stp: false
```

`optional: true` prevents boot from waiting for a monitor or USB adapter that
is disconnected. No gateway, default route, or DNS server belongs on this
dedicated network.

Generate and inspect the backend configuration before applying it:

```bash
sudo chmod 600 /etc/netplan/60-monitor-bridge.yaml
sudo netplan generate
sudo netplan try
```

Accept the trial only after confirming that unrelated connectivity still
works. Although these should be dedicated adapters, arrange local console
access before changing networking on a remote machine. If `netplan try` is not
suitable for the installed backend, use `sudo netplan apply` only with a tested
rollback path.

Then restart `monitor-dhcp` and perform the inspection commands from the
temporary setup section.

## Wake a powered-off monitor through the bridge

Wake-on-LAN does not need the monitor's IPv4 address or an ARP entry. It embeds
the target MAC repeatedly in a magic Ethernet frame. Install Ubuntu's
`etherwake` package and send the frame through the bridge:

```bash
sudo apt-get update
sudo apt-get install etherwake
sudo etherwake -b -i br-monitor 00:11:22:33:44:55
```

Use the MAC from the matching `[hosts]` entry. `-i br-monitor` selects the
bridge, and `-b` uses an Ethernet broadcast destination. The bridge floods the
frame to both monitor ports; only the monitor whose MAC is encoded in the magic
packet should wake. Broadcasting is also robust after the sleeping monitor's
dynamic bridge-FDB entry has aged out.

If a particular monitor documents that it requires a unicast magic frame, omit
`-b`:

```bash
sudo etherwake -i br-monitor 00:11:22:33:44:55
```

The broadcast form is the recommended default. A unicast frame can depend on a
correct learned bridge-FDB entry, although an unknown unicast is normally
flooded. If the monitor supports a SecureOn password, consult `man etherwake`
for the `-p` option.

Confirm that a magic frame leaves the relevant physical port:

```bash
sudo tcpdump -eni enp4s0 'ether proto 0x0842'
```

Run `etherwake` from another terminal. Capturing a frame proves that Linux and
the bridge sent it; it does not prove that the monitor accepted it.

For Wake-on-LAN to work:

- the monitor must explicitly support WOL in its powered-off or standby state;
- the LAN electronics must retain standby power and continue listening;
- the cable, USB adapter, bridge, and physical port must remain up;
- bridge-family firewall rules must not discard the frame; and
- the sender needs root privileges or `CAP_NET_RAW`.

If powering off the monitor removes power from its Ethernet PHY completely,
software on the PC cannot wake it over that cable. Check the monitor's power,
energy-saving, network-standby, or quick-start settings. Also retain the MAC in
configuration: a sleeping interface cannot answer ARP or DHCP to rediscover it.

## Verify both monitors

Follow DHCP activity while starting or renewing both monitors:

```bash
journalctl -u monitor-dhcp.service -f
```

The log should show each MAC receiving its own address. Inspect packets and
test connectivity:

```bash
sudo tcpdump -eni br-monitor 'udp port 67 or udp port 68 or arp'
ping -c 3 192.168.50.10
ping -c 3 192.168.50.11
ip neigh show dev br-monitor
bridge fdb show br br-monitor
```

The PC application can connect normally to `192.168.50.10` and
`192.168.50.11`. No IP forwarding or routing between interfaces is involved.

If a host firewall is enabled, allow inbound UDP port 67 and outbound UDP port
68 on `br-monitor`, plus the application traffic used to control the monitors.
Disable or reconfigure any `dnsmasq`, Kea, `isc-dhcp-server`, or other DHCP
server attached to this bridge.

## Troubleshooting

- **The service rejects `br-monitor`:** confirm that the bridge exists, is up,
  and owns exactly the `server_ip` and mask from the TOML file. Then restart,
  rather than reload, the service.
- **Both monitors receive the same address:** add distinct `[hosts]`
  reservations and ensure the recorded wired MAC addresses are different.
- **One adapter is missing after reboot:** check its predictable interface name
  with `ip -br link`, USB enumeration, and `networkctl status`. Update Netplan if
  its name changed.
- **DHCP reaches only one port:** inspect `bridge link show`, carrier state, and
  any bridge-family nftables rules. Both members must be forwarding.
- **WOL is visible in `tcpdump` but does not wake the display:** verify the MAC,
  monitor WOL/network-standby settings, standby power, and whether the model
  accepts broadcast or unicast magic frames.
- **A link takes time to pass traffic:** ensure STP is disabled only if the
  installation truly has the documented loop-free, direct-link topology.

## Roll back the persistent configuration

Stop `monitor-dhcp`, remove or revert the bridge YAML, and restore the previous
Netplan definitions for `enp4s0` and `enp5s0`. Validate and apply the result:

```bash
sudo systemctl stop monitor-dhcp.service
sudo netplan generate
sudo netplan try
```

Finally change `interface` in the service configuration to the intended
independent adapter before restarting it. Two simultaneous service instances
also require separate TOML files, different subnets, and separate systemd
instance units; that alternative is intentionally outside this bridge guide.
