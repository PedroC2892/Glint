#!/bin/sh
# Install Glint for the current user (no root needed).
#
#   ./install.sh              build (profile 'local'), then install binary, menu entry, icon suite and Super+Space shortcut
#   ./install.sh --uninstall  remove everything this script installed
#
# Paths follow the XDG Base Directory spec: $XDG_DATA_HOME (default
# ~/.local/share) for the menu entry and icon, ~/.local/bin for the binary.
set -eu

APP_ID=glint
REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TARGET_DIR=${CARGO_TARGET_DIR:-"$REPO_DIR/target"}
case $TARGET_DIR in
    /*) ;;
    *) TARGET_DIR="$PWD/$TARGET_DIR" ;;
esac
DATA_HOME=${XDG_DATA_HOME:-"$HOME/.local/share"}
case $DATA_HOME in /*) ;; *) DATA_HOME="$HOME/.local/share" ;; esac
CONFIG_HOME=${XDG_CONFIG_HOME:-"$HOME/.config"}
case $CONFIG_HOME in /*) ;; *) CONFIG_HOME="$HOME/.config" ;; esac
BIN_DIR="$HOME/.local/bin"
DESKTOP_FILE="$DATA_HOME/applications/$APP_ID.desktop"
AUTOSTART_DIR="$CONFIG_HOME/autostart"
AUTOSTART_FILE="$AUTOSTART_DIR/$APP_ID.desktop"
PNG_SIZES="16 24 32 48 64 128 256 512"

have() { command -v "$1" >/dev/null 2>&1; }

desktop_quote() {
    printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g')"
}

wait_for_daemon_stop() {
    sock="${XDG_RUNTIME_DIR:-}/glint/glint.sock"
    i=0
    while [ "$i" -lt 30 ]; do
        if [ -z "${XDG_RUNTIME_DIR:-}" ] || [ ! -e "$sock" ]; then
            if ! have pgrep || ! pgrep -x "$APP_ID" >/dev/null 2>&1; then
                break
            fi
        elif have python3; then
            if ! python3 -c 'import socket, sys; s = socket.socket(socket.AF_UNIX); s.settimeout(0.2); s.connect(sys.argv[1])' "$sock" 2>/dev/null; then
                break
            fi
        elif have pgrep; then
            if ! pgrep -x "$APP_ID" >/dev/null 2>&1; then
                break
            fi
        else
            if [ ! -e "$sock" ]; then
                break
            fi
        fi
        sleep 0.1
        i=$((i + 1))
    done
}

refresh_caches() {
    if have update-desktop-database; then
        update-desktop-database "$DATA_HOME/applications" >/dev/null 2>&1 || true
    fi
    for base in "$HOME/.icons/hicolor" "$DATA_HOME/icons/hicolor"; do
        if have gtk-update-icon-cache && [ -d "$base" ]; then
            gtk-update-icon-cache -f -t "$base" >/dev/null 2>&1 || true
        fi
    done
    if have kbuildsycoca6; then
        kbuildsycoca6 >/dev/null 2>&1 || true
    elif have kbuildsycoca5; then
        kbuildsycoca5 >/dev/null 2>&1 || true
    fi
}

if [ "${1:-}" = "--uninstall" ]; then
    "$BIN_DIR/$APP_ID" --quit >/dev/null 2>&1 || true
    wait_for_daemon_stop
    rm -f "$BIN_DIR/$APP_ID" "$DESKTOP_FILE" "$AUTOSTART_FILE" "$DATA_HOME/pixmaps/$APP_ID.png" "$DATA_HOME/pixmaps/$APP_ID.svg"
    for base in "$HOME/.icons/hicolor" "$DATA_HOME/icons/hicolor"; do
        rm -f "$base/scalable/apps/$APP_ID.svg"
        for s in $PNG_SIZES; do
            rm -f "$base/${s}x${s}/apps/$APP_ID.png"
        done
    done
    if have gsettings && gsettings list-schemas 2>/dev/null | grep -q "org.gnome.settings-daemon.plugins.media-keys"; then
        SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
        PATH_ENTRY="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/glint/"
        CUR=$(gsettings get "$SCHEMA" custom-keybindings 2>/dev/null || echo "")
        if echo "$CUR" | grep -q "$PATH_ENTRY"; then
            MOD=$(echo "$CUR" | sed "s#,'$PATH_ENTRY'##g; s#'$PATH_ENTRY',##g; s#'$PATH_ENTRY'##g")
            gsettings set "$SCHEMA" custom-keybindings "$MOD" 2>/dev/null || true
        fi
    fi
    refresh_caches
    echo "Glint removed. Your search history stays in $DATA_HOME/$APP_ID (delete it if you like)."
    exit 0
fi

if [ -n "${1:-}" ]; then
    echo "usage: $0 [--uninstall]" >&2
    exit 2
fi

# 1. Build (or reuse an existing local or release binary when cargo is missing).
if have cargo; then
    echo "Building Glint (local)…"
    cargo build --profile local --manifest-path "$REPO_DIR/Cargo.toml"
    BIN_SRC="$TARGET_DIR/local/$APP_ID"
elif [ -f "$TARGET_DIR/local/$APP_ID" ]; then
    BIN_SRC="$TARGET_DIR/local/$APP_ID"
elif [ -f "$TARGET_DIR/release/$APP_ID" ]; then
    BIN_SRC="$TARGET_DIR/release/$APP_ID"
else
    echo "cargo was not found and no built binary exists; install Rust from https://rustup.rs first." >&2
    exit 1
fi

# 2. Binary.
mkdir -p "$BIN_DIR"
install -m 755 "$BIN_SRC" "$BIN_DIR/$APP_ID"

# 3. Icon: SVG + PNG suite + pixmaps.
# We install to ~/.icons/hicolor (user icon theme), ~/.local/share/icons/hicolor (if writable),
# and ~/.local/share/pixmaps so that all desktop launchers find the icon immediately.
install_icons_to() {
    target_base="$1"
    svg_target="$target_base/scalable/apps/$APP_ID.svg"
    mkdir -p "$(dirname "$svg_target")" 2>/dev/null || return 0
    install -m 644 "$REPO_DIR/assets/$APP_ID.svg" "$svg_target" 2>/dev/null || true
    for s in $PNG_SIZES; do
        out="$target_base/${s}x${s}/apps/$APP_ID.png"
        mkdir -p "$(dirname "$out")" 2>/dev/null || continue
        if [ -f "$REPO_DIR/assets/icons/${s}x${s}.png" ]; then
            install -m 644 "$REPO_DIR/assets/icons/${s}x${s}.png" "$out" 2>/dev/null || true
        fi
    done
}

install_icons_to "$HOME/.icons/hicolor"
install_icons_to "$DATA_HOME/icons/hicolor"

mkdir -p "$DATA_HOME/pixmaps" 2>/dev/null || true
if [ -f "$REPO_DIR/assets/icons/256x256.png" ]; then
    install -m 644 "$REPO_DIR/assets/icons/256x256.png" "$DATA_HOME/pixmaps/$APP_ID.png" 2>/dev/null || true
fi
install -m 644 "$REPO_DIR/assets/$APP_ID.svg" "$DATA_HOME/pixmaps/$APP_ID.svg" 2>/dev/null || true

# 4. Menu entry.
#    X-KDE-Shortcuts asks Plasma for Meta+Space (Win+Space) when it is free.
EXEC_BIN=$(desktop_quote "$BIN_DIR/$APP_ID")
mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" <<EOF
[Desktop Entry]
Type=Application
Version=1.5
Name=Glint
GenericName=Spotlight Search Launcher
Comment=Spotlight-style search for apps, files, math, definitions and web
Exec=$EXEC_BIN
Icon=$APP_ID
Terminal=false
Categories=Utility;Core;System;FileTools;
Keywords=search;launcher;spotlight;find;calculator;notes;glint;files;apps;
StartupNotify=false
StartupWMClass=$APP_ID
X-KDE-Shortcuts=Meta+Space
Actions=Settings;

[Desktop Action Settings]
Name=Glint Settings
Exec=$EXEC_BIN --settings
EOF
chmod 644 "$DESKTOP_FILE"

# 5. Keyboard shortcut (Super+Space / Win+Space).
#    Plasma reads X-KDE-Shortcuts in the desktop entry above.
#    On GNOME desktops, configure custom shortcut if gsettings is available.
if have gsettings && gsettings list-schemas 2>/dev/null | grep -q "org.gnome.settings-daemon.plugins.media-keys"; then
    SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
    CURRENT_BINDINGS=$(gsettings get "$SCHEMA" custom-keybindings 2>/dev/null || echo "@as []")
    PATH_ENTRY="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/glint/"
    case "$CURRENT_BINDINGS" in
        *"$PATH_ENTRY"*) ;;
        *)
            if [ "$CURRENT_BINDINGS" = "@as []" ] || [ "$CURRENT_BINDINGS" = "[]" ]; then
                NEW_BINDINGS="['$PATH_ENTRY']"
            else
                NEW_BINDINGS=$(echo "$CURRENT_BINDINGS" | sed "s/]/, '$PATH_ENTRY']/")
            fi
            gsettings set "$SCHEMA" custom-keybindings "$NEW_BINDINGS" 2>/dev/null || true
            ;;
    esac
    BIND_SCHEMA="org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$PATH_ENTRY"
    gsettings set "$BIND_SCHEMA" name "Glint" 2>/dev/null || true
    gsettings set "$BIND_SCHEMA" command "$BIN_DIR/$APP_ID" 2>/dev/null || true
    gsettings set "$BIND_SCHEMA" binding "<Super>space" 2>/dev/null || true
    echo "Configured Super+Space (Win+Space) shortcut via GNOME settings."
fi

# 6. Autostart entry (daemon mode).
mkdir -p "$AUTOSTART_DIR"
cat > "$AUTOSTART_FILE" <<EOF
[Desktop Entry]
Type=Application
Name=Glint
Exec=$EXEC_BIN --daemon
NoDisplay=true
X-GNOME-Autostart-enabled=true
X-KDE-autostart-phase=2
EOF
chmod 644 "$AUTOSTART_FILE"

refresh_caches

# 7. Restart resident daemon.
"$BIN_DIR/$APP_ID" --quit >/dev/null 2>&1 || true
wait_for_daemon_stop
nohup "$BIN_DIR/$APP_ID" --daemon >/dev/null 2>&1 &

echo "Glint installed successfully!"
echo "- Executable: $BIN_DIR/$APP_ID"
echo "- Launcher entry: $DESKTOP_FILE"
echo "- Autostart: $AUTOSTART_FILE (resident daemon mode)"
echo "- Icon: $APP_ID (SVG & PNG suite installed)"
echo "- Shortcut: Win + Space (Super + Space)"
