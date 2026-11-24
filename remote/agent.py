"""weft remote agent - python standard library only.

Started as `python3 -u -c "import sys;exec(sys.stdin.buffer.read(int(sys.stdin.buffer.readline())))"`
over `ssh -T`: this source arrives on stdin and lives only in memory, nothing
is written to the remote disk.  It is deliberately thin: it only does what has
to happen on the remote machine (file system access, process table, running
POSIX tools).  Rendering, editing, filtering, sorting, completion ranking and
history all happen locally in weft.

Protocol: one JSON object per line in both directions.
    -> {"id": 7, "op": "list", "path": "/etc"}
    <- {"id": 7, "ok": true, "entries": [...]}
"""

import base64
import json
import os
import signal
import socket
import stat
import subprocess
import sys
import threading
import time

MAX_READ = 8 * 1024 * 1024
_out = sys.stdout.buffer
_lock = threading.Lock()


def send(obj):
    data = json.dumps(obj, ensure_ascii=False).encode("utf-8", "replace") + b"\n"
    with _lock:
        _out.write(data)
        _out.flush()


def _path(p):
    p = p or "."
    # Windows python behind an msys/cygwin shell sees /c/Users style paths;
    # a Windows host's PowerShell says /C:/Users (its folder, OSC 7).
    if os.name == "nt" and len(p) >= 3 and p[0] == "/" and p[1].isalpha() and p[2] == ":":
        p = p[1:]
    elif os.name == "nt" and len(p) >= 3 and p[0] == "/" and p[2] == "/" and p[1].isalpha():
        p = p[1] + ":" + p[2:]
    return os.path.abspath(os.path.expanduser(p))


def op_hello(req):
    return {
        "home": os.path.expanduser("~"),
        "cwd": os.getcwd(),
        "host": socket.gethostname(),
        "user": _user(),
        "platform": sys.platform,
        "python": sys.version.split()[0],
        "pid": os.getpid(),
    }


def op_list(req):
    d = _path(req.get("path"))
    hidden = bool(req.get("hidden"))
    out = []
    with os.scandir(d) as it:
        for e in it:
            if not hidden and e.name.startswith("."):
                continue
            try:
                st = e.stat(follow_symlinks=True)
                isdir = stat.S_ISDIR(st.st_mode)
                size, mtime = st.st_size, int(st.st_mtime)
            except OSError:
                isdir, size, mtime = False, 0, 0
            item = {"n": e.name, "d": isdir, "l": e.is_symlink(), "s": 0 if isdir else size, "m": mtime}
            if req.get("mode"):
                try:
                    item["p"] = stat.filemode(e.stat(follow_symlinks=False).st_mode)
                except OSError:
                    item["p"] = ""
            out.append(item)
    if req.get("ignored") is False:
        drop = _git_ignored(d, [x["n"] for x in out])
        out = [x for x in out if x["n"] not in drop]
    return {"path": d, "entries": out}


