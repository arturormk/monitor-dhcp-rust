#!/usr/bin/env bash
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "tests/netns.sh must run as root (use sudo)" >&2
    exit 1
fi

for command in ip dhclient ping python3; do
    command -v "$command" >/dev/null || { echo "missing required command: $command" >&2; exit 1; }
done

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
BIN=${MONITOR_DHCP_BIN:-"$ROOT_DIR/target/release/monitor-dhcp"}
[[ -x $BIN ]] || { echo "build first with: cargo build --release" >&2; exit 1; }

SUFFIX=$$
SERVER_NS="mdhcp-server-$SUFFIX"
CLIENT_NS="mdhcp-client-$SUFFIX"
OTHER_NS="mdhcp-other-$SUFFIX"
TMP_DIR=$(mktemp -d /tmp/monitor-dhcp-netns.XXXXXX)
CONFIG="$TMP_DIR/config.toml"
LOG="$TMP_DIR/server.log"
PID_FILE="$TMP_DIR/dhclient.pid"
LEASE_FILE="$TMP_DIR/dhclient.leases"
SERVER_PID=

cleanup() {
    if [[ -n ${SERVER_PID} ]]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    ip netns del "$CLIENT_NS" 2>/dev/null || true
    ip netns del "$OTHER_NS" 2>/dev/null || true
    ip netns del "$SERVER_NS" 2>/dev/null || true
    rm -r -- "$TMP_DIR"
}
trap cleanup EXIT

ip netns add "$SERVER_NS"
ip netns add "$CLIENT_NS"
ip netns add "$OTHER_NS"
ip link add srv0 type veth peer name mon0
ip link set srv0 netns "$SERVER_NS"
ip link set mon0 netns "$CLIENT_NS"
ip link add srv1 type veth peer name other0
ip link set srv1 netns "$SERVER_NS"
ip link set other0 netns "$OTHER_NS"

ip -n "$SERVER_NS" addr add 192.168.50.1/24 dev srv0
ip -n "$SERVER_NS" addr add 192.168.60.1/24 dev srv1
for item in "$SERVER_NS lo" "$SERVER_NS srv0" "$SERVER_NS srv1" "$CLIENT_NS lo" "$CLIENT_NS mon0" "$OTHER_NS lo" "$OTHER_NS other0"; do
    read -r namespace interface <<<"$item"
    ip -n "$namespace" link set "$interface" up
done
ip -n "$CLIENT_NS" link set mon0 address 02:11:22:33:44:55
ip -n "$OTHER_NS" link set other0 address 02:aa:bb:cc:dd:ee

write_config() {
    local assigned=$1
    printf '%s\n' \
        'interface = "srv0"' \
        'server_ip = "192.168.50.1"' \
        'subnet_mask = "255.255.255.0"' \
        'default_ip = "192.168.50.2"' \
        'lease_seconds = 120' \
        '[hosts]' \
        "\"02:11:22:33:44:55\" = \"$assigned\"" >"$CONFIG"
}

write_config 192.168.50.10
ip netns exec "$SERVER_NS" "$BIN" --config "$CONFIG" >"$LOG" 2>&1 &
SERVER_PID=$!
for _ in {1..40}; do
    grep -q 'DHCP server started' "$LOG" && break
    kill -0 "$SERVER_PID" 2>/dev/null || { cat "$LOG" >&2; exit 1; }
    sleep 0.1
done
grep -q 'DHCP server started' "$LOG"

# A real client completes discovery/request and installs the mapped address.
ip netns exec "$CLIENT_NS" dhclient -4 -1 -v -pf "$PID_FILE" -lf "$LEASE_FILE" mon0
ip -n "$CLIENT_NS" -4 addr show dev mon0 | grep -q '192.168.50.10/24'
ip netns exec "$CLIENT_NS" ping -c 1 -W 2 192.168.50.1 >/dev/null

# A direct unicast renewal receives an ACK.
ip netns exec "$CLIENT_NS" python3 "$ROOT_DIR/tests/netns_client.py" renew mon0 \
    --expect 192.168.50.10

# Invalid traffic does not terminate the server.
ip netns exec "$CLIENT_NS" python3 "$ROOT_DIR/tests/netns_client.py" invalid mon0
sleep 0.1
kill -0 "$SERVER_PID"

# A DISCOVER with the broadcast flag clear still receives the deterministic offer.
ip -n "$CLIENT_NS" addr flush dev mon0
ip netns exec "$CLIENT_NS" python3 "$ROOT_DIR/tests/netns_client.py" discover mon0 \
    --expect 192.168.50.10

# The second server-side interface is isolated by SO_BINDTODEVICE.
ip netns exec "$OTHER_NS" python3 "$ROOT_DIR/tests/netns_client.py" no-response other0

# Valid reload changes the assignment without restarting the process.
write_config 192.168.50.20
kill -HUP "$SERVER_PID"
for _ in {1..30}; do
    grep -q 'Configuration reloaded' "$LOG" && break
    sleep 0.1
done
ip netns exec "$CLIENT_NS" python3 "$ROOT_DIR/tests/netns_client.py" discover mon0 \
    --expect 192.168.50.20

# Invalid reload is rejected and the last valid assignment remains active.
printf '%s\n' 'not valid TOML = [' >"$CONFIG"
kill -HUP "$SERVER_PID"
for _ in {1..30}; do
    grep -q 'Configuration reload rejected' "$LOG" && break
    sleep 0.1
done
ip netns exec "$CLIENT_NS" python3 "$ROOT_DIR/tests/netns_client.py" discover mon0 \
    --expect 192.168.50.20

# Restore the valid config and use dhclient again to verify repeatability.
write_config 192.168.50.20
kill -HUP "$SERVER_PID"
sleep 0.6
ip netns exec "$CLIENT_NS" dhclient -4 -1 -v -pf "$PID_FILE" -lf "$LEASE_FILE" mon0
ip -n "$CLIENT_NS" -4 addr show dev mon0 | grep -q '192.168.50.20/24'

echo "network namespace integration tests passed"
