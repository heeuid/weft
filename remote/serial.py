"""weft serial console bridge - python standard library only.

Run by weft over `ssh -tt HOST` as `python3 -c 'exec(base64 of this)' PORT BAUD`:
it lives only in memory, nothing is written to the remote disk. It connects
the terminal (ssh's pty, put in raw mode) to the serial port (raw, at BAUD)
both ways until Ctrl+] is pressed or the port goes away.

Before opening, it says who else has the port open (two readers split the
data between them and both see garbage) and asks before going on. When the
port may not be opened (not in its group, e.g. dialout), it offers to open
it with sudo: it then ends with SUDO_EXIT and the command weft sent runs
it again under sudo, which asks for the password in this terminal (weft
keeps none of it).
"""

import glob
import errno
import os
import select
import sys
import termios
import tty

QUIT = b"\x1d"  # Ctrl+]
SUDO_EXIT = 77  # "run me again with sudo" (see host::serial_command)


def _lsof_holders(dev):
    """lsof's (pid, user, command) for `dev`, where there is no /proc
    (macOS, BSD)."""
    import subprocess
    try:
        out = subprocess.run(["lsof", "-FpcL", dev], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, universal_newlines=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    found, cur = [], None
    for line in out.splitlines():
        if line.startswith("p"):
            cur = [line[1:], "?", ""]
            found.append(cur)
        elif cur is not None and line.startswith("c"):
            cur[2] = line[1:]
        elif cur is not None and line.startswith("L"):
            cur[1] = line[1:]
    return [(p, u, c) for p, u, c in found if p.isdigit() and int(p) != os.getpid()]


def holders(dev):
    """(pid, user, command) of other processes with `dev` open, and lock files."""
    if not os.path.isdir("/proc/self/fd"):
        return sorted(set(_lsof_holders(dev)))
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


def ask():
    """A line typed (the terminal still cooked), read as it comes: no
    buffer of sys.stdin keeps what follows it."""
    got = b""
    while b"\n" not in got and b"\r" not in got:
        data = os.read(0, 1024)
        if not data:
            break
        got += data
    return got.decode("utf-8", "replace").strip().lower()


def set_speed(port, baud, attrs):
    """`attrs` (raw, 8N1) applied at `baud`: a speed termios names, else
    any the driver takes (Linux: termios2's BOTHER; macOS: IOSSIOSPEED),
    as 74880 for an ESP8266's boot or 921600 on a Mac."""
    speed = getattr(termios, "B%d" % baud, None)
    attrs[4] = attrs[5] = speed if speed is not None else termios.B9600
    termios.tcsetattr(port, termios.TCSANOW, attrs)
    if speed is not None:
        return
    import fcntl
    import struct
    if sys.platform.startswith("linux"):
        # struct termios2: 4 flags, c_line, c_cc[19], ispeed, ospeed.
        TCGETS2, TCSETS2, BOTHER, CBAUD = 0x802C542A, 0x402C542B, 0o010000, 0o010017
        fmt = "IIIIB19sII"
        buf = fcntl.ioctl(port, TCGETS2, bytes(struct.calcsize(fmt)))
        i, o, c, l, line, cc, _, _ = struct.unpack(fmt, buf)
        # Output and input speed both (the input's bits sit 16 higher).
        c = (c & ~(CBAUD | CBAUD << 16)) | BOTHER | BOTHER << 16
        fcntl.ioctl(port, TCSETS2, struct.pack(fmt, i, o, c, l, line, cc, baud, baud))
    elif sys.platform == "darwin":
        IOSSIOSPEED = 0x80045402
        fcntl.ioctl(port, IOSSIOSPEED, struct.pack("I", baud))
    else:
        raise OSError(errno.EINVAL, "speed %d is not one this system names" % baud)


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
        answer = ask()
        if answer not in ("y", "yes"):
            say("not opened\n")
            return
    try:
        port = os.open(dev, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError as e:
        say("\x1b[31mcannot open %s: %s\x1b[0m\n" % (dev, e.strerror))
        if e.errno == errno.EACCES and os.getuid() != 0:
            # The port's own group, as the system has it (dialout, uucp, …).
            try:
                import grp
                group = grp.getgrgid(os.stat(dev).st_gid).gr_name
            except (OSError, KeyError, ImportError):
                group = "the port's group (ls -l %s)" % dev
            say("(for good: add your user to the group %s, then log in again)\n" % group)
            say("Open it with sudo now? Your password is asked here and not kept. [y/N] ")
            answer = ask()
            if answer in ("y", "yes"):
                sys.exit(SUDO_EXIT)
        sys.exit(1)
    # The port's settings as found, given back as it closes.
    before = termios.tcgetattr(port)
    tty.setraw(port)
    attrs = termios.tcgetattr(port)
    # 8N1 with no flow control, whatever the last program left (minicom's
    # hardware flow control held every write while the device's CTS was
    # down; XOFF in the data stopped what was sent).
    attrs[0] &= ~(termios.IXON | termios.IXOFF | getattr(termios, "IXANY", 0))
    attrs[2] = (attrs[2] & ~(termios.PARENB | termios.CSTOPB | termios.CSIZE | getattr(termios, "CRTSCTS", 0))) | termios.CS8
    attrs[2] |= termios.CLOCAL | termios.CREAD
    try:
        set_speed(port, baud, attrs)
    except (OSError, termios.error) as e:
        say("\x1b[31m%s does not take %d baud: %s\x1b[0m\n" % (dev, baud, getattr(e, "strerror", None) or e))
        termios.tcsetattr(port, termios.TCSANOW, before)
        os.close(port)
        sys.exit(1)
    say("\x1b[2mconnected to %s at %d · Ctrl+] to quit\x1b[0m\n" % (dev, baud))
    saved = termios.tcgetattr(0)
    tty.setraw(0)
    # What is typed waits here until the port takes it (a slow line, a
    # paste): reading goes on meanwhile, and so does Ctrl+].
    pending = b""
    quit = False
    gone = False
    import time
    until = None
    try:
        while not (quit and not pending):
            # Ended with something still unsent: five seconds to send it.
            if quit and until is None:
                until = time.time() + 5
            if until is not None and time.time() > until:
                break
            ready, writable, _ = select.select([] if quit else [0, port] if len(pending) < 1 << 20 else [port], [port] if pending else [], [], 0.5 if quit else None)
            if 0 in ready:
                data = os.read(0, 4096)
                # What came before Ctrl+] goes; then it ends.
                quit = not data or QUIT in data
                pending += data.split(QUIT)[0]
            if port in writable and pending:
                try:
                    pending = pending[os.write(port, pending):]
                except BlockingIOError:
                    pass
                except OSError:
                    gone = True
                    break
            if port in ready:
                try:
                    data = os.read(port, 4096)
                except BlockingIOError:
                    continue
                except OSError:
                    data = b""
                if not data:
                    # Unplugged: a hung-up tty reads nothing for good.
                    gone = True
                    break
                # As the device sent it: weft shows a lone LF as CRLF, and
                # logs what came.
                os.write(1, data)
    finally:
        termios.tcsetattr(0, termios.TCSADRAIN, saved)
        # What was written goes out before the port closes (macOS drops
        # what is still queued at the close).
        try:
            termios.tcdrain(port)
        except termios.error:
            pass
        try:
            termios.tcsetattr(port, termios.TCSANOW, before)
        except termios.error:
            pass
        os.close(port)
    # What follows is the bridge's, not the device's (weft's log stops).
    os.write(1, b'\x1b]7777;{"t":"serial-end"}\x07')
    if gone:
        say("\n\x1b[31m%s went away\x1b[0m\n" % dev)
    say("\ndisconnected from %s\n" % dev)


main()
