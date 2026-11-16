#!/bin/sh
# Install weft for this user on Linux or macOS (no root needed).
#
#   ./install.sh                  Linux: ~/.local/opt/weft   macOS: ~/Applications/weft
#   ./install.sh /some/dir        somewhere else
#   ... desktop                   also a launcher on the desktop
#   ... path                      also `weft` on your PATH (a link in ~/.local/bin)
#
# weft is one program (shell scripts and the remote agent are built in); its
# data (settings, memos, history, logs) goes in data/ next to it. Run this
# again to update: a weft that is running goes on, and the new one starts the
# next time (or from its "restart" button). ./uninstall.sh removes it.
#
# Linux: the program, its icon and data/ in the folder; an entry in the
#   applications menu (~/.local/share/applications/weft.desktop).
# macOS: weft.app and data/ side by side in the folder (Launchpad and
#   Spotlight find weft.app there); the app keeps its data in that data/.
set -eu
cd "$(dirname "$0")"

# (WEFT_INSTALL_OS and WEFT_NO_BUILD are for tools/install_test.py.)
case "${WEFT_INSTALL_OS:-$(uname -s)}" in
    Darwin | macos) os=macos ;;
    *) os=linux ;;
esac
if [ "$os" = macos ]; then
    dest="$HOME/Applications/weft"
else
    dest="$HOME/.local/opt/weft"
fi
want_desktop=
want_path=
for a in "$@"; do
    case "$a" in
        desktop) want_desktop=1 ;;
        path) want_path=1 ;;
        *) dest=$a ;;
    esac
