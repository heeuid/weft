"""The numbers in docs/regression.md: how many tests of each kind there
are, and how much of the code each kind runs (line coverage).

    python tools/testdoc.py            measure, and rewrite the numbers in the document
    python tools/testdoc.py --print    measure, and only print them

It builds the tests once more with coverage instrumentation into
target/cov (a few minutes the first time), runs them kind by kind, and
reads the result with the llvm tools rustup has (`rustup component add
llvm-tools`, and `cargo install cargo-binutils` for `cargo cov` /
`cargo profdata`).

The kinds:
  unit         a module's own tests: one function or type at a time
  screen       regress.rs and the other tests that run the whole app
               without a window: clicks, keys and drags fed to the real
               frame, checked by the app's state and what it drew
  integration  the ignored tests: real shells and the agent, end to end
               (`cargo test --workspace -- --ignored --test-threads=1`)
Measurements (frame_times, scenes) are run by hand and are not counted.
"""
import glob, json, os, re, subprocess, sys, datetime

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COV = os.path.join(ROOT, 'target', 'cov')
PROF = os.path.join(COV, 'prof')
ENV = dict(os.environ, CARGO_TARGET_DIR=COV, RUSTFLAGS='-C instrument-coverage')
MEASUREMENTS = ['regress::frame_times', 'scenes::scenes']
# Tests that run the whole app, outside regress.rs.
APP_TESTS = ['app::tests::a_process_list_from_another_host_is_dropped', 'app::tests::a_save_finds_its_file_after_a_rename', 'transfer::tests::copies_between_any_two_hosts',
             'ui_panels::tests::memos_never_conflict_with_their_own_saves', 'ui_panels::tests::side_panels_that_follow_the_terminal_follow_it',
             'ui_panels::tests::redo_key_stays_with_a_text_field', 'ui_term::tests::dragging_a_pane_header_moves_the_pane']


def sh(args, **kw):
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, errors='replace', **kw)


def binaries():
    """The test executables, by crate."""
    out = sh(['cargo', 'test', '--workspace', '--no-run', '--message-format=json'], env=ENV)
    if out.returncode != 0:
        sys.exit(out.stderr[-3000:])
    exes = {}
    for line in out.stdout.splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get('reason') == 'compiler-artifact' and m.get('executable') and m.get('profile', {}).get('test'):
            exes[m['target']['name'].replace('-', '_')] = m['executable']
    return exes


def tests_of(exe, ignored=False):
    out = sh([exe, '--list'] + (['--ignored'] if ignored else [])).stdout
    return [l[:-len(': test')] for l in out.splitlines() if l.endswith(': test')]


def is_screen(name):
    return name.startswith('regress::') or name in APP_TESTS


def run_kind(kind, exes, plan):
    """Run the tests of one kind; the merged profile's path."""
    for f in glob.glob(os.path.join(PROF, f'{kind}-*.profraw')):
        os.remove(f)
    failed = []
    for crate, exe in exes.items():
        names, extra = plan(crate, exe)
        if not names:
            continue
        env = dict(os.environ, LLVM_PROFILE_FILE=os.path.join(PROF, f'{kind}-{crate}-%p-%m.profraw'))
        # Exact names, a few hundred at most: one run of the binary.
        r = sh([exe, '--exact'] + extra + names, env=env)
        if r.returncode != 0:
            failed += [l for l in r.stdout.splitlines() if l.endswith('FAILED')]
    raws = glob.glob(os.path.join(PROF, f'{kind}-*.profraw'))
    data = os.path.join(PROF, f'{kind}.profdata')
    sh(['cargo', 'profdata', '--', 'merge', '-sparse'] + raws + ['-o', data])
    return data, failed


def coverage(data, exes):
    """Lines run / lines there are, by source file (weft's own only)."""
    objs = []
    for exe in exes.values():
        objs += ['--object', exe]
    out = sh(['cargo', 'cov', '--', 'export', '--format=text', '--summary-only', f'--instr-profile={data}'] + objs)
    files = {}
    for f in json.loads(out.stdout)['data'][0]['files']:
        path = os.path.relpath(f['filename'], ROOT).replace('\\', '/')
        if path.startswith('..') or path.startswith('target/') or not (path.startswith('src/') or path.startswith('crates/')):
            continue
        s = f['summary']['lines']
        files[path] = (s['covered'], s['count'])
    return files


def crate_of(path):
    return path.split('/')[1] if path.startswith('crates/') else 'weft (the app)'


TEST_ONLY = ('src/regress.rs', 'src/testkit.rs', 'src/scenes.rs', 'src/shot.rs')


