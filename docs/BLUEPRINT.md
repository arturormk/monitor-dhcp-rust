# BLUEPRINT.md — Minimal Rust DHCP Server

## 1. Project overview

Implement a small, deterministic DHCPv4 server in Rust for Linux.

The server's sole purpose is to assign predictable IPv4 addresses to devices, primarily commercial monitors, connected to a dedicated Ethernet interface on a PC.

Some monitors support DHCP but do not allow static IP configuration. This program makes them accessible at known addresses without requiring a conventional DHCP server.

**Design philosophy: Do the smallest thing that works reliably.**

This is not intended to be a general-purpose DHCP server.

Target platforms:

- Ubuntu 24.04 LTS
- Ubuntu 26.04 LTS
- Other reasonably modern Linux distributions

The application should be easy to compile, inspect, deploy, and troubleshoot.

## 2. Functional requirements

The server must:

1. Listen for DHCPv4 requests on one explicitly configured Ethernet interface.
2. Assign a configurable default IP address to any unknown MAC address.
3. Optionally assign specific IP addresses to specific MAC addresses.
4. Respond correctly to initial DHCP discovery and address requests.
5. Respond to DHCP lease renewal and rebinding requests.
6. Handle clients that reboot and attempt to reuse a previously assigned address.
7. Support configuration reload without restarting the process.
8. Log significant DHCP activity.
9. Run as a systemd service.
10. Remain completely isolated from the PC's other network interfaces.

It must not require a database, persistent lease file, separate management service, or external DHCP daemon.

### Explicit non-goals

Do not implement:

- Dynamic address pools
- Complex lease allocation
- IPv6 or DHCPv6
- DHCP relay support
- PXE/network boot
- DNS management
- Dynamic DNS updates
- Network address translation
- Routing
- Web interface
- REST API
- Authentication
- DHCPFORCERENEW
- Multithreading or asynchronous runtime
- Persistent client state

Avoid adding functionality not required by this blueprint.

## 3. Configuration

Use a TOML configuration file.

Default path:

`/etc/monitor-dhcp/config.toml`

Example:

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

### Address selection

For every DHCP client:

1. Extract the client's Ethernet MAC address from the DHCP packet.
2. Normalize the MAC address for comparison.
3. Look for that MAC in `[hosts]`.
4. If found, use its configured IP address.
5. Otherwise, use `default_ip`.

This lookup must be deterministic.

The same MAC must always receive the same configured address.

### Permissive mode

**Permissive mode is mandatory and is the only mode required.**

Unknown MAC addresses always receive `default_ip`.

There is no MAC whitelist or strict mode.

The server is intended for a dedicated interface containing a single monitor. The possibility of multiple unknown clients receiving the same default IP is an accepted limitation.

Do not implement dynamic collision avoidance or an address pool.

### Configuration validation

At startup, validate:

- Interface exists.
- Interface is Ethernet-compatible.
- Server IP is valid and assigned to the specified interface.
- Subnet mask is valid and contiguous.
- Default and reserved IPs are usable unicast addresses in the configured subnet.
- No client address equals the server address.
- No duplicate MAC keys after normalization.
- No duplicate IP assignments among explicit MAC mappings.
- Lease duration is a positive, representable DHCP lease interval.
- MAC addresses are valid six-byte Ethernet addresses.
- Configured interface name is valid for Linux.

A reserved IP may equal the default IP because the interface is intended for a single active client. Document the resulting limitation.

Configuration errors must produce understandable diagnostic messages and a nonzero exit status.

Do not silently substitute defaults for invalid configuration.

## 4. DHCP protocol

Use DHCPv4 as defined in RFC 2131 and its relevant DHCP options from RFC 2132.

The server listens on UDP port 67 and communicates with clients on UDP port 68.

### 4.1 DHCPDISCOVER

When a valid DHCPDISCOVER is received:

