"""After `python tools/testdoc.py`: what no test ran, to see what a new
test should reach.

    python tools/uncovered.py                 functions never run, by file
    python tools/uncovered.py src/widgets.rs  the lines of one file that never ran

It reads the merged profile testdoc.py left in target/cov/prof.
"""
import glob, json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COV = os.path.join(ROOT, 'target', 'cov')
DATA = os.path.join(COV, 'prof', 'all.profdata')


def objects():
    exes = []
    for name in ('weft', 'weft_core', 'weft_draw'):
        found = glob.glob(os.path.join(COV, 'debug', 'deps', f'{name}-*.exe')) or [f for f in glob.glob(os.path.join(COV, 'debug', 'deps', f'{name}-*')) if '.' not in os.path.basename(f)]
        if found:
            exes += ['--object', max(found, key=os.path.getmtime)]
    return exes


def demangle(name):
    """`_ZN4weft9ui_panels3App10ui_history17h…E` → weft::ui_panels::App::ui_history."""
    m = re.match(r'_ZN(.*)E$', name)
    if not m:
        return name
    rest, parts = m.group(1), []
    while rest and rest[0].isdigit():
        n = re.match(r'\d+', rest).group(0)
        parts.append(rest[len(n):len(n) + int(n)])
        rest = rest[len(n) + int(n):]
    parts = [p for p in parts if not re.fullmatch(r'h[0-9a-f]{16}', p)]
    text = '::'.join(parts)
    for a, b in (('$LT$', '<'), ('$GT$', '>'), ('$u20$', ' '), ('$RF$', '&'), ('$C$', ','), ('$LP$', '('), ('$RP$', ')'), ('..', '::'), ('$u7b$', '{'), ('$u7d$', '}')):
        text = text.replace(a, b)
    return text


def main():
    if not os.path.exists(DATA):
        sys.exit('no profile: run python tools/testdoc.py first')
    out = subprocess.run(['cargo', 'cov', '--', 'export', '--format=text', f'--instr-profile={DATA}'] + objects(), cwd=ROOT, capture_output=True, text=True, errors='replace')
    data = json.loads(out.stdout)['data'][0]
    rel = lambda f: os.path.relpath(f, ROOT).replace('\\', '/')
    if len(sys.argv) > 1:
        want = sys.argv[1].replace('\\', '/')
        for f in data['files']:
            if rel(f['filename']) != want:
                continue
            # Segments: line, column, count, has a count, is a region's start, is a gap.
            dead, line0, count0 = set(), None, None
            for line, _col, count, has, _start, _gap in f['segments']:
                if line0 is not None and count0 == 0:
                    dead.update(range(line0, line + 1))
                line0, count0 = line, (count if has else None)
            src = open(os.path.join(ROOT, want), encoding='utf-8').read().splitlines()
            runs, start, prev = [], None, None
            for n in sorted(dead):
                if start is None:
                    start = prev = n
                elif n == prev + 1:
                    prev = n
                else:
                    runs.append((start, prev))
                    start = prev = n
            if start is not None:
                runs.append((start, prev))
            for a, b in runs:
                if b - a >= 3:
                    print(f'{a:5}-{b:<5} {src[a - 1].strip()[:110]}')
        return
    never = {}
    for fn in data['functions']:
        files = [rel(f) for f in fn['filenames']]
        if not files or files[0].startswith('..') or 'target/' in files[0]:
            continue
        name = demangle(fn['name'])
        if '{{closure}}' in name or '::tests::' in name or name.endswith('::tests'):
            continue
        lines = max((r[2] - r[0] + 1 for r in fn['regions']), default=0)
        key = (files[0], name)
        # A generic function has a copy per use: never run only if none ran.
        never[key] = (never.get(key, (0, 0))[0] + fn['count'], lines)
    by_file = {}
    for (file, name), (count, lines) in never.items():
        if count == 0 and lines >= 4:
            by_file.setdefault(file, []).append((lines, name))
    for file in sorted(by_file, key=lambda f: -sum(l for l, _ in by_file[f])):
        fns = sorted(by_file[file], reverse=True)
        print(f'{file}: {len(fns)} functions never run, {sum(l for l, _ in fns)} lines')
        for lines, name in fns[:40]:
            print(f'   {lines:4}  {name.split("::", 1)[-1]}')


if __name__ == '__main__':
    main()