def _git_ignored(d, names):
    """The names in folder `d` that git ignores (.gitignore and friends);
    none outside a repository or without git."""
    if not names:
        return set()
    try:
        p = subprocess.run(["git", "-C", d, "check-ignore", "-z", "--stdin"], input="\0".join(names).encode(),
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return set()
    # 0: some ignored, 1: none, 128: not a repository.
    if p.returncode != 0:
        return set()
    return set(x for x in p.stdout.decode("utf-8", "replace").split("\0") if x)


def op_stat(req):
    p = _path(req.get("path"))
    try:
        st = os.stat(p)
    except OSError:
        return {"path": p, "exists": False}
    return {"path": p, "exists": True, "dir": stat.S_ISDIR(st.st_mode), "size": st.st_size, "mtime": st.st_mtime}


def _user():
    # The OS's word for it (USER, LOGNAME, USERNAME or the password
    # database), not one environment variable's.
    try:
        import getpass
        return getpass.getuser()
    except Exception:
        return ""


def op_read(req):
    p = _path(req.get("path"))
    # A folder, told as one (not by an exception's name: Windows' Python
    # raises PermissionError for it, and it was opened as a file).
    if os.path.isdir(p):
        return {"path": p, "is_dir": True}
    st = os.stat(p)
    if st.st_size > int(req.get("max", MAX_READ)):
        raise ValueError("file too large (%d bytes)" % st.st_size)
    with open(p, "rb") as f:
        raw = f.read()
    res = {"path": p, "size": len(raw), "mtime": st.st_mtime}
    try:
        res["text"] = raw.decode("utf-8")
    except UnicodeDecodeError:
        res["b64"] = base64.b64encode(raw).decode()
    return res


def op_write(req):
    p = _path(req.get("path"))
    expect = req.get("expect_mtime")
    if expect is not None and os.path.exists(p) and not req.get("force"):
        cur = os.stat(p).st_mtime
        if abs(cur - float(expect)) > 1e-6:
            return {"path": p, "conflict": True, "mtime": cur}
    data = base64.b64decode(req["b64"]) if "b64" in req else req.get("text", "").encode("utf-8")
    # Write in place so ownership, mode and hard links of the user's file are kept.
    with open(p, "wb") as f:
        f.write(data)
    return {"path": p, "size": len(data), "mtime": os.stat(p).st_mtime}


# File transfer (weft relays chunks between any two hosts, this one being
# either end): what a path holds, then chunks read from or written at an
# offset. Writes go to a ".weft-part" file renamed into place at the end,
# so a transfer cut short never leaves a half file under the real name.
MAX_CHUNK = 4 * 1024 * 1024


def op_walk(req):
    p = _path(req.get("path"))
    if not os.path.isdir(p):
        return {"dir": False, "items": [{"p": "", "s": os.path.getsize(p), "d": False}]}
    items = []
    for root, dirs, files in os.walk(p):
        dirs.sort()
        rel = os.path.relpath(root, p)
        rel = "" if rel == "." else rel.replace(os.sep, "/")
        if rel:
            items.append({"p": rel, "s": 0, "d": True})
        for name in sorted(files):
            try:
                size = os.path.getsize(os.path.join(root, name))
            except OSError:
                continue  # a broken link
            items.append({"p": rel + "/" + name if rel else name, "s": size, "d": False})
    return {"dir": True, "items": items}


def op_rchunk(req):
    p = _path(req.get("path"))
    off = int(req.get("off", 0))
    n = min(int(req.get("len", MAX_CHUNK)), MAX_CHUNK)
    with open(p, "rb") as f:
        size = os.fstat(f.fileno()).st_size
        f.seek(off)
        raw = f.read(n)
    return {"b64": base64.b64encode(raw).decode(), "n": len(raw), "eof": off + len(raw) >= size, "size": size}


def op_wchunk(req):
    p = _path(req.get("path"))
    off = int(req.get("off", 0))
    data = base64.b64decode(req.get("b64", ""))
    with open(p, "r+b" if off > 0 else "wb") as f:
        f.seek(off)
        f.write(data)
    return {"n": len(data)}


def op_finish(req):
    part = _path(req.get("part"))
    if req.get("abort"):
        try:
            os.remove(part)
        except OSError:
            pass
        return {}
    os.replace(part, _path(req.get("path")))
    return {}


def op_mkdirs(req):
    root = _path(req.get("root"))
    os.makedirs(root, exist_ok=True)
    for d in req.get("dirs", []):
        os.makedirs(os.path.join(root, d), exist_ok=True)
    return {}


def op_mkdir(req):
    p = _path(req.get("path"))
    os.mkdir(p)
    return {"path": p}


def op_create(req):
    p = _path(req.get("path"))
    # O_EXCL: never over a file that exists.
    os.close(os.open(p, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o666))
    return {"path": p}


def op_rename(req):
    src, dst = _path(req.get("from")), _path(req.get("to"))
    # rename() replaces silently: never onto something else. (The same
    # file under another spelling: only its letters' case changes.)
    if os.path.lexists(dst) and not (os.path.exists(src) and os.path.samefile(src, dst)):
        raise OSError("%s already exists" % os.path.basename(dst))
    os.rename(src, dst)
    return {"path": dst}


def op_delete(req):
    if not str(req.get("path") or "").strip():
        raise OSError("refusing to delete a root")
    p = _path(req.get("path"))
    if os.path.dirname(p) == p:
        raise OSError("refusing to delete a root")
    if req.get("trash"):
        _trash(p, os.path.expanduser("~"), sys.platform == "darwin")
        return {"path": p, "trashed": True}
    if os.path.isdir(p) and not os.path.islink(p):
        import shutil
        shutil.rmtree(p)  # links inside are removed, not followed
    else:
        os.remove(p)
    return {"path": p, "trashed": False}


def _trash(p, home, mac):
    """Move p to the trash, as weft does on this PC (host.rs, to_trash):
    macOS' ~/.Trash (or the disk's .Trashes/<uid>); elsewhere the
    freedesktop.org trash, which file managers show and put back from:
    $XDG_DATA_HOME/Trash on the home's disk, else the disk's own
    .Trash/<uid> (a sticky folder made for it) or .Trash-<uid>. Not on
    Windows: over ssh there is no one to ask about what the Recycle Bin
    will not take, and it would go for good, unasked."""
    if os.name == "nt":
        raise OSError("no Recycle Bin over ssh: delete it for good instead")
    import stat as st, time
    from urllib.parse import quote
    dev = os.lstat(p).st_dev
    uid = os.getuid()

    def same_disk(d):
        try:
            return os.stat(d).st_dev == dev
        except OSError:
            return False

    def top():
        t = p
        while os.path.dirname(t) != t and same_disk(os.path.dirname(t)):
            t = os.path.dirname(t)
        return t

    name = os.path.basename(p)

    def names():
        yield name
        k = 2
        while True:
            yield "%s.%d" % (name, k)
            k += 1

    if mac:
        trash = os.path.join(home, ".Trash") if same_disk(home) else os.path.join(top(), ".Trashes", str(uid))
        os.makedirs(trash, exist_ok=True)
        for n in names():
            to = os.path.join(trash, n)
            if not os.path.lexists(to):
                os.rename(p, to)
                return
    data = os.environ.get("XDG_DATA_HOME") or ""
    if not data.startswith("/"):
        data = os.path.join(home, ".local", "share")
    if same_disk(data) or (not os.path.exists(data) and same_disk(home)):
        trash, rel = os.path.join(data, "Trash"), None
    else:
        rel = top()
        shared = os.path.join(rel, ".Trash")
        sticky = os.path.isdir(shared) and not os.path.islink(shared) and os.lstat(shared).st_mode & st.S_ISVTX
        trash = os.path.join(shared, str(uid)) if sticky else os.path.join(rel, ".Trash-%d" % uid)
    for d in ("files", "info"):
        os.makedirs(os.path.join(trash, d), exist_ok=True)
    shown = os.path.relpath(p, rel) if rel else p
    info = "[Trash Info]\nPath=%s\nDeletionDate=%s\n" % (quote(shown, safe="/-_.~"), time.strftime("%Y-%m-%dT%H:%M:%S"))
    for n in names():
        info_file = os.path.join(trash, "info", n + ".trashinfo")
        try:
            fd = os.open(info_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        except FileExistsError:
            continue
        with os.fdopen(fd, "w") as f:
            f.write(info)
        try:
            os.rename(p, os.path.join(trash, "files", n))
        except OSError:
            os.remove(info_file)
            raise
        return


def _port_holders(dev):
    """Other processes with a serial port open (pid, user, command), and
    the pid in a lock file (minicom, screen…). Other users' processes show
    only to root."""
    import glob
    real = os.path.realpath(dev)
    found = {}
    for fd in glob.glob("/proc/[0-9]*/fd/*"):
        try:
            if os.path.realpath(fd) != real:
                continue
            pid = fd.split("/")[2]
            with open("/proc/%s/cmdline" % pid, "rb") as f:
                cmd = f.read().replace(b"\0", b" ").decode("utf-8", "replace").strip()
            uid = os.stat("/proc/%s" % pid).st_uid
            try:
                import pwd
                user = pwd.getpwuid(uid).pw_name
            except Exception:
                user = str(uid)
            found[pid] = {"pid": int(pid), "user": user, "cmd": cmd[:200]}
        except (OSError, ValueError):
            pass
    for lock in ("/var/lock/LCK..", "/run/lock/LCK.."):
        try:
            with open(lock + os.path.basename(real)) as f:
                pid = f.read().strip().split()[0]
            if pid not in found and os.path.exists("/proc/%s" % pid):
                found[pid] = {"pid": int(pid), "user": "?", "cmd": "(lock file)"}
        except (OSError, IndexError):
            pass
    return list(found.values())


def op_serial_ports(req):
    """Serial ports (USB adapters, on-board UARTs) with who has them open."""
    import glob
    by_id = {}
    for link in glob.glob("/dev/serial/by-id/*"):
        by_id[os.path.realpath(link)] = os.path.basename(link)
    ports = []
    # Every tty with a device behind it, whatever its driver names it
    # (Linux: /sys/class/tty/<name>/device), those with a stable name, and
    # the call-out side on macOS and the BSDs (cu.*, cua*: they open
    # without waiting for a carrier).
    devs = set()
    for d in glob.glob("/sys/class/tty/*/device"):
        devs.add("/dev/" + os.path.basename(os.path.dirname(d)))
    devs.update(os.path.realpath(link) for link in glob.glob("/dev/serial/by-id/*"))
    for pattern in ("/dev/cu.*", "/dev/cua[!.]*"):
        devs.update(glob.glob(pattern))
    for dev in sorted(d for d in devs if os.path.exists(d)):
        name = os.path.basename(dev)
        if name in ("cu.Bluetooth-Incoming-Port", "cu.debug-console"):
            continue
        # A serial-core port with no UART behind it (most PCs list 32 of
        # them): its type is 0. Told by the kernel, not by a driver's name.
        try:
            with open("/sys/class/tty/%s/type" % name) as f:
                if f.read().strip() == "0":
                    continue
        except OSError:
            pass
        ports.append({"port": dev, "desc": by_id.get(os.path.realpath(dev), ""), "holders": _port_holders(dev)})
    return {"ports": ports}


def op_complete(req):
    d = _path(req.get("dir"))
    prefix = req.get("prefix", "")
    names = []
    with os.scandir(d) as it:
        for e in it:
            if e.name.startswith(prefix) and (prefix.startswith(".") or not e.name.startswith(".")):
                try:
                    names.append(e.name + ("/" if e.is_dir() else ""))
                except OSError:
                    names.append(e.name)
    names.sort()
    return {"dir": d, "prefix": prefix, "names": names[:500]}


def _procs_windows():
    """Windows' processes, as op_procs gives them (no CPU %: it takes two
    readings; no user: asking each process for its owner is slow)."""
    # A line each, its fields joined by a tab ([char]9: no escapes to quote).
    script = ("Get-CimInstance Win32_Process | ForEach-Object { @($_.ProcessId, $_.ParentProcessId, $_.WorkingSetSize, "
              "$(if ($_.CreationDate) { [int]((Get-Date) - $_.CreationDate).TotalSeconds } else { 0 }), "
              "$(if ($_.CommandLine) { $_.CommandLine } else { $_.Name })) -join [char]9 }")
    out = subprocess.run(["powershell", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script],
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False).stdout.decode("utf-8", "replace")
    total, _ = _meminfo()
    rows = []
    for line in out.splitlines():
        parts = line.split("\t", 4)
        if len(parts) < 5:
            continue
        try:
            pid, ppid, ws, secs = int(parts[0]), int(parts[1]), int(parts[2] or 0), int(parts[3] or 0)
        except ValueError:
            continue
        d, rest = divmod(secs, 86400)
        h, rest = divmod(rest, 3600)
        m, s = divmod(rest, 60)
        etime = ("%d-" % d if d else "") + ("%02d:%02d:%02d" % (h, m, s) if d or h else "%02d:%02d" % (m, s))
        rows.append([pid, ppid, "", 0.0, round(100.0 * ws / total, 1) if total else 0.0, etime, parts[4].strip()])
    return rows


def op_procs(req):
    if os.name == "nt":
        return {"procs": _procs_windows()}
    # POSIX ps; the column set is supported by procps and BSD ps alike.
    # In the C locale: numbers with a dot (de_DE's 0,5 was dropped).
    out = subprocess.run(
        ["ps", "-A", "-o", "pid=,ppid=,user=,pcpu=,pmem=,etime=,args="],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False,
        env=dict(os.environ, LC_ALL="C"),
    ).stdout.decode("utf-8", "replace")
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 6)
        if len(parts) < 7:
            continue
        pid, ppid, user, cpu, mem, etime, args = parts
        try:
            rows.append([int(pid), int(ppid), user, float(cpu), float(mem), etime, args])
        except ValueError:
            continue
    return {"procs": rows}


def op_kill(req):
    sig = getattr(signal, "SIG" + str(req.get("sig", "TERM")).upper(), signal.SIGTERM)
    os.kill(int(req["pid"]), sig)
    return {"pid": req["pid"]}


def _meminfo():
    if os.name == "nt":
        # GlobalMemoryStatusEx: total and available physical memory.
        try:
            import ctypes

            class MEMORYSTATUSEX(ctypes.Structure):
                _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong), ("ullTotalPhys", ctypes.c_ulonglong),
                            ("ullAvailPhys", ctypes.c_ulonglong), ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                            ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong), ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]
            st = MEMORYSTATUSEX()
            st.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
            if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st)):
                return st.ullTotalPhys, st.ullTotalPhys - st.ullAvailPhys
        except (OSError, AttributeError, ValueError):
            pass
        return None, None
    try:
        info = {}
        with open("/proc/meminfo") as f:
            for line in f:
                k, v = line.split(":", 1)
                info[k] = int(v.split()[0]) * 1024
        total = info.get("MemTotal", 0)
        avail = info.get("MemAvailable", info.get("MemFree", 0))
        return total, total - avail
    except (OSError, ValueError):
        return None, None