1. Identify the client by its Ethernet MAC address.
2. Look up its configured IP address.
3. Construct a DHCPOFFER.
4. Set `yiaddr` to the assigned address.
5. Include the server identifier, subnet mask, and lease duration.
6. Return the offer using the appropriate DHCP delivery mechanism.

Do not create a lease record.

Repeated DISCOVER packets from the same client should receive the same offer.

### 4.2 DHCPREQUEST

Handle the principal DHCPREQUEST cases separately.

**SELECTING**

The client selects an address from a DHCP server's offer.

- If the request identifies another DHCP server, ignore it.
- If it identifies this server and requests its configured address, send DHCPACK.
- If it identifies this server but requests a different address, send DHCPNAK.

**INIT-REBOOT**

The client reboots and attempts to reuse a previously assigned address.

- If the requested IP equals the client's configured IP, send DHCPACK.
- If the requested IP differs, send DHCPNAK where the server is authoritative for the directly attached client.
- The client should then return to discovery and receive its current configured address.

**RENEWING**

The client is already using an address and requests a lease extension, usually by unicast.

- If `ciaddr` matches its configured address, send DHCPACK.
- If the address is no longer valid, send DHCPNAK when appropriate under RFC 2131.

**REBINDING**

The client broadcasts a request because its original server did not answer renewal.

- If its address matches the configured assignment, send DHCPACK.
- Otherwise send DHCPNAK when appropriate.

Distinguish these cases using `ciaddr`, the requested-IP option, and the server-identifier option.

Do not blindly ACK every DHCPREQUEST.

### 4.3 DHCPRELEASE

When a client releases its lease:

- Log the event.
- Do not remove or alter the configured address assignment.
- No response is required.

A subsequent DISCOVER from the same client still receives its configured address.

### 4.4 DHCPDECLINE

When a client reports an address conflict:

- Log a warning with the client's MAC address and reported IP.
- Do not allocate an alternative address.
- Do not permanently disable the configured assignment.

The application has no address pool and cannot automatically resolve an actual network conflict.

### 4.5 DHCPINFORM

Implement minimal DHCPINFORM support.

Respond with DHCPACK containing applicable network parameters.

Do not allocate an address, include a lease duration, or set `yiaddr` to a newly assigned address.

### 4.6 Unsupported messages

Ignore unsupported DHCP messages safely.

Malformed packets must never crash the server.

Reject or ignore invalid message formats, non-Ethernet hardware address types, and unsupported relay traffic.

Do not respond to BOOTP-only requests.

## 5. DHCP packet construction

Use the `dhcproto` Rust crate for packet parsing and encoding.

Do not manually implement DHCP binary encoding unless a specific library limitation requires it.

Outgoing DHCP responses must correctly populate:

- BOOTP reply opcode
- Transaction ID (`xid`) copied from the request
- Client hardware address (`chaddr`)
- Client address fields as appropriate
- Offered/client address (`yiaddr`) as appropriate
- Server address information
- DHCP message type (option 53)
- Server identifier (option 54)
- Subnet mask (option 1), where applicable
- Lease time (option 51), where applicable

For normal OFFER and address-assigning ACK packets, provide:

| DHCP option | Value |
|---|---|
| Message type (53) | OFFER or ACK |
| Server identifier (54) | `server_ip` |
| Subnet mask (1) | `subnet_mask` |
| Lease time (51) | `lease_seconds` |
| Renewal time (58) | Half the lease |
| Rebinding time (59) | Seven-eighths of the lease |

Validate lease values such that the renewal and rebinding values are meaningful and ordered.

Do not advertise a router (option 3) or DNS servers (option 6) by default.

These monitors are normally reachable directly from the local PC and do not require internet access.

Handle client parameter-request lists without introducing unnecessary DHCP options.

Follow RFC requirements for minimum packet lengths, option termination, and response construction.

## 6. DHCP response delivery

Correct packet delivery is essential.

