#!/bin/sh
set -eu

usage() {
    echo "usage: $0 [deb|rpm|all]" >&2
    exit 2
}

format=${1:-all}
case "$format" in
    deb|rpm|all) ;;
    *) usage ;;
esac
[ "$#" -le 1 ] || usage

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target=x86_64-unknown-linux-musl
binary="$repo_dir/target/$target/release/monitor-dhcp"
dist_dir="$repo_dir/dist"

cd "$repo_dir"
cargo build --release --locked --target "$target"
display_version=$($binary --version | awk '{print $2}')
version=${display_version#v}
case "$version" in
    ''|*[!0-9.]*) echo "invalid package version: $version" >&2; exit 1 ;;
esac

mkdir -p "$dist_dir"

build_deb() {
    command -v dpkg-deb >/dev/null 2>&1 || {
        echo "dpkg-deb is required to build the Debian package" >&2
        exit 1
    }
    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT HUP INT TERM
    chmod 0755 "$stage"
    install -d -m 0755 "$stage/DEBIAN" "$stage/usr/bin" \
        "$stage/lib/systemd/system" "$stage/usr/share/doc/monitor-dhcp"
    install -d -m 0750 "$stage/etc/monitor-dhcp"
    install -m 0755 "$binary" "$stage/usr/bin/monitor-dhcp"
    install -m 0644 packaging/monitor-dhcp.service \
        "$stage/lib/systemd/system/monitor-dhcp.service"
    install -m 0640 packaging/config.toml "$stage/etc/monitor-dhcp/config.toml"
    install -m 0644 README.md LICENSE "$stage/usr/share/doc/monitor-dhcp/"
    installed_size=$(du -sk "$stage" | awk '{print $1}')
    sed -e "s/@VERSION@/$version/" \
        -e "s/@INSTALLED_SIZE@/$installed_size/" \
        packaging/debian/control.in > "$stage/DEBIAN/control"
    install -m 0644 packaging/debian/conffiles "$stage/DEBIAN/conffiles"
    install -m 0755 packaging/debian/postinst packaging/debian/prerm \
        packaging/debian/postrm "$stage/DEBIAN/"
    dpkg-deb --root-owner-group --build "$stage" \
        "$dist_dir/monitor-dhcp_${version}_amd64.deb"
    (cd "$dist_dir" && sha256sum "monitor-dhcp_${version}_amd64.deb" > \
        "monitor-dhcp_${version}_amd64.deb.sha256")
    rm -rf "$stage"
    trap - EXIT HUP INT TERM
}

build_rpm() {
    command -v rpmbuild >/dev/null 2>&1 || {
        echo "rpmbuild is required to build the RPM package" >&2
        exit 1
    }
    topdir=$(mktemp -d)
    trap 'rm -rf "$topdir"' EXIT HUP INT TERM
    mkdir -p "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" \
        "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"
    install -m 0755 "$binary" "$topdir/SOURCES/monitor-dhcp"
    install -m 0644 packaging/monitor-dhcp.service packaging/config.toml \
        README.md LICENSE "$topdir/SOURCES/"
    install -m 0644 packaging/rpm/monitor-dhcp.spec "$topdir/SPECS/"
    rpmbuild -bb --define "_topdir $topdir" \
        --define "_unitdir /usr/lib/systemd/system" \
        --define "package_version $version" \
        "$topdir/SPECS/monitor-dhcp.spec"
    rpm_path=$(find "$topdir/RPMS" -type f -name '*.rpm' | head -n 1)
    [ -n "$rpm_path" ] || { echo "rpmbuild produced no RPM" >&2; exit 1; }
    rpm_name=$(basename "$rpm_path")
    cp "$rpm_path" "$dist_dir/$rpm_name"
    (cd "$dist_dir" && sha256sum "$rpm_name" > "$rpm_name.sha256")
    rm -rf "$topdir"
    trap - EXIT HUP INT TERM
}

case "$format" in
    deb) build_deb ;;
    rpm) build_rpm ;;
    all) build_deb; build_rpm ;;
esac

echo "Packages written to $dist_dir"
