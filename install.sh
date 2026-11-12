#!/bin/sh
# Install Lookout for the current user (no root needed).
#
#   ./install.sh              build, then install binary, menu entry and icon
#   ./install.sh --uninstall  remove everything this script installed
#
# Paths follow the XDG Base Directory spec: $XDG_DATA_HOME (default
# ~/.local/share) for the menu entry and icon, ~/.local/bin for the binary.
set -eu

APP_ID=lookout
REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
case $DATA_HOME in /*) ;; *) DATA_HOME="$HOME/.local/share" ;; esac
BIN_DIR="$HOME/.local/bin"
DESKTOP_FILE="$DATA_HOME/applications/$APP_ID.desktop"
ICON_DIR="$DATA_HOME/icons/hicolor"
SVG_ICON="$ICON_DIR/scalable/apps/$APP_ID.svg"
PNG_SIZES="48 128 256"

have() { command -v "$1" >/dev/null 2>&1; }

refresh_caches() {
    if have update-desktop-database; then
        update-desktop-database "$DATA_HOME/applications" >/dev/null 2>&1 || true
    fi
    if have gtk-update-icon-cache && [ -f "$ICON_DIR/index.theme" ]; then
        gtk-update-icon-cache -f -t "$ICON_DIR" >/dev/null 2>&1 || true
    fi
    if have kbuildsycoca6; then
        kbuildsycoca6 >/dev/null 2>&1 || true
    elif have kbuildsycoca5; then
        kbuildsycoca5 >/dev/null 2>&1 || true
    fi
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN_DIR/$APP_ID" "$DESKTOP_FILE" "$SVG_ICON"
    for s in $PNG_SIZES; do
        rm -f "$ICON_DIR/${s}x${s}/apps/$APP_ID.png"
    done
    refresh_caches
    echo "Lookout removed. Your search history stays in $DATA_HOME/$APP_ID (delete it if you like)."
    exit 0
fi

if [ -n "${1:-}" ]; then
    echo "usage: $0 [--uninstall]" >&2
    exit 2
fi

# 1. Build (or reuse an existing release binary when cargo is missing).
if have cargo; then
    echo "Building Lookout…"
    cargo build --release --manifest-path "$REPO_DIR/Cargo.toml"
elif [ ! -x "$REPO_DIR/target/release/$APP_ID" ]; then
    echo "cargo was not found and no built binary exists; install Rust from https://rustup.rs first." >&2
    exit 1
fi

# 2. Binary.
mkdir -p "$BIN_DIR"
install -m 755 "$REPO_DIR/target/release/$APP_ID" "$BIN_DIR/$APP_ID"

# 3. Icon: SVG always, PNG sizes when a converter is available.
mkdir -p "$(dirname "$SVG_ICON")"
install -m 644 "$REPO_DIR/assets/$APP_ID.svg" "$SVG_ICON"
for s in $PNG_SIZES; do
    out="$ICON_DIR/${s}x${s}/apps/$APP_ID.png"
    mkdir -p "$(dirname "$out")"
    if have rsvg-convert; then
        rsvg-convert -w "$s" -h "$s" "$REPO_DIR/assets/$APP_ID.svg" -o "$out" 2>/dev/null || rm -f "$out"
    elif have inkscape; then
        inkscape "$REPO_DIR/assets/$APP_ID.svg" -w "$s" -h "$s" -o "$out" >/dev/null 2>&1 || rm -f "$out"
    elif have magick; then
        magick -background none "$REPO_DIR/assets/$APP_ID.svg" -resize "${s}x${s}" "$out" 2>/dev/null || rm -f "$out"
    fi
done

# 4. Menu entry. X-KDE-Shortcuts asks Plasma for Meta+Space when it is free;
#    other desktops ignore it.
mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Version=1.5
Name=Lookout
GenericName=Search Launcher
Comment=Find apps, files and settings, do maths and conversions
Exec="$BIN_DIR/$APP_ID"
Icon=$APP_ID
Terminal=false
Categories=Utility;Core;
Keywords=search;launcher;find;calculator;
StartupNotify=false
StartupWMClass=$APP_ID
X-KDE-Shortcuts=Meta+Space
EOF
chmod 644 "$DESKTOP_FILE"

refresh_caches

echo "Lookout installed. Find it in your applications menu."
echo "Tip: bind a keyboard shortcut (for example Super+Space) to: $BIN_DIR/$APP_ID"