Some clients can receive initial DHCP responses by IP broadcast. Others may expect link-layer unicast before their IP interface is configured.

### Initial implementation

Use a normal IPv4 UDP socket.

- Listen on port 67.
- Enable `SO_BROADCAST`.
- Bind the socket to the configured interface using `SO_BINDTODEVICE`.
- Receive DHCP traffic only from that interface.
- Send initial responses to client UDP port 68.
- Send ordinary renewal responses using the correct unicast destination.

Implement destination selection according to RFC 2131, including the request's broadcast flag and `ciaddr`.

For directly attached clients with `ciaddr = 0`, broadcast replies may be used as an initial compatibility strategy.

Document that this can be insufficient for clients expecting link-layer unicast despite not setting the broadcast flag.

### Compatibility requirement

Test clients that do not set the broadcast flag.

If required, support link-layer unicast using a narrowly scoped Linux `AF_PACKET` mechanism or another technically sound approach.

Do not introduce raw packet handling unless testing demonstrates that the standard UDP approach is insufficient.

However, the final implementation must not claim full compatibility with such clients unless this behavior has actually been verified.

Avoid adding a large networking framework solely to solve this problem.

### Interface isolation

The DHCP server must never serve the PC's other interfaces.

`SO_BINDTODEVICE` is mandatory.

Validate the interface at startup and fail closed if interface binding fails.

Do not silently fall back to listening on all interfaces.

The server must not enable IP forwarding or alter routing tables.

## 7. Linux interface configuration

The application's job is DHCP service, not interface management.

It must not assign, remove, or alter the PC's IP addresses.

The configured interface must already have its server IP address.

Example temporary setup:

```bash
sudo ip addr add 192.168.50.1/24 dev enp3s0
sudo ip link set enp3s0 up
```

Example resulting network:

```text
Linux PC
  enp3s0
  192.168.50.1/24
       |
       | Ethernet cable
       |
Monitor
  DHCP client
  192.168.50.2/24
```

The application should validate that the specified `server_ip` and subnet mask match an appropriate address/prefix on the interface.

If the interface or configured address is missing, exit with a useful error rather than attempting to repair the networking configuration.

Document persistent interface configuration on Ubuntu using NetworkManager or Netplan.

Explain how to disable conflicting DHCP server services on the dedicated interface.

## 8. Lease behavior

No lease database is needed.

The lease duration is configured with `lease_seconds`, defaulting to 86400 (24 hours).

Every successful DHCPACK effectively grants a fresh lease of that duration.

The server does not track lease expiration.

It does not need to know which clients currently have leases.

It does not persist client information between runs.

After restarting, it can immediately serve the same address assignments from the configuration file.

### Changing an assigned address

If an administrator changes the IP assigned to a MAC:

1. Reload the configuration.
2. Use the new assignment for all subsequent requests.
3. When the client attempts to validate or renew the old address, handle the request according to RFC 2131.
4. Where appropriate, DHCPNAK causes the client to restart address acquisition.

Do not attempt to push immediate address revocations to clients.

Do not implement DHCPFORCERENEW.

Document that immediate reassignment may require rebooting the monitor or restarting its network interface.

## 9. Configuration reload

Support `SIGHUP`.

On receiving SIGHUP:

1. Read the configuration file again.
2. Parse and validate the replacement configuration completely.
3. If valid, replace the active configuration atomically.
4. If invalid, log the error and continue using the previous valid configuration.

There must never be a partially applied configuration.

### Reload limitation

Changing address assignments, the default IP, subnet mask, or lease duration should be possible through SIGHUP, provided the configuration remains valid for the existing interface.

Changing `interface` or `server_ip` must require a full service restart.

If those values differ during reload, reject the reload and keep the current configuration.

Do not create a control socket or management API merely for reloading.

## 10. Logging

Log to standard error.

systemd will collect application output through journald.

Support conventional levels:

