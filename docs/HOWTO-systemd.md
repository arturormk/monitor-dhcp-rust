# Run monitor-dhcp as a system service

This guide installs one `monitor-dhcp` process as a system-level systemd
service. It targets Ubuntu 20.04 through Ubuntu 26.04 and works with either a
physical Ethernet interface or a Linux bridge.

The recommended arrangement is:

- a root-owned executable under `/usr/local/bin`;
- a dedicated, unprivileged `monitor-dhcp` account;
- a root-owned configuration readable by the service group;
- the interface and server address selected in the TOML configuration;
- the configuration file path selected by a systemd environment setting; and
- only the two Linux capabilities required to bind the DHCP socket.

This is a system service, not a user service. DHCP uses privileged UDP port 67,
must bind to a system network interface, and must start without an interactive
login session.

## 1. Configure the network first

`monitor-dhcp` does not create interfaces or assign addresses. Before starting
the service, configure the dedicated Ethernet interface or bridge with the
same IPv4 address and prefix that the TOML file will specify.

For a single physical interface, see the network configuration in the project
[README](../README.md#configure-the-dedicated-interface). For two monitor
links joined by a bridge, see
[HOWTO-NetworkBridge.md](HOWTO-NetworkBridge.md).

The interface must:

- exist and be Ethernet-compatible;
- own the configured `server_ip` with the configured `subnet_mask`;
- have no competing DHCP server;
- not provide a default route or DNS for the host; and
- be isolated from production networks.

This guide uses `br-monitor` and `/etc/monitor-dhcp/display-wall.toml` as
examples. A physical name such as `enp3s0` works identically.

## 2. Install the executable

Build the release executable, then install it as a root-owned file that cannot
be modified by the service account:

```bash
cargo build --release --locked
sudo install -o root -g root -m 0755 \
  target/release/monitor-dhcp /usr/local/bin/monitor-dhcp
```

Confirm that the installed executable starts and that it is the expected
version:

```bash
/usr/local/bin/monitor-dhcp --version
ls -l /usr/local/bin/monitor-dhcp
```

Do not grant capabilities directly to the executable with `setcap`. The unit
grants capabilities only to this service process. Keeping the executable free
of file capabilities avoids granting extra privilege when an administrator or
another service runs it outside systemd.

## 3. Create the service account

Create a stable system group and user with no home directory and no login
shell. The commands are safe to repeat:

```bash
if ! getent group monitor-dhcp >/dev/null; then
  sudo groupadd --system monitor-dhcp
fi
if ! getent passwd monitor-dhcp >/dev/null; then
  sudo useradd --system --gid monitor-dhcp --no-create-home \
    --shell /usr/sbin/nologin monitor-dhcp
fi
```

A static service account is preferable to `DynamicUser=yes` here because the
configuration has a stable group owner and administrators may want to inspect
process ownership consistently. The daemon writes no lease database, state,
cache, or log files; logs go to the journal.

## 4. Install a specific configuration

Create the configuration directory so only root and the service group can
traverse it:

```bash
sudo install -o root -g monitor-dhcp -m 0750 -d /etc/monitor-dhcp
sudo install -o root -g monitor-dhcp -m 0640 \
  config.example.toml /etc/monitor-dhcp/display-wall.toml
sudoedit /etc/monitor-dhcp/display-wall.toml
```

For a bridge, the relevant part might be:

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

For a direct connection, set `interface` to the physical Ethernet interface.
The interface belongs in TOML rather than in the unit because it is part of the
DHCP configuration and is validated together with the address and subnet.

Keep the file owned by root. The daemon only needs group read access and should
not be able to rewrite its own policy:

```bash
sudo chown root:monitor-dhcp /etc/monitor-dhcp/display-wall.toml
sudo chmod 0640 /etc/monitor-dhcp/display-wall.toml
```

Run the static validation as the service user. This catches syntax, address,
lease, and reservation errors without opening UDP port 67:

```bash
sudo -u monitor-dhcp /usr/local/bin/monitor-dhcp \
  --config /etc/monitor-dhcp/display-wall.toml --check
```

The success message says that the network interface was not checked. Runtime
startup performs that check after the system network configuration is active.

## 5. Install the unit

Install the unit supplied by the project:

```bash
sudo install -o root -g root -m 0644 packaging/monitor-dhcp.service \
  /etc/systemd/system/monitor-dhcp.service
sudo systemctl daemon-reload
```

The installed unit defaults to `/etc/monitor-dhcp/config.toml`. Select the
specific file for this installation with a drop-in rather than editing the
unit itself:

```bash
sudo systemctl edit monitor-dhcp.service
```

Enter:

```ini
[Service]
Environment="MONITOR_DHCP_CONFIG=/etc/monitor-dhcp/display-wall.toml"
```

The drop-in is stored under
`/etc/systemd/system/monitor-dhcp.service.d/`. It survives replacement of the
main unit during an upgrade and makes the local choice visible separately from
the packaged policy.

Inspect the fully merged unit and confirm the selected path:

```bash
systemctl cat monitor-dhcp.service
systemctl show monitor-dhcp.service -p Environment
```

Paths containing whitespace work with the `${MONITOR_DHCP_CONFIG}` expansion
used by the unit, but conventional whitespace-free configuration paths are
easier to operate and troubleshoot.

## 6. Understand the unit policy

The supplied unit intentionally uses directives available in systemd 245, the
version shipped by Ubuntu 20.04. It therefore remains valid on newer target
releases without requiring version-specific unit variants.

### Startup and network ordering

`Wants=network-online.target` and `After=network-online.target` order startup
after the host's network manager reports its configuration ready. This is an
ordering relationship, not a substitute for configuring the interface.

`ExecStartPre` runs `monitor-dhcp --check` using the same account, filesystem
sandbox, and selected path as the daemon. The main process then verifies that
the interface exists and owns the exact server address and mask. A fixed sleep
in the unit would only hide an ordering problem and is not used.

`Type=exec` makes `systemctl start` report process setup failures such as a
missing executable or service account. The daemon stays in the foreground, so
there is no PID file, forked background process, or shell wrapper for systemd
to track.

For a Netplan bridge, configure the bridge persistently and put the address on
the bridge itself. A bridge may exist without carrier, which is useful when a
monitor is powered off. For a hot-plugged physical USB adapter, start or restart
the service after the adapter has appeared and Netplan has configured it.

### Privileges

The process runs as `monitor-dhcp`, not root. Its capability bounding and
ambient sets contain only:

- `CAP_NET_BIND_SERVICE`, to bind UDP port 67; and
- `CAP_NET_RAW`, required by Linux for this use of `SO_BINDTODEVICE`.

`NoNewPrivileges=true` prevents later privilege gain. The executable itself has
no file capabilities or set-user-ID bit.

### Filesystem and kernel isolation

The service has a read-only host filesystem through `ProtectSystem=strict`, no
home-directory access, a private temporary directory, and a restricted device
view. It cannot change kernel modules, tunables, logs, clocks, control groups,
the hostname, namespaces, real-time scheduling, or SUID/SGID bits. It also
cannot create writable-and-executable memory mappings and is restricted to the
native system-call architecture.

These restrictions fit the daemon because it reads one configuration file,
opens one UDP socket, reads interface metadata, logs to standard error, and
keeps no persistent state.

### Address families

`RestrictAddressFamilies` permits:

- `AF_INET` for DHCPv4;
- `AF_NETLINK` for Linux interface and address discovery through
  `getifaddrs`; and
- `AF_UNIX` for normal local runtime and library integration.

Do not remove `AF_NETLINK`: static `--check` would still succeed, but runtime
interface validation could fail inside the sandbox.

### Restart behavior

`Restart=on-failure` restarts the daemon after an unexpected runtime failure,
but not after a clean SIGTERM from `systemctl stop`. systemd's normal start-rate
limit is made explicit: at most five starts in 60 seconds, separated by five
seconds. This prevents an invalid configuration or unavailable interface from
causing an unbounded rapid restart loop. Fix the cause and use
`systemctl reset-failed` before starting again if the limit was reached.

## 7. Verify and start the service

Ask systemd to parse the installed unit:

```bash
sudo systemd-analyze verify /etc/systemd/system/monitor-dhcp.service
```

The command may report problems in unrelated installed units; look
specifically for messages naming `monitor-dhcp.service`.

Start the service and enable it for subsequent boots:

```bash
sudo systemctl enable --now monitor-dhcp.service
systemctl status monitor-dhcp.service --no-pager
journalctl -u monitor-dhcp.service -n 50 --no-pager
```

Expected journal output includes the static validation success message followed
by a startup line naming the configured interface and server address.

Confirm the effective process identity, capabilities, and DHCP socket:

```bash
systemctl show monitor-dhcp.service \
  -p User -p Group -p MainPID -p AmbientCapabilities \
  -p CapabilityBoundingSet -p NoNewPrivileges
sudo ss -ulpn 'sport = :67'
```

Optionally inspect systemd's heuristic security report:

```bash
systemd-analyze security monitor-dhcp.service
```

Treat the score as review assistance, not a pass/fail test. Some restrictions
are deliberately unavailable because this is a network service. In particular,
`PrivateNetwork=true` would isolate the daemon from the monitor interface and
must not be added.

## 8. Operate and reload it

Follow transactions:

```bash
journalctl -u monitor-dhcp.service -f
```

After changing reservations, the default address, subnet mask, or lease time,
run a manual check and request a reload:

```bash
sudo -u monitor-dhcp /usr/local/bin/monitor-dhcp \
  --config /etc/monitor-dhcp/display-wall.toml --check
sudo systemctl reload monitor-dhcp.service
journalctl -u monitor-dhcp.service -n 20 --no-pager
```

The daemon loads the file in response to SIGHUP. An invalid replacement is
rejected and the previous in-memory configuration remains active. Confirm the
`Configuration reloaded` journal entry; `systemctl reload` returning does not
guarantee that a semantically incompatible change was accepted.

Changes to `interface` or `server_ip` require a restart because the socket or
network identity changes:

```bash
sudo systemctl restart monitor-dhcp.service
```

Coordinate an address change with Netplan or the relevant network manager so
the new address is already present when the process starts.

Set debug logging with another drop-in when diagnosing protocol behavior:

```bash
sudo systemctl edit monitor-dhcp.service
```

Add or update:

```ini
[Service]
Environment="RUST_LOG=debug"
```

Then restart the service. Restore `RUST_LOG=info` afterward; transaction-level
debug logging can be noisy.

## 9. Troubleshoot startup failures

Always begin with the unit status and current-boot journal:

```bash
systemctl status monitor-dhcp.service --no-pager
journalctl -b -u monitor-dhcp.service --no-pager
systemctl cat monitor-dhcp.service
```

Common causes are:

- **The configuration cannot be read:** verify the selected path, directory
  traversal permissions, and `root:monitor-dhcp` ownership with mode `0640`.
- **The interface does not exist:** check the TOML name with `ip -br link` and
  ensure a USB adapter has appeared or the bridge was created by Netplan.
- **The server address is not assigned:** compare the TOML values with
  `ip -4 address show dev <interface>`. Put a bridge address on the bridge, not
  on a member port.
- **UDP port 67 is already occupied:** inspect `sudo ss -ulpn 'sport = :67'`
  and disable the competing DHCP server on this interface.
- **The unit reports an execution error:** confirm that
  `/usr/local/bin/monitor-dhcp` exists, is executable, and matches the host
  architecture.
- **The unit reports a user or group error:** recreate the static service
  account and group, then retry.
- **The unit reached its start limit:** correct the underlying error, run
  `sudo systemctl reset-failed monitor-dhcp.service`, and start it again.

Do not work around permission errors by changing `User=root`, granting
`CAP_NET_ADMIN`, disabling all hardening, or making the configuration
world-writable. Those changes are unnecessary for the supported design.

## 10. Upgrade or remove the service

For an upgrade, install the new executable and unit, reload systemd, validate,
and restart:

```bash
sudo install -o root -g root -m 0755 \
  target/release/monitor-dhcp /usr/local/bin/monitor-dhcp
sudo install -o root -g root -m 0644 packaging/monitor-dhcp.service \
  /etc/systemd/system/monitor-dhcp.service
sudo systemctl daemon-reload
sudo -u monitor-dhcp /usr/local/bin/monitor-dhcp \
  --config /etc/monitor-dhcp/display-wall.toml --check
sudo systemctl restart monitor-dhcp.service
```

The drop-in selecting `display-wall.toml` remains separate and is not replaced.

To remove the service, disable and stop it, then remove the installed unit,
drop-ins, executable, configuration, user, and group according to the site's
data-retention policy. Keep a copy of the TOML file if its MAC reservations are
needed for a later installation.