def op_sys(req):
    res = {"time": time.time()}
    try:
        res["load"] = list(os.getloadavg())
    except (OSError, AttributeError):
        pass
    total, used = _meminfo()
    if total:
        res["mem_total"], res["mem_used"] = total, used
    try:
        import shutil
        du = shutil.disk_usage(_path(req.get("path")))
        res["disk_total"], res["disk_free"] = du.total, du.free
    except (OSError, AttributeError):
        pass
    try:
        res["ncpu"] = os.cpu_count()
    except Exception:
        pass
    return res


def _grep_python(root, pattern, icase, hidden, limit, line_max=400):
    """grep -rnIE in Python, where there is no grep (a Windows host): the
    same folders left out, binary files skipped, the pattern as Python's re
    reads it (the extended syntax grep -E has, near enough)."""
    import re
    rx = re.compile(pattern, re.IGNORECASE if icase else 0)
    skip = {".git", "node_modules", "target"}
    hits = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = sorted(d for d in dirnames if d not in skip and (hidden or not d.startswith(".")))
        for name in sorted(filenames):
            if not hidden and name.startswith("."):
                continue
            path = os.path.join(dirpath, name)
            try:
                with open(path, "rb") as f:
                    data = f.read(16 * 1024 * 1024)
            except OSError:
                continue
            if b"\0" in data[:8192]:
                continue
            for num, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
                if rx.search(line):
                    hits.append([path, num, line[:line_max]])
                    if len(hits) >= limit:
                        return hits
    return hits