- Error
- Warning
- Info
- Debug

Default level: `info`.

Example:

```text
INFO  DHCP server started on enp3s0 (192.168.50.1)
INFO  DISCOVER mac=00:11:22:33:44:55
INFO  OFFER mac=00:11:22:33:44:55 ip=192.168.50.10
INFO  REQUEST mac=00:11:22:33:44:55 ip=192.168.50.10
INFO  ACK mac=00:11:22:33:44:55 ip=192.168.50.10
INFO  Configuration reloaded
WARN  DECLINE mac=00:11:22:33:44:55 ip=192.168.50.10
```

Include transaction identifiers in debug logs.

Malformed packets should be logged at debug level, with rate limiting or suppression if necessary to avoid flooding logs.

No log file management or rotation code is required.

## 11. Command-line interface

Suggested executable name:

`monitor-dhcp`

Supported commands:

```bash
monitor-dhcp --config /etc/monitor-dhcp/config.toml
monitor-dhcp --config ./test.toml --check
monitor-dhcp --help
monitor-dhcp --version
```

`--check` must validate the configuration without opening port 67 or starting the server.

It should perform configuration syntax and semantic checks, but need not require the configured network interface to exist. Clearly distinguish static validation from runtime interface validation.

Support log-level selection using either `RUST_LOG` or a simple CLI option.

Do not build a complex command-line framework.

## 12. Implementation architecture

Prefer a small, synchronous Rust program.

Suggested project structure:

```text
monitor-dhcp/
├── Cargo.toml
├── Cargo.lock
├── BLUEPRINT.md
├── README.md
├── config.example.toml
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── dhcp.rs
│   └── server.rs
├── tests/
│   └── protocol.rs
└── packaging/
    └── monitor-dhcp.service
```

Responsibilities:

**main.rs**

- Parse arguments.
- Load configuration.
- Initialize logging.
- Start server.
- Handle shutdown and SIGHUP.

**config.rs**

- Parse TOML.
- Validate settings.
- Normalize MAC addresses.
- Determine assigned IP for a client.
- Implement safe configuration reload.

**dhcp.rs**

- Decode packets.
- Classify DHCP messages and request states.
- Construct DHCP responses.
- Implement protocol decisions.

**server.rs**

- Open and configure network socket.
- Bind to Ethernet interface.
- Receive packets.
- Dispatch to DHCP logic.
- Send responses.
- Maintain the single-threaded event loop.

Keep packet processing separate from network I/O so protocol behavior can be unit-tested.

### Dependencies

Prefer:

- `dhcproto` — DHCPv4 packet encoding and decoding
- `socket2` — Socket configuration
- `serde` — Configuration deserialization
- `toml` — TOML parser
- `log` and a minimal logging implementation
- `libc` or `nix` — Linux signals and necessary OS integration

Other small dependencies are acceptable when justified.

Check the installed Rust toolchain against the minimum supported versions of selected crates.

Avoid:

- Tokio
- Async runtimes
- Web frameworks
- Database crates
- Large DHCP server frameworks

There should be no `unsafe` application code unless unavoidable for Linux OS interaction, and any such use must be small, documented, and encapsulated.

## 13. Runtime and signal handling

The main server loop should:

1. Wait for incoming DHCP packets.
2. Parse each packet safely.
3. Determine the client assignment.
4. Construct a protocol-appropriate response.
5. Send it through the configured interface.
6. Continue listening.

Use a simple mechanism to receive SIGHUP and termination signals without introducing complicated threading.

A Linux signal file descriptor with polling is acceptable.

Avoid signal handlers performing I/O, memory allocation, or configuration parsing.

Handle SIGTERM and SIGINT gracefully.

The service should stop promptly, close its socket, and exit cleanly.

There is no lease state to save.

## 14. Security and operational constraints

The server binds to privileged UDP port 67.

Minimize privileges.

