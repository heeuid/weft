# Changelog

weft's version is in `Cargo.toml` and shown by `weft --version`, Help, and
the palette's "About weft". Each release is tagged `v<version>` in git.
The middle number goes up when something is added, the last one when
things are only fixed.

## 0.2.2

Found by a review of everything changed since 0.1.0.

Fixed
- "Save and close" no longer closes over a file that was changed while
  the save was on its way: weft asks about that one. Cancel also cancels
  a close that was waiting for files to be saved.
- Deleting on this PC: something that can't go to the Recycle Bin (a
  network share, a drive without one, too big) is no longer deleted for
  good silently; Windows says so and asks.
- A table selection in read mode no longer takes Ctrl+C from the terminal
  or another field, and gives way to text selected in the document.
- A process list arriving late from the host shown before is dropped
  (Kill could have ended a process here with a pid from there).
- A file renamed while it was being saved no longer stays "saving".
- New names Windows would read as something else (`a:b`, `what?`, a dot
  at the end) are refused on this PC. A remote rename that only changes
  the letters' case works on case-insensitive disks.
- Linux, macOS: a closed terminal lets go of what ran in it at once, as
  before 0.2.1.
- `cargo clippy --workspace --all-targets` is clean.

## 0.2.1

Fixed
- Console hosts no longer outlive weft. Windows' console host asks where
  the cursor is as it starts and does nothing until it is told, not even
  close; a terminal that was closed before weft had drawn it (a tab opened
  and closed at once, weft closed as it started) never answered, so its
  `OpenConsole.exe` stayed behind for good, each keeping a core busy. One
  was left about every time weft closed; they slowed the whole PC down
  over days. Left-over ones from before can be ended in Task Manager
  (`OpenConsole.exe` with no weft running), or go at the next sign-out.
- weft waits (up to 3 s, after its window is gone) for its terminals to be
  closed before it ends.

Changed
- weft is a workspace of three crates: `weft-core` (terminal, hosts,
  search; no GUI), `weft-draw` (theme, fonts, markdown layout) and the app.
  A change builds only what it touches. `cargo test --workspace` runs
  everything; without `--release` the dependencies are still optimized,
  so tests build in about a minute and run in under one.

## 0.2.0

The first numbered release; everything before it was 0.1.0.

Added
- `weft --version` / `-V`; the version in Help and "About weft"; local
  shells get `TERM_PROGRAM=weft` and `TERM_PROGRAM_VERSION`.
- Files: new folder, new file, rename and delete, on this PC and on remote
  hosts. Deleting asks first; on this PC it goes to the Recycle Bin.
- Procs for this PC (it was remote only): the list, sort, filter, end a
  process.
- Rendered markdown tables (read mode): select some letters of a cell, a
  cell, or a block of cells; Ctrl+C copies, cells tab-separated.
- The command palette matches fuzzily by default; a button beside the
  query switches to text, glob or regex, and is remembered.
- weft asks before closing a file with unsaved changes or a terminal with
  a command still running, also when the window closes or weft restarts.
  Each can be switched off.
- What was open comes back after a restart: the editor's files and how
  each was shown, the folders opened in Files, each panel's folder.

Fixed
- Block characters are drawn by weft, not taken from the font: Claude
  Code's logo had a black band (D2Coding), black bars (Hack) or stripes (a
  proportional font).
- Text stays on the terminal's grid with any font, also one that isn't
  fixed-width.
- Esc closes the composer, the scrollback search and the editor's find bar.
- The hint beside a dragged file no longer covers the folder it would
  drop into.
- Listing, reading and searching local files no longer hold up the window
  (they run on a thread of their own).

Tests
- A regression suite drives the real app without a window
  (docs/regression.md lists what each test covers).