done
case "$dest" in
    /*) ;;
    *) dest="$(pwd)/$dest" ;;
esac

# Cargo is often in ~/.cargo/bin, which a shell may not have on its PATH.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
fi
# The program: built here when cargo is installed, else one built before.
if [ -z "${WEFT_NO_BUILD:-}" ] && [ -f Cargo.toml ] && command -v cargo >/dev/null 2>&1; then
    if ! cargo build --release; then
        echo "weft: the build failed, see above. Nothing was installed." >&2
        exit 1
    fi
elif [ -z "${WEFT_NO_BUILD:-}" ]; then
    echo "weft: cargo not found, so nothing was built; installing target/release/weft as it is."
fi
src=target/release/weft
if [ ! -f "$src" ]; then
    echo "weft: $src not found. Install Rust from https://rustup.rs and run this again." >&2
    exit 1
fi
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml 2>/dev/null | head -n 1)
version=${version:-0}
# A weft built before from older sources (cargo was not found, or the
# build was skipped) is not put in place as this version.
built=$("$src" --version 2>/dev/null | head -n 1)
case "$built" in
    "weft $version" | "weft $version "*) ;;
    "weft "*)
        echo "weft: $src is ${built%% (*}, but these sources are weft $version: it was not built again." >&2
        echo "      Make cargo findable (https://rustup.rs; it is in ~/.cargo/bin) and run this again." >&2
        echo "      Nothing was installed." >&2
        exit 1
        ;;
esac

echo "Installing weft $version into $dest"
mkdir -p "$dest"

# A file put in place by a rename: a weft running from there goes on with
# the one it started with.
put() { # put SOURCE TARGET MODE
    cp "$1" "$2.new.$$"
    chmod "$3" "$2.new.$$"
    mv -f "$2.new.$$" "$2"
}
# What this install made outside the folder, for uninstall.sh to take away.
made="$dest/.installed"
: >"$made.new.$$"
note() { echo "$1" >>"$made.new.$$"; }

if [ "$os" = macos ]; then
    app="$dest/weft.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    put "$src" "$app/Contents/MacOS/weft-bin" 755
    # The app's own program is this launcher: the data is in data/ next to
    # weft.app (inside the app it would be hard to find, and lost when the
    # app is replaced).
    cat >"$app/Contents/MacOS/weft.new.$$" <<'EOF'
#!/bin/sh
here=$(cd "$(dirname "$0")" && pwd)
WEFT_DATA_DIR="$(cd "$here/../../.." && pwd)/data"
export WEFT_DATA_DIR
exec "$here/weft-bin" "$@"
EOF
    chmod 755 "$app/Contents/MacOS/weft.new.$$"
    mv -f "$app/Contents/MacOS/weft.new.$$" "$app/Contents/MacOS/weft"
    cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>weft</string>
    <key>CFBundleDisplayName</key><string>weft</string>
    <key>CFBundleIdentifier</key><string>io.github.heeuid.weft</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>weft</string>
    <key>CFBundleIconFile</key><string>weft</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
    # The icon, made from assets/weft.png with macOS's own tools.
    if command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1 && [ -f assets/weft.png ]; then
        set_dir="${TMPDIR:-/tmp}/weft.$$.iconset"
        mkdir -p "$set_dir"
        for s in 16 32 64 128 256; do
            sips -z "$s" "$s" assets/weft.png --out "$set_dir/icon_${s}x${s}.png" >/dev/null
        done
        for s in 16 32 64 128; do
            cp "$set_dir/icon_$((s * 2))x$((s * 2)).png" "$set_dir/icon_${s}x${s}@2x.png"
        done
        iconutil -c icns "$set_dir" -o "$app/Contents/Resources/weft.icns" && echo "  app icon"
        rm -rf "$set_dir"
    fi
    # Built here, the app carries no "downloaded" mark and opens without a
    # warning; a weft brought from elsewhere might: take the mark off.
    if command -v xattr >/dev/null 2>&1; then
        xattr -dr com.apple.quarantine "$app" 2>/dev/null || true
    fi
    echo "  weft.app (Launchpad, Spotlight, or Finder > $dest)"
    launcher="$app/Contents/MacOS/weft"
    if [ -n "$want_desktop" ]; then
        mkdir -p "$HOME/Desktop"
        ln -sfn "$app" "$HOME/Desktop/weft.app"
        note "$HOME/Desktop/weft.app"
        echo "  on the desktop"
    fi
else
    put "$src" "$dest/weft" 755
    [ -f assets/weft.png ] && put assets/weft.png "$dest/weft.png" 644
    launcher="$dest/weft"
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    mkdir -p "$apps"
    entry() { # entry FILE
        cat >"$1" <<EOF
[Desktop Entry]
Type=Application
Name=weft
Comment=Terminal with GUI woven into the text grid
Exec="$dest/weft"
Icon=$dest/weft.png
Terminal=false
Categories=System;TerminalEmulator;
EOF
    }
    entry "$apps/weft.desktop"
    note "$apps/weft.desktop"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" >/dev/null 2>&1 || true
    echo "  applications menu entry"
    if [ -n "$want_desktop" ]; then
        desk=$(command -v xdg-user-dir >/dev/null 2>&1 && xdg-user-dir DESKTOP || echo "$HOME/Desktop")
        mkdir -p "$desk"
        entry "$desk/weft.desktop"
        chmod 755 "$desk/weft.desktop"
        # GNOME runs a desktop launcher only once it is marked trusted.
        command -v gio >/dev/null 2>&1 && gio set "$desk/weft.desktop" metadata::trusted true 2>/dev/null || true
        note "$desk/weft.desktop"
        echo "  on the desktop"
    fi
fi

if [ -n "$want_path" ]; then
    mkdir -p "$HOME/.local/bin"
    ln -sfn "$launcher" "$HOME/.local/bin/weft"
    note "$HOME/.local/bin/weft"
    case ":$PATH:" in
        *":$HOME/.local/bin:"*) echo "  on your PATH: weft" ;;
        *)
            # ~/.local/bin put on the PATH by your shell's own file, as
            # install.bat sets the PATH on Windows; uninstall.sh takes the
            # line out again (it is marked).
            mark="# added by weft install.sh"
            case "$(basename "${SHELL:-sh}")" in
                zsh) rc="$HOME/.zshrc" ;;
                bash) if [ "$os" = macos ]; then rc="$HOME/.bash_profile"; else rc="$HOME/.bashrc"; fi ;;
                fish) rc="$HOME/.config/fish/conf.d/weft.fish" ;;
                *) rc="$HOME/.profile" ;;
            esac
            if [ "$(basename "$rc")" = weft.fish ]; then
                line="fish_add_path \$HOME/.local/bin $mark"
            else
                line="export PATH=\"\$HOME/.local/bin:\$PATH\" $mark"
            fi
            mkdir -p "$(dirname "$rc")"
            if ! grep -qF "$mark" "$rc" 2>/dev/null; then
                printf '\n%s\n' "$line" >>"$rc"
            fi
            note "rc:$rc"
            echo "  on your PATH from new terminals: a line added to $rc:"
            echo "      $line"
            ;;
    esac
else
    echo "  (to run it as \`weft\` in a terminal too: ./install.sh path)"
fi

# Earlier installs' links are kept on the list until uninstall takes them.
if [ -f "$made" ]; then
    cat "$made" >>"$made.new.$$"
fi
sort -u "$made.new.$$" >"$made"
rm -f "$made.new.$$"

# Data from ./weft.sh (target/release/data), the first time only.
if [ ! -f "$dest/data/settings.json" ] && [ -f target/release/data/settings.json ]; then
    echo "  copying your data from target/release/data"
    mkdir -p "$dest/data"
    cp -R target/release/data/. "$dest/data/"
fi
[ -f uninstall.sh ] && [ "$(cd "$(dirname "$0")" && pwd)" != "$dest" ] && put uninstall.sh "$dest/uninstall.sh" 755

echo
echo "weft $version is installed in $dest. Remove it with $dest/uninstall.sh"
# A weft already running goes on as the version it started as.
if command -v pgrep >/dev/null 2>&1 && pgrep -f "$dest/(weft.app/Contents/MacOS/weft-bin|weft)( |$)" >/dev/null 2>&1; then
    echo "weft is running: quit it (all its windows; Cmd+Q on a Mac) and open it again to use $version."
fi
