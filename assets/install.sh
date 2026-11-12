#!/bin/sh
# Installs the latest Flora release binary for Linux or macOS from GitHub Releases.
set -eu

releases='https://github.com/Hyphonical/Flora/releases/latest/download'

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) echo 'Unsupported operating system' >&2; exit 1 ;;
esac

case "$(uname -m)" in
    x86_64|amd64) arch=x86_64 ;;
    arm64|aarch64) arch=aarch64 ;;
    *) echo 'Unsupported architecture' >&2; exit 1 ;;
esac

mkdir -p "$HOME/.local/bin"
tmp=$(mktemp "$HOME/.local/bin/.flora.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM
curl -fsSL "$releases/flora-$os-$arch" -o "$tmp"
chmod 755 "$tmp"
mv -f "$tmp" "$HOME/.local/bin/flora"
printf 'Installed %s\n' "$HOME/.local/bin/flora" >&2
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) echo 'Add $HOME/.local/bin to PATH.' >&2 ;;
esac
