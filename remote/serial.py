"""weft serial console bridge - python standard library only.

Run by weft over `ssh -tt HOST` as `python3 -c 'exec(base64 of this)' PORT BAUD`:
it lives only in memory, nothing is written to the remote disk. It connects
the terminal (ssh's pty, put in raw mode) to the serial port (raw, at BAUD)
both ways until Ctrl+] is pressed or the port goes away.

Before opening, it says who else has the port open (two readers split the
data between them and both see garbage) and asks before going on.
"""

import glob
import errno
import os
import select
import sys
import termios
import tty

QUIT = b"\x1d"  # Ctrl+]


def holders(dev):
    """(pid, user, command) of other processes with `dev` open, and lock files."""
    real = os.path.realpath(dev)
    found = set()
    for fd in glob.glob("/proc/[0-9]*/fd/*"):
        try:
            if os.path.realpath(fd) != real:
                continue
            pid = fd.split("/")[2]
            if int(pid) == os.getpid():
                continue
            with open("/proc/%s/cmdline" % pid, "rb") as f:
                cmd = f.read().replace(b"\0", b" ").decode("utf-8", "replace").strip()
            uid = os.stat("/proc/%s" % pid).st_uid
            try:
                import pwd
                user = pwd.getpwuid(uid).pw_name
            except Exception:
                user = str(uid)
            found.add((pid, user, cmd))
        except (OSError, ValueError):
            pass
    # minicom, screen and friends leave a lock file with their pid.
    for lock in ("/var/lock/LCK..%s" % os.path.basename(real), "/run/lock/LCK..%s" % os.path.basename(real)):
        try:
            with open(lock) as f:
                pid = f.read().strip().split()[0]
            if os.path.exists("/proc/%s" % pid):
                found.add((pid, "?", "(lock file %s)" % lock))
        except (OSError, IndexError):
            pass
    return sorted(found)


def say(text):
    os.write(1, text.replace("\n", "\r\n").encode("utf-8", "replace"))


def main():
    dev, baud = sys.argv[1], int(sys.argv[2])
    others = holders(dev)
    if others:
        say("\x1b[33m%s is already open:\x1b[0m\n" % dev)
        for pid, user, cmd in others:
            say("  pid %-7s %-10s %s\n" % (pid, user, cmd[:150]))
        if os.getuid() != 0:
            say("(processes of other users show only to root)\n")
        say("Two readers split the data and both see garbage. Open anyway? [y/N] ")
        answer = sys.stdin.readline().strip().lower()
        if answer not in ("y", "yes"):
            say("not opened\n")
            return
    speed = getattr(termios, "B%d" % baud, None)
    if speed is None:
        say("unsupported speed %d\n" % baud)
        sys.exit(1)
    try:
        port = os.open(dev, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as e:
        say("\x1b[31mcannot open %s: %s\x1b[0m\n" % (dev, e.strerror))
        if e.errno == errno.EACCES:
            # The port's own group, as the system has it (dialout, uucp, ...).
            try:
                import grp
                group = grp.getgrgid(os.stat(dev).st_gid).gr_name
            except (OSError, KeyError, ImportError):
                group = "the port's group (ls -l %s)" % dev
            say("(add your user to the group %s, then log in again)\n" % group)
        sys.exit(1)
    tty.setraw(port)
    attrs = termios.tcgetattr(port)
    attrs[4] = attrs[5] = speed
    attrs[2] |= termios.CLOCAL | termios.CREAD
    termios.tcsetattr(port, termios.TCSANOW, attrs)
    say("\x1b[2mconnected to %s at %d · Ctrl+] to quit\x1b[0m\n" % (dev, baud))
    saved = termios.tcgetattr(0)
    tty.setraw(0)
    last = b""
    try:
        while True:
            ready, _, _ = select.select([0, port], [], [])
            if 0 in ready:
                data = os.read(0, 4096)
                if not data or QUIT in data:
                    break
                os.write(port, data)
            if port in ready:
                try:
                    data = os.read(port, 4096)
                except OSError:
                    say("\n\x1b[31m%s went away\x1b[0m\n" % dev)
                    break
                if not data:
                    continue
                # A lone LF is shown as CRLF (as picocom's --imap lfcrlf).
                out = bytearray()
                for b in data:
                    if b == 10 and last != b"\r":
                        out += b"\r"
                    out.append(b)
                    last = bytes([b])
                os.write(1, bytes(out))
    finally:
        termios.tcsetattr(0, termios.TCSADRAIN, saved)
        os.close(port)
    say("\ndisconnected from %s\n" % dev)


main()
