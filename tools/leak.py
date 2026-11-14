"""Does a test run leave console hosts behind? (Windows.)

    python tools/leak.py <test binary, or a glob for it> [arguments for it]

e.g. python tools/leak.py "target/debug/deps/weft-*.exe" regress::

Runs the newest binary matching, then counts `OpenConsole.exe` /
`conhost.exe` whose parent process is gone, new since before the run:
right after, and again later. A terminal whose pseudo console was never
closed leaves its host behind for good, and each keeps a core busy; weft
did that until 0.2.1. It lists every process of the PC (through
PowerShell), which is why it is a script and not a test.
"""
import glob, json, os, subprocess, sys, time


def orphans():
    ps = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress"
    procs = json.loads(subprocess.run(['powershell', '-NoProfile', '-Command', ps], capture_output=True, text=True, errors='replace').stdout)
    alive = {p['ProcessId'] for p in procs}
    return sorted(p['ProcessId'] for p in procs if p['Name'] in ('OpenConsole.exe', 'conhost.exe') and p['ParentProcessId'] not in alive)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    found = glob.glob(sys.argv[1])
    if not found:
        sys.exit(f'no such binary: {sys.argv[1]}')
    exe = max(found, key=os.path.getmtime)
    before = orphans()
    print('binary', exe, '| already without a parent:', len(before))
    r = subprocess.run([exe] + sys.argv[2:], capture_output=True, text=True, errors='replace')
    print((r.stdout.strip().splitlines() or ['?'])[-1])
    waited = 0
    left = []
    for at in (3, 20, 130):
        time.sleep(at - waited)
        waited = at
        left = [p for p in orphans() if p not in before]
        print(f'after {at:>3} s: {len(left)} left behind', left[:8])
    sys.exit(1 if left else 0)


if __name__ == '__main__':
    main()
