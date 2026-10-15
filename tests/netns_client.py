#!/usr/bin/env python3
"""Small dependency-free DHCP wire probe used by netns.sh."""

import argparse
import ipaddress
import socket
import struct
import sys

MAC = bytes.fromhex("021122334455")
XID = 0x4D444843
COOKIE = bytes((99, 130, 83, 99))


def packet(kind, ciaddr="0.0.0.0", requested=None, server=None, broadcast=False):
    flags = 0x8000 if broadcast else 0
    header = struct.pack(
        "!BBBBIHH4s4s4s4s16s64s128s",
        1,
        1,
        6,
        0,
        XID,
        0,
        flags,
        ipaddress.IPv4Address(ciaddr).packed,
        bytes(4),
        bytes(4),
        bytes(4),
        MAC + bytes(10),
        bytes(64),
        bytes(128),
    )
    options = bytearray(COOKIE + bytes((53, 1, kind)))
    if requested:
        options.extend(bytes((50, 4)) + ipaddress.IPv4Address(requested).packed)
    if server:
        options.extend(bytes((54, 4)) + ipaddress.IPv4Address(server).packed)
    options.append(255)
    return (header + options).ljust(300, b"\0")


def receive(interface, payload, destination, bind_ip="0.0.0.0", timeout=2.0):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, interface.encode() + b"\0")
    sock.bind((bind_ip, 68))
    sock.settimeout(timeout)
    sock.sendto(payload, (destination, 67))
    data, _ = sock.recvfrom(2048)
    if len(data) < 240 or struct.unpack("!I", data[4:8])[0] != XID:
        raise RuntimeError("received an invalid DHCP response")
    message_type = None
    offset = 240
    while offset < len(data):
        code = data[offset]
        offset += 1
        if code == 0:
            continue
        if code == 255:
            break
        length = data[offset]
        offset += 1
        value = data[offset : offset + length]
        offset += length
        if code == 53 and length == 1:
            message_type = value[0]
    return message_type, str(ipaddress.IPv4Address(data[16:20]))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("discover", "renew", "no-response", "invalid"))
    parser.add_argument("interface")
    parser.add_argument("--expect")
    parser.add_argument("--server", default="192.168.50.1")
    args = parser.parse_args()

    if args.mode == "invalid":
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_BINDTODEVICE, args.interface.encode() + b"\0")
        sock.sendto(b"invalid", ("255.255.255.255", 67))
        return

    payload = packet(1) if args.mode != "renew" else packet(3, ciaddr=args.expect)
    destination = args.server if args.mode == "renew" else "255.255.255.255"
    bind_ip = args.expect if args.mode == "renew" else "0.0.0.0"
    try:
        message_type, offered = receive(args.interface, payload, destination, bind_ip)
    except socket.timeout:
        if args.mode == "no-response":
            return
        raise RuntimeError("timed out waiting for DHCP response") from None
    if args.mode == "no-response":
        raise RuntimeError("unexpected DHCP response on isolated interface")
    expected_type = 5 if args.mode == "renew" else 2
    if message_type != expected_type:
        raise RuntimeError(f"expected message type {expected_type}, got {message_type}")
    if args.expect and offered not in (args.expect, "0.0.0.0"):
        raise RuntimeError(f"expected {args.expect}, got {offered}")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"netns_client.py: {error}", file=sys.stderr)
        sys.exit(1)