def op_grep(req):
    pattern = req["pattern"]
    root = _path(req.get("path"))
    # How many lines and how long each: a glob or fuzzy search asks for more
    # ("scan"), weft deciding which of them match.
    limit = int(req.get("scan", req.get("limit", 500)))
    line_max = int(req.get("line_max", 400))
    if not req.get("regex"):
        literal = "".join("\\" + c if c in ".[]()*+?{}|^$\\" else c for c in pattern)
    else:
        literal = pattern
    import shutil
    if not shutil.which("grep"):
        hits = _grep_python(root, literal, bool(req.get("icase")), bool(req.get("hidden")), limit, line_max)
        return {"root": root, "pattern": literal, "hits": hits, "truncated": len(hits) >= limit}
    cmd = ["grep", "-rnIH", "--exclude-dir=.git", "--exclude-dir=node_modules", "--exclude-dir=target"]
    if req.get("icase"):
        cmd.append("-i")
    # Always extended syntax (-E: ( | + ? are operators, \( a bracket); text
    # as typed goes escaped. -F with -i crashes some greps (msys).
    if not req.get("regex"):
        pattern = "".join("\\" + c if c in ".[]()*+?{}|^$\\" else c for c in pattern)
    cmd += ["-E", "-e", pattern]
    cwd = None
    if req.get("hidden"):
        cmd.append(root)
    else:
        # Skip names starting with a dot. grep applies --exclude-dir to the
        # folders it is given too, by any part of their path (so all of
        # ~/.config/x would go): give it the folder's visible entries,
        # from inside it, and let the rule work on names below.
        entries = sorted(e for e in os.listdir(root) if not e.startswith("."))
        if not entries:
            return {"root": root, "pattern": pattern, "hits": [], "truncated": False}
        cmd[1:1] = ["--exclude=.*", "--exclude-dir=.*"]
        cmd += ["--"] + entries
        cwd = root
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=cwd)
    hits = []
    for raw in proc.stdout:
        line = raw.decode("utf-8", "replace").rstrip("\n").rstrip("\r")
        path, _, rest = line.partition(":")
        num, _, text = rest.partition(":")
        if num.isdigit():
            hits.append([os.path.join(root, path) if cwd else path, int(num), text[:line_max]])
        if len(hits) >= limit:
            proc.kill()
            break
    err = proc.stderr.read().decode("utf-8", "replace").strip()
    proc.wait()
    # 0 found, 1 none; 2 is trouble such as a bad pattern (unreadable files
    # also give 2, alongside hits); anything else, grep itself failed.
    if proc.returncode not in (0, 1) and not hits and len(hits) < limit:
        # A grep without these options (not GNU's or a BSD's): text as typed
        # is searched here instead.
        if not req.get("regex"):
            hits = _grep_python(root, literal, bool(req.get("icase")), bool(req.get("hidden")), limit, line_max)
            return {"root": root, "pattern": literal, "hits": hits, "truncated": len(hits) >= limit}
        raise ValueError(err.splitlines()[0] if err else "grep failed")
    return {"root": root, "pattern": pattern, "hits": hits, "truncated": len(hits) >= limit}