def pct(c, n):
    return f'{100.0 * c / n:.0f} %' if n else '-'


def main():
    os.makedirs(PROF, exist_ok=True)
    exes = binaries()
    lists = {c: tests_of(e) for c, e in exes.items()}
    ignored = {c: [t for t in tests_of(e, True) if t not in MEASUREMENTS] for c, e in exes.items()}
    for c in lists:
        lists[c] = [t for t in lists[c] if t not in ignored[c] and t not in MEASUREMENTS]
    kinds = {
        'unit': lambda c, e: ([t for t in lists[c] if not is_screen(t)], []),
        'screen': lambda c, e: ([t for t in lists[c] if is_screen(t)], []),
        'integration': lambda c, e: (ignored[c], ['--ignored', '--test-threads=1']),
        'all': lambda c, e: (lists[c] + ignored[c], ['--include-ignored', '--test-threads=4']),
    }
    counts = {k: {c: len(plan(c, exes[c])[0]) for c in exes} for k, plan in kinds.items()}
    cov, failures = {}, {}
    for kind, plan in kinds.items():
        data, failed = run_kind(kind, exes, plan)
        cov[kind] = coverage(data, exes)
        failures[kind] = failed
        print(kind, 'done', '' if not failed else f'({len(failed)} failed)', flush=True)

    files = sorted(f for f in cov['all'] if f not in TEST_ONLY)
    crates = ['weft (the app)', 'weft-core', 'weft-draw']
    names = {'weft (the app)': 'weft', 'weft-core': 'weft_core', 'weft-draw': 'weft_draw'}
    commit = sh(['git', 'rev-parse', '--short', 'HEAD']).stdout.strip()
    L = []
    L.append(f'Measured {datetime.date.today()} at commit {commit} by `python tools/testdoc.py` (it rewrites this part).')
    L.append('')
    L.append('**How many tests**')
    L.append('')
    L.append('| Kind | the app | weft-core | weft-draw | together |')
    L.append('|---|--:|--:|--:|--:|')
    for kind, label in [('unit', 'Unit'), ('screen', 'Screen (the whole app, no window)'), ('integration', 'Integration (real shells, the agent; run with `--ignored`)')]:
        row = [counts[kind].get(names[c], 0) for c in crates]
        L.append(f'| {label} | {row[0]} | {row[1]} | {row[2]} | {sum(row)} |')
    tot = [counts['all'].get(names[c], 0) for c in crates]
    L.append(f'| **All** | {tot[0]} | {tot[1]} | {tot[2]} | **{sum(tot)}** |')
    L.append('')
    L.append('**How much of the code they run** (lines run / lines there are; test code itself left out)')
    L.append('')
    L.append('| Crate | lines | unit | screen | integration | all together |')
    L.append('|---|--:|--:|--:|--:|--:|')
    def total(kind, sel):
        c = sum(cov[kind].get(f, (0, 0))[0] for f in files if sel(f))
        n = sum(cov['all'][f][1] for f in files if sel(f))
        return c, n
    for cr in crates:
        sel = lambda f, cr=cr: crate_of(f) == cr
        n = total('all', sel)[1]
        L.append(f'| {cr} | {n} | ' + ' | '.join(pct(*total(k, sel)) for k in ['unit', 'screen', 'integration', 'all']) + ' |')
    n = total('all', lambda f: True)[1]
    L.append(f'| **everything** | {n} | ' + ' | '.join('**' + pct(*total(k, lambda f: True)) + '**' for k in ['unit', 'screen', 'integration', 'all']) + ' |')
    L.append('')
    L.append('By file (all kinds together), least covered first:')
    L.append('')
    L.append('| File | lines | run | not run |')
    L.append('|---|--:|--:|--:|')
    for f in sorted(files, key=lambda f: cov['all'][f][0] / max(cov['all'][f][1], 1)):
        c, n = cov['all'][f]
        L.append(f'| `{f}` | {n} | {pct(c, n)} | {n - c} |')
    block = '\n'.join(L)
    if any(failures.values()):
        print('FAILED while measuring:', failures)
    if '--print' in sys.argv:
        print(block)
        return
    doc = os.path.join(ROOT, 'docs', 'regression.md')
    s = open(doc, encoding='utf-8').read()
    a, b = '<!-- numbers:begin -->', '<!-- numbers:end -->'
    assert a in s and b in s, 'docs/regression.md has no numbers markers'
    s = s[:s.index(a) + len(a)] + '\n' + block + '\n' + s[s.index(b):]
    open(doc, 'w', encoding='utf-8', newline='\n').write(s)
    print('docs/regression.md updated')


if __name__ == '__main__':
    main()
