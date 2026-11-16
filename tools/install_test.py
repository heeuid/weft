"""install_test.py: install.sh and uninstall.sh, for Linux and for macOS, in
a home folder of their own, with a stand-in weft (no build): install with
a desktop launcher and a PATH link, install again (an update), uninstall
keeping the data, install again (the data is still there), uninstall with
the data. Runs anywhere `sh` does (on Windows, Git Bash's).

    python tools/install_test.py

What it cannot see: that the real weft starts (a window has to open on a
real Linux desktop or Mac), and macOS's icon (made with sips/iconutil,
which only a Mac has).
"""
import os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
failures = []


def check(cond, what):
    print(('  ok    ' if cond else '  FAIL  ') + what)
    if not cond:
        failures.append(what)


def run(repo, home, osname, *args, input_text=''):
    env = dict(os.environ, HOME=home, WEFT_INSTALL_OS=osname, WEFT_NO_BUILD='1')
    env.pop('XDG_DATA_HOME', None)
    r = subprocess.run(['sh', *args], cwd=repo, env=env, capture_output=True, text=True, input=input_text)
    if r.returncode:
        print(r.stdout, r.stderr)
    return r


def read(p):
    with open(p, encoding='utf-8') as f:
        return f.read()


def one(osname):
    print(f'== {osname}')
    tmp = tempfile.mkdtemp(prefix=f'weft-install-{osname}-')
    try:
        repo, home = os.path.join(tmp, 'repo'), os.path.join(tmp, 'home')
        os.makedirs(os.path.join(repo, 'target', 'release', 'data'))
        os.makedirs(os.path.join(repo, 'assets'))
        os.makedirs(home)
        for f in ('install.sh', 'uninstall.sh', 'Cargo.toml'):
            shutil.copy(os.path.join(ROOT, f), repo)
        shutil.copy(os.path.join(ROOT, 'assets', 'weft.png'), os.path.join(repo, 'assets'))
        # The stand-in weft says where its data would be.
        with open(os.path.join(repo, 'target', 'release', 'weft'), 'w', newline='\n') as f:
            f.write('#!/bin/sh\necho "data=${WEFT_DATA_DIR:-beside}"\n')
        with open(os.path.join(repo, 'target', 'release', 'data', 'settings.json'), 'w') as f:
            f.write('{"from": "weft.sh"}')
        version = [l.split('"')[1] for l in read(os.path.join(repo, 'Cargo.toml')).splitlines() if l.startswith('version = ')][0]

        dest = os.path.join(home, 'Applications', 'weft') if osname == 'macos' else os.path.join(home, '.local', 'opt', 'weft')
        r = run(repo, home, osname, 'install.sh', 'desktop', 'path')
        check(r.returncode == 0, 'install.sh desktop path')
        check(f'weft {version} is installed in' in r.stdout, 'it says what it installed, and where')
        made = read(os.path.join(dest, '.installed')).split('\n')
        if osname == 'macos':
            app = os.path.join(dest, 'weft.app', 'Contents')
            check(os.path.isfile(os.path.join(app, 'MacOS', 'weft-bin')), 'weft.app holds the program')
            plist = read(os.path.join(app, 'Info.plist'))
            check('<string>weft</string>' in plist and f'<string>{version}</string>' in plist and 'APPL' in plist, 'Info.plist: the app, its version')
            out = run(repo, home, osname, os.path.join(app, 'MacOS', 'weft')).stdout.strip()
            check(out == 'data=' + os.path.join(dest, 'data').replace('\\', '/') or out.endswith('/Applications/weft/data'), f'the app keeps its data next to weft.app ({out})')
            check(any(p.endswith('Desktop/weft.app') for p in made), 'a desktop link to the app')
        else:
            check(os.path.isfile(os.path.join(dest, 'weft')) and os.path.isfile(os.path.join(dest, 'weft.png')), 'the program and its icon')
            entry = read(os.path.join(home, '.local', 'share', 'applications', 'weft.desktop'))
            # (The script sees the folder as sh writes it: /c/... under Git Bash.)
            check('/.local/opt/weft/weft"' in entry and 'Exec="' in entry and 'Icon=' in entry, 'an applications menu entry that runs it')
            check(os.path.isfile(os.path.join(home, 'Desktop', 'weft.desktop')), 'a desktop launcher')
        check(any(p.endswith('.local/bin/weft') for p in made), 'a PATH link, on the list for uninstall')
        check(os.path.exists(os.path.join(home, '.local', 'bin', 'weft')), 'the PATH link is there')
        check(read(os.path.join(dest, 'data', 'settings.json')) == '{"from": "weft.sh"}', "weft.sh's data comes along the first time")
        check(os.path.isfile(os.path.join(dest, 'uninstall.sh')), 'uninstall.sh is next to it')

        # An update: the data stays as weft left it.
        with open(os.path.join(dest, 'data', 'settings.json'), 'w') as f:
            f.write('{"from": "weft"}')
        r = run(repo, home, osname, 'install.sh')
        check(r.returncode == 0, 'install.sh again (an update)')
        check(read(os.path.join(dest, 'data', 'settings.json')) == '{"from": "weft"}', "an update leaves weft's data alone")
        check(len([l for l in read(os.path.join(dest, '.installed')).split('\n') if l]) == len([l for l in made if l]), 'the list of what it made is not doubled')

        # Uninstall, keeping the data (from the installed folder, as one would).
        r = run(repo, home, osname, os.path.join(dest, 'uninstall.sh'), 'keep')
        check(r.returncode == 0, 'uninstall.sh keep')
        left = sorted(os.listdir(dest)) if os.path.isdir(dest) else []
        check(left == ['data'], f'only the data is left ({left})')
        check(not os.path.exists(os.path.join(home, '.local', 'share', 'applications', 'weft.desktop')), 'the menu entry is gone')
        desk = os.path.join(home, 'Desktop', 'weft.app' if osname == 'macos' else 'weft.desktop')
        if osname == 'macos' and os.path.isdir(desk) and not os.path.islink(desk):
            print('  (skip  the desktop link: this sh copies instead of linking, so it is not recognised as ours)')
        else:
            check(not os.path.exists(desk), 'the desktop launcher is gone')
        link = os.path.join(home, '.local', 'bin', 'weft')
        if os.path.islink(link) or not os.path.exists(link):
            check(not os.path.exists(link), 'the PATH link is gone')
        else:
            print('  (skip  the PATH link: this sh copies instead of linking, so it is not recognised as ours)')

        # Again, then away with the data too.
        check(run(repo, home, osname, 'install.sh').returncode == 0, 'install.sh after that')
        check(read(os.path.join(dest, 'data', 'settings.json')) == '{"from": "weft"}', 'the kept data is picked up')
        r = run(repo, home, osname, 'uninstall.sh', 'purge')
        check(r.returncode == 0, 'uninstall.sh purge (from the source folder)')
        check(not os.path.exists(dest), 'nothing is left of it')
        r = run(repo, home, osname, 'uninstall.sh', 'keep')
        check(r.returncode != 0 and 'not installed' in r.stderr, 'uninstalling what is not there says so')
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


for o in ('linux', 'macos'):
    one(o)
print('all passed' if not failures else f'{len(failures)} failed')
sys.exit(1 if failures else 0)
