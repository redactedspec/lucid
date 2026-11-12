#!/bin/sh
# Installs the latest Lucid release for Linux x86-64.
#
#   curl -fsSL https://lucidlang.dev/install.sh | sh
#
# It reads the release list at https://lucidlang.dev/releases.txt,
# downloads the archive over HTTPS, checks its SHA-256 against the list,
# and puts `lucid` in ~/.lucid/bin. It needs no root, runs nothing it
# downloads, and edits no shell profile. Run it again to update.
#
# LUCID_HOME changes the install directory from ~/.lucid.
# LUCID_RELEASES changes where the release list is read from.

set -eu

fail() {
    echo "lucid install: $*" >&2
    exit 1
}

main() {
    list_url="${LUCID_RELEASES:-https://lucidlang.dev/releases.txt}"
    home="${LUCID_HOME:-$HOME/.lucid}"
    bin="$home/bin"

    [ "$(uname -s)" = Linux ] || fail "this script is for Linux. On Windows, run: irm https://lucidlang.dev/install.ps1 | iex"
    [ "$(uname -m)" = x86_64 ] || fail "Lucid has binaries for x86-64 only, and this machine is $(uname -m)."
    command -v curl >/dev/null || fail "curl is needed."
    command -v tar >/dev/null || fail "tar is needed."
    if command -v sha256sum >/dev/null; then
        hash_cmd="sha256sum"
    elif command -v shasum >/dev/null; then
        hash_cmd="shasum -a 256"
    else
        fail "sha256sum or shasum is needed to check the download."
    fi

    # The binary is linked against glibc, and uses symbols up to 2.39.
    glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}') || true
    [ -n "$glibc" ] || fail "Lucid needs glibc 2.39 or newer, and this system has no glibc (musl?)."
    major=${glibc%%.*}
    minor=${glibc#*.}
    minor=${minor%%.*}
    if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 39 ]; }; then
        fail "Lucid needs glibc 2.39 or newer, and this system has $glibc. Build from source instead: https://lucidlang.dev/docs/installation/"
    fi

    list=$(curl -fsSL "$list_url") || fail "cannot read the release list at $list_url"
    line=$(printf '%s\n' "$list" | awk '$1 !~ /^#/ && $2 == "linux-x86_64" { print; exit }')
    [ -n "$line" ] || fail "the release list has no Linux x86-64 build."
    version=$(echo "$line" | awk '{print $1}')
    url=$(echo "$line" | awk '{print $3}')
    sum=$(echo "$line" | awk '{print $4}')
    case "$url" in
        https://*) ;;
        *) fail "the release list gives a download that is not HTTPS: $url" ;;
    esac
    printf '%s' "$sum" | grep -Eq '^[0-9a-f]{64}$' || fail "the release list gives a bad SHA-256 for $version."

    if [ -x "$bin/lucid" ] && [ "$("$bin/lucid" --version 2>/dev/null || true)" = "lucid $version" ]; then
        echo "Lucid $version is already installed in $bin."
        exit 0
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM
    echo "Downloading Lucid $version..."
    curl --proto '=https' --tlsv1.2 -fSL --progress-bar -o "$tmp/lucid.tar.gz" "$url" || fail "the download failed: $url"
    got=$($hash_cmd "$tmp/lucid.tar.gz" | awk '{print $1}')
    [ "$got" = "$sum" ] || fail "the download's SHA-256 is $got, but the release list says $sum. Nothing was installed."

    mkdir "$tmp/x"
    tar -xzf "$tmp/lucid.tar.gz" -C "$tmp/x" lucid || fail "the archive has no lucid binary."
    mkdir -p "$bin"
    cp "$tmp/x/lucid" "$bin/lucid.new"
    chmod 755 "$bin/lucid.new"
    mv -f "$bin/lucid.new" "$bin/lucid"

    echo "Installed $("$bin/lucid" --version) in $bin."
    case ":$PATH:" in
        *":$bin:"*) ;;
        *)
            echo
            echo "$bin is not on your PATH. To add it, put this line in your shell's profile:"
            echo
            echo "    export PATH=\"$bin:\$PATH\""
            ;;
    esac
    command -v cc >/dev/null || {
        echo
        echo "lucid build links through cc, which was not found. Install gcc or clang to build programs."
    }
}

main "$@"
