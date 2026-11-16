#!/bin/sh
# Remove weft installed by install.sh: the program, its menu entry and the
# links it made. Your data (settings, memos, history, logs in data/) is kept
# unless you say to delete it.
#
#   ./uninstall.sh                Linux: ~/.local/opt/weft   macOS: ~/Applications/weft
#   ./uninstall.sh /some/dir      from where you installed it
#   ... purge                     also delete the data, without asking
#   ... keep                      keep the data, without asking
set -eu

case "${WEFT_INSTALL_OS:-$(uname -s)}" in
    Darwin | macos) os=macos ;;
    *) os=linux ;;
esac
here=$(cd "$(dirname "$0")" && pwd)
dest=
choice=
for a in "$@"; do
    case "$a" in
        purge) choice=delete ;;
        keep) choice=keep ;;
        *) dest=$a ;;
    esac
done
# Run from the installed folder: that folder.
if [ -z "$dest" ] && [ -f "$here/.installed" ]; then
    dest=$here
fi
if [ -z "$dest" ]; then
    if [ "$os" = macos ]; then dest="$HOME/Applications/weft"; else dest="$HOME/.local/opt/weft"; fi
fi
case "$dest" in
    /*) ;;
    *) dest="$(pwd)/$dest" ;;
esac
if [ "$os" = macos ]; then program="$dest/weft.app/Contents/MacOS/weft-bin"; else program="$dest/weft"; fi
# Installed there: the program, or (it was put in the Trash, or deleted,
# first) what the install left beside it, which is then cleared up.
if [ ! -f "$program" ] && [ ! -f "$dest/.installed" ] && [ ! -d "$dest/data" ]; then
    echo "weft: not installed in $dest (give the folder: ./uninstall.sh /path/to/weft)" >&2
    exit 1
fi

# weft is closed first (a running one would write its data back).
if command -v pgrep >/dev/null 2>&1; then
    while pgrep -f "$program" >/dev/null 2>&1; do
        printf 'weft is running. Close every weft window, then press Enter (Ctrl+C to stop) '
        read -r _ || exit 1
    done
fi

if [ -z "$choice" ] && [ -d "$dest/data" ]; then
    echo "Your weft data is in $dest/data (settings, memos, history, logs)."
    printf 'Delete it too? [y/N] '
    read -r answer || answer=
    case "$answer" in
        y | Y | yes | YES) choice=delete ;;
        *) choice=keep ;;
    esac
fi

# What the install made outside the folder: only what is still its own (a
# link to this weft, or a launcher that runs it).
if [ -f "$dest/.installed" ]; then
    while IFS= read -r f; do
        [ -n "$f" ] || continue
        case "$f" in
            rc:*)
                # The PATH line install.sh added to a shell's file: that
                # line only (a file of weft's own goes whole).
                rc=${f#rc:}
                mark="# added by weft install.sh"
                if [ "$(basename "$rc")" = weft.fish ]; then
                    rm -f "$rc" && echo "  removed $rc"
                elif [ -f "$rc" ] && grep -qF "$mark" "$rc"; then
                    grep -vF "$mark" "$rc" >"$rc.weft.$$" || true
                    cat "$rc.weft.$$" >"$rc"
                    rm -f "$rc.weft.$$"
                    echo "  took weft's PATH line out of $rc"
                fi
                continue
                ;;
        esac
        if [ -L "$f" ]; then
            case "$(readlink "$f")" in
                "$dest"/*) rm -f "$f" && echo "  removed $f" ;;
            esac
        elif [ -f "$f" ] && grep -q "Exec=\"$dest/weft\"" "$f" 2>/dev/null; then
            rm -f "$f" && echo "  removed $f"
        fi
    done <"$dest/.installed"
fi
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
command -v update-desktop-database >/dev/null 2>&1 && [ -d "$apps" ] && update-desktop-database "$apps" >/dev/null 2>&1 || true

if [ "$os" = macos ]; then
    rm -rf "$dest/weft.app"
else
    rm -f "$dest/weft" "$dest/weft.png"
fi
rm -f "$dest/.installed" "$dest/uninstall.sh"
if [ "$choice" = delete ]; then
    rm -rf "$dest/data"
    echo "  data deleted"
elif [ -d "$dest/data" ]; then
    echo "  data kept in $dest/data (installing again picks it up)"
fi
rmdir "$dest" 2>/dev/null || true
echo "weft is removed."