# Folders listed but not descended into by "tree".
TREE_SKIP = {".git", "node_modules", "target", "__pycache__", ".venv", "venv", ".cache", ".tox", "dist", "build"}


def op_tree(req):
    # Every name under a folder, for the file panel filter (matching is local):
    # the nearest first, a folder at a time by depth (at the limit the deepest
    # is cut, not a folder next to the root a big one crowded out); a link to
    # a folder gone into, each real folder once.
    import collections
    root = _path(req.get("path"))
    hidden = bool(req.get("hidden"))
    limit = int(req.get("max", 20000))
    out = []
    queue = collections.deque([""])
    seen = {os.path.realpath(root)}
    while queue:
        rel = queue.popleft()
        try:
            it = os.scandir(os.path.join(root, rel) if rel else root)
        except OSError:
            continue
        with it:
            for e in it:
                if not hidden and e.name.startswith("."):
                    continue
                try:
                    isdir = e.is_dir()
                except OSError:
                    isdir = False
                r = rel + "/" + e.name if rel else e.name
                out.append([r, isdir])
                if len(out) >= limit:
                    return {"root": root, "entries": out, "truncated": True}
                if isdir and e.name not in TREE_SKIP:
                    real = os.path.realpath(e.path)
                    if real not in seen:
                        seen.add(real)
                        queue.append(r)
    return {"root": root, "entries": out, "truncated": False}