Under systemd, use `CAP_NET_BIND_SERVICE` and any additional capabilities demonstrably needed for interface binding or raw packet handling.

Do not automatically require `CAP_NET_RAW` if no raw socket is used.

Prefer running under a dedicated unprivileged service account.

Systemd hardening should include reasonable protections such as:

- `NoNewPrivileges=true`
- `ProtectSystem=strict`
- `ProtectHome=true`
- `PrivateTmp=true`
- `RestrictAddressFamilies` limited to required families

Do not enable hardening settings that break the required networking behavior.

The program must:

- Fail if it cannot bind to the specified interface.
- Reject invalid configurations.
- Treat network packets as untrusted data.
- Never panic because of malformed DHCP packets.
- Avoid accepting relay-agent traffic.
- Limit unnecessary memory allocations.
- Remain responsive after repeated invalid packets.

No authentication or client filtering is required because the network is physically dedicated.

Do not interpret this as authorization to serve DHCP on arbitrary interfaces.

## 15. systemd integration

Provide an example unit file.

Suggested service:

```ini
[Unit]
Description=Minimal DHCP server for dedicated monitor interface
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/monitor-dhcp --config /etc/monitor-dhcp/config.toml
ExecReload=/bin/kill -HUP $MAINPID
Restart=on-failure
RestartSec=2
User=monitor-dhcp
Group=monitor-dhcp
AmbientCapabilities=CAP_NET_BIND_SERVICE CAP_NET_RAW
CapabilityBoundingSet=CAP_NET_BIND_SERVICE CAP_NET_RAW
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

This is an illustrative unit, not a finalized security configuration.

In particular:

- Remove `CAP_NET_RAW` if it is unnecessary.
- Add only capabilities required by the chosen socket implementation.
- Verify binding to the interface under the configured service account.
- Ensure the interface and its server IP are actually configured before service startup.
- Consider a specific dependency on the interface configuration service rather than relying exclusively on `network-online.target`.

Document installation, enablement, restart, reload, and log inspection.

Example:

```bash
sudo systemctl enable --now monitor-dhcp
sudo systemctl reload monitor-dhcp
journalctl -u monitor-dhcp -f
```

## 16. Testing

Tests are mandatory.

### 16.1 Unit tests

Test configuration parsing:

- Valid configuration
- Missing required fields
- Invalid MAC
- Duplicate normalized MAC
- Duplicate explicit IP
- IP outside subnet
- Address equal to server IP
- Invalid subnet mask
- Invalid lease duration

Test address selection:

- Unknown MAC receives default IP
- Known MAC receives specific IP
- MAC comparison is case-insensitive
- Repeated lookup is deterministic

Test DHCP protocol logic:

- DISCOVER produces OFFER
- Correct REQUEST produces ACK
- Incorrect REQUEST produces NAK when appropriate
- REQUEST identifying another DHCP server is ignored
- INIT-REBOOT with correct IP produces ACK
- INIT-REBOOT with obsolete IP produces NAK
- RENEWING with correct IP produces ACK
- REBINDING with correct IP produces ACK
- RELEASE produces no response
- DECLINE produces no response
- INFORM produces a correctly formed ACK
- Malformed packets are ignored safely
- Relay traffic is ignored
- Transaction ID and client MAC are preserved

### 16.2 Integration testing

Create a Linux network namespace test using a virtual Ethernet (`veth`) pair.

One namespace represents the DHCP server interface.

The other represents the monitor.

Test a real DHCP client against the Rust server.

Verify:

1. Client discovers the DHCP server.
2. Server offers the correct IP.
3. Client receives the address and subnet mask.
4. Client can communicate with the server's IP.
5. Repeating DHCP negotiation gives the same address.
6. Renewal succeeds.
7. Changing a MAC assignment and reloading is reflected in subsequent protocol exchanges.
8. Invalid packets do not terminate the server.

Run packet captures where appropriate:

```bash
sudo tcpdump -ni <interface> 'udp port 67 or udp port 68'
```

Include a test for a client with the DHCP broadcast flag cleared.

If that test fails, document and address the packet-delivery problem rather than silently accepting it.

### 16.3 Interface-isolation test

Create two separate virtual interfaces.

Configure the DHCP server to serve one of them.

Send DHCPDISCOVER packets on both.

Verify:

- The configured interface receives DHCP responses.
- The other interface receives none.

This test is essential.

### 16.4 Configuration-reload test

While the server is running:

1. Modify a MAC-specific assignment.
2. Send SIGHUP.
3. Confirm new requests use the new assignment.
4. Replace configuration with invalid TOML.
5. Send SIGHUP again.
6. Confirm the server logs an error and continues using the previous valid configuration.

## 17. README requirements

Provide a useful README describing:

- Purpose of the application
- Installation and build instructions
- Example TOML configuration
- How to identify an Ethernet interface using `ip link`
- How to configure the PC's static server IP
- How to run the program manually
- How to install and start the systemd service
- How to inspect logs
- How to reload configuration
- How to troubleshoot DHCP using tcpdump
- How to test without physical hardware
- Known limitations

Document both Ubuntu 24.04 and 26.04.

Explain that the interface should be isolated and must not already be served by another DHCP server.

## 18. Acceptance criteria

The implementation is complete when:

- `cargo fmt --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo test` passes.
- The program builds in release mode.
- Invalid configuration is detected and reported clearly.
- The server binds only to the configured interface.
- A real DHCP client obtains the configured default IP.
- A client with a MAC-specific mapping obtains its mapped IP.
- Lease renewal works.
- Clients can reboot and reacquire their configured address.
- SIGHUP reload works without restarting.
- Invalid reloads preserve the previous configuration.
- Other Ethernet interfaces are unaffected.
- The application runs successfully as a systemd service.
- README and example configuration are complete.
- Any limitation concerning DHCP unicast/broadcast delivery is explicitly tested and documented.

Integration tests that require root or network namespace privileges may use a separate test script. They must not silently be omitted from the documented validation procedure.

## 19. Development guidance for Codex

Implement this project incrementally.

Suggested order:

1. Create Cargo project and configuration parser.
2. Implement configuration validation and deterministic MAC-to-IP lookup.
3. Implement DHCP packet parsing and response generation.
4. Implement UDP socket bound to a specific interface.
5. Implement the DHCP server loop.
6. Implement correct DHCPREQUEST state handling.
7. Add SIGHUP reload and signal handling.
8. Add unit tests.
9. Add network namespace integration tests.
10. Address any DHCP response delivery compatibility failures.
11. Add systemd service and documentation.
12. Run formatting, linting, tests, and release build.

Prefer simple, readable Rust.

Use standard library types wherever practical.

Do not add abstractions merely to anticipate requirements that are not in this blueprint.

Do not implement a general-purpose DHCP server.

The finished project should be easy for a competent Rust programmer to understand in a single sitting.

## 20. References

- RFC 2131 — Dynamic Host Configuration Protocol
  https://www.rfc-editor.org/rfc/rfc2131
- RFC 2132 — DHCP Options and BOOTP Vendor Extensions
  https://www.rfc-editor.org/rfc/rfc2132
- RFC 3203 — DHCP Reconfigure Extension (out of scope)
  https://www.rfc-editor.org/rfc/rfc3203
- dhcproto crate documentation
  https://docs.rs/dhcproto/
- socket2 crate documentation
  https://docs.rs/socket2/

## 21. Final principle

**The configuration file is the source of truth.**

The server does not manage a database of leases.

It does not decide which IP addresses are available.

It does not discover new network topology.

It simply determines the expected IPv4 address for a requesting MAC, then participates correctly in the DHCP protocol to give the client that address.

Keep it boring, deterministic, and reliable.
