#!/usr/bin/env bash
set -u

PROJECT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
PROGRAM=monitor-dhcp

die() {
    echo "quickstart.sh: $*" >&2
    exit 1
}

command -v cargo >/dev/null 2>&1 || die "cargo is required; install Rust with rustup first"
command -v rustc >/dev/null 2>&1 || die "rustc is required; install Rust with rustup first"

install_target() {
    local target=$1
    if rustup target list --installed 2>/dev/null | grep -Fxq "$target"; then
        return 0
    fi
    command -v rustup >/dev/null 2>&1 || return 1
    echo "Installing Rust target $target..."
    rustup target add "$target"
}

echo "Create:"
echo "  1) Executable"
echo "  2) Debian package (.deb, amd64)"
echo "  3) RPM package (.rpm, x86_64)"
if ! read -r -p "Selection [1, executable]: " artifact_choice; then
    artifact_choice=""
fi
artifact_choice=${artifact_choice:-1}

case "${artifact_choice,,}" in
    1 | executable | binary)
        artifact=executable
        ;;
    2 | deb | debian | .deb)
        artifact=deb
        ;;
    3 | rpm | .rpm)
        artifact=rpm
        ;;
    *)
        die "invalid selection '$artifact_choice'; choose 1, 2, or 3"
        ;;
esac

cd "$PROJECT_DIR" || die "cannot enter project directory $PROJECT_DIR"

if [[ $artifact != executable ]]; then
    if [[ $artifact == deb ]]; then
        command -v dpkg-deb >/dev/null 2>&1 || \
            die "dpkg-deb is required to create a Debian package"
    else
        command -v rpmbuild >/dev/null 2>&1 || \
            die "rpmbuild is required to create an RPM package"
    fi

    package_target=x86_64-unknown-linux-musl
    install_target "$package_target" || \
        die "could not install Rust target $package_target"
    "$PROJECT_DIR/packaging/build-packages.sh" "$artifact" || \
        die "$artifact package build failed"
    echo "Run quickstart.sh again to create another artifact type."
    exit 0
fi

host_machine=$(uname -m)
case "$host_machine" in
    x86_64 | amd64)
        default_choice=1
        default_name="x86_64"
        ;;
    armv6l)
        default_choice=2
        default_name="ARMv6"
        ;;
    armv7l | armv8l)
        default_choice=3
        default_name="ARMv7"
        ;;
    *)
        default_choice=1
        default_name="x86_64 (the current platform '$host_machine' is not one of the supported choices)"
        ;;
esac

echo "Build $PROGRAM for:"
echo "  1) x86_64"
echo "  2) ARMv6 (hard-float)"
echo "  3) ARMv7 (hard-float)"
if ! read -r -p "Selection [${default_choice}, ${default_name}]: " choice; then
    choice=""
fi
choice=${choice:-$default_choice}

case "${choice,,}" in
    1 | x86_64 | amd64)
        architecture=x86_64
        static_target=x86_64-unknown-linux-musl
        fallback_target=x86_64-unknown-linux-gnu
        static_linker=""
        fallback_linker=""
        ;;
    2 | armv6 | armv6l)
        architecture=armv6
        static_target=arm-unknown-linux-musleabihf
        fallback_target=arm-unknown-linux-gnueabihf
        static_linker=arm-linux-musleabihf-gcc
        fallback_linker=arm-linux-gnueabihf-gcc
        ;;
    3 | armv7 | armv7l)
        architecture=armv7
        static_target=armv7-unknown-linux-musleabihf
        fallback_target=armv7-unknown-linux-gnueabihf
        static_linker=arm-linux-musleabihf-gcc
        fallback_linker=arm-linux-gnueabihf-gcc
        ;;
    *)
        die "invalid selection '$choice'; choose 1, 2, or 3"
        ;;
esac

build_with_cargo() {
    local target=$1
    local target_linker=$2
    if [[ -n $target_linker ]]; then
        command -v "$target_linker" >/dev/null 2>&1 || return 1
        local linker_variable="CARGO_TARGET_${target^^}_LINKER"
        linker_variable=${linker_variable//-/_}
        env "$linker_variable=$target_linker" cargo build --release --target "$target"
    else
        cargo build --release --target "$target"
    fi
}

echo "Attempting a static musl build for $architecture ($static_target)..."
static_build=false
if install_target "$static_target" && build_with_cargo "$static_target" "$static_linker"; then
    selected_target=$static_target
    static_build=true
elif command -v cross >/dev/null 2>&1 && cross build --release --target "$static_target"; then
    selected_target=$static_target
    static_build=true
fi

if [[ $static_build != true ]]; then
    echo >&2
    echo "A static musl build was not possible with the installed toolchain." >&2
    echo "Falling back to $fallback_target; this executable may require the target system's glibc." >&2
    install_target "$fallback_target" || die "could not install Rust target $fallback_target"
    if ! build_with_cargo "$fallback_target" "$fallback_linker"; then
        if [[ -n $fallback_linker ]]; then
            die "build failed; install 'cross' or the '$fallback_linker' cross compiler and try again"
        fi
        die "build failed for target $fallback_target"
    fi
    selected_target=$fallback_target
fi

executable="$PROJECT_DIR/target/$selected_target/release/$PROGRAM"
[[ -x $executable ]] || die "cargo reported success, but $executable was not created"

echo
echo "Build complete for $architecture."
echo "Executable: $executable"
if command -v readelf >/dev/null 2>&1; then
    if readelf -l "$executable" 2>/dev/null | grep -q 'INTERP'; then
        echo "Linking: dynamic (static linking was unavailable)"
    else
        echo "Linking: static (no runtime shared libraries required)"
    fi
elif [[ $static_build == true ]]; then
    echo "Linking: static musl target (no runtime shared libraries required)"
else
    echo "Linking: not verified (readelf is unavailable)"
fi