def op_cwd(req):
    # A shell's directory from outside (shells without integration): Linux
    # /proc, else lsof (macOS, BSD).
    pid = int(req["pid"])
    try:
        return {"cwd": os.readlink("/proc/%d/cwd" % pid)}
    except OSError:
        pass
    try:
        out = subprocess.run(["lsof", "-a", "-p", str(pid), "-d", "cwd", "-Fn"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, universal_newlines=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError):
        out = ""
    for line in out.splitlines():
        if line.startswith("n"):
            return {"cwd": line[1:]}
    raise OSError("cannot read the directory of process %d" % pid)


OPS = {
    "hello": op_hello, "list": op_list, "stat": op_stat, "read": op_read, "write": op_write,
    "complete": op_complete, "procs": op_procs, "kill": op_kill, "sys": op_sys, "grep": op_grep,
    "tree": op_tree, "cwd": op_cwd, "walk": op_walk, "rchunk": op_rchunk, "wchunk": op_wchunk,
    "finish": op_finish, "mkdirs": op_mkdirs, "serial_ports": op_serial_ports,
    "mkdir": op_mkdir, "create": op_create, "rename": op_rename, "delete": op_delete,
}


def handle(req):
    rid = req.get("id")
    try:
        fn = OPS.get(req.get("op"))
        if fn is None:
            raise ValueError("unknown op %r" % req.get("op"))
        res = fn(req)
        res["id"], res["ok"] = rid, True
        send(res)
    except Exception as exc:
        send({"id": rid, "ok": False, "error": "%s: %s" % (type(exc).__name__, exc)})


def main():
    send({"id": 0, "ok": True, "event": "ready", **op_hello({})})
    for raw in sys.stdin.buffer:
        line = raw.strip()
        if not line:
            continue
        try:
            req = json.loads(line.decode("utf-8", "replace"))
        except ValueError as exc:
            send({"id": None, "ok": False, "error": "bad json: %s" % exc})
            continue
        threading.Thread(target=handle, args=(req,), daemon=True).start()


main()
