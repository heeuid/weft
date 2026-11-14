# Changelog

weft's version is in `Cargo.toml` and shown by `weft --version`, Help, and
the palette's "About weft". Each release is tagged `v<version>` in git.
The middle number goes up when something is added, the last one when
things are only fixed.

## 0.3.0

Added
- **Display test** (a link in Help, the palette, the ☰ menu): a window of
  four test screens drawn with the chosen font by the terminal's own
  painter: blocks and a logo, colours and attributes, columns, boxes.
  Each says what to look for, so a font can be checked by eye.

Fixed
- Box drawing characters (light, heavy and double lines, rounded
  corners) are drawn by weft, like the block characters since 0.2.0: a
  font's lines stop short of the row above and below, and frames and
  trees came out broken between rows.
- Esc goes to what has the keyboard. The windows that got Esc in 0.2.3
  took it from a terminal beside them (vim under an open Help) or from
  the palette or a composer on top of them.
- More typed after Ctrl+S, before the save was done, is no longer taken
  for saved; closing weft waits for a save under way.
- A selection in a markdown table stays with that table when another
  scrolls out of view.
- What waited for a remote agent that died fails, and says so, instead of
  waiting for good.
- The editor keeps the layout of the last few files, not of every file
  ever opened.

Tests
- They run side by side: about 20 s for all of them (it was 45).
- The app can be drawn to pixels without a window, and tests look at the
  picture: the logo, blocks and columns in four fonts, box lines that
  must join, the Display test window.
- Eighteen screens drawn by the version before 0.2.0 and by this one
  were compared pixel by pixel: twelve are identical, the others differ
  by what was added.
- docs/regression.md names every test (a test fails otherwise), says
  what each kind covers in numbers (`python tools/testdoc.py`), and
  lists what is left to check by hand.

## 0.2.3

Fixed
- Esc closes Help, Keyboard shortcuts, Privacy, Serial console, the memo
  picker and Fonts (they only had their close button).
- One more way a console host could be left behind: told a cursor place
  outside the screen it was opened with (a terminal that had grown and
  filled before the host asked), it never took the answer. The first
  answer is kept inside that screen.
- A folder with thousands of git-ignored files is listed. Asking git about
  them all at once made each side wait for the other for good, and Files
  never showed the folder.

Faster (nothing looks different)
- A big source file in the editor: 11 ms a frame down to under 2 (8 000
  lines). Its layout is kept until the text, the colours or the fonts
  change, instead of being copied and looked up again every frame.
- Markdown: tables scrolled out of view are not laid out.
- settings.json is written when something changed, not every ten seconds
  regardless (a hitch each time where an antivirus reads every write).

Tests
- More of the screen is driven through the real app: every dialog closes
  on Esc, a file dragged onto a folder, Send to the other Files panel, the
  eye menu, a memo typed and saved, the composer, panels splitting, Procs
  in its panel. The remote agent's file operations are run here by python
  and checked against this PC's.
- `cargo test -p weft frame_times -- --ignored --nocapture` prints how
  long a frame takes in a few heavy situations.

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
