# Changelog

weft's version is in `Cargo.toml` and shown by `weft --version`, Help, and
the palette's "About weft". Each release is tagged `v<version>` in git.
The middle number goes up when something is added, the last one when
things are only fixed.

## 0.11.6

- **Tooltips came up too soon** (half a second, while passing over the
  Files rows), and once one had, every row passed showed its own at
  once. A tooltip comes once the pointer has rested on a thing for 0.8 s,
  and on the next thing only after resting there too.
- A test of a terminal's first lines waited 30 s for a stand-in shell to
  end; a busy CI Mac took longer once (0.11.5 on main): 90 s, as the
  others that start terminals.

## 0.11.5

What weft did on one system only, on every one, and characters it could
not draw:

- **A character weft's fonts lack** (kana, hanzi, Thai, Arabic, …: in a
  terminal, a file, a name) was a box. It is looked for in the fonts the
  system has installed (Windows' and macOS' font folders, fontconfig's
  on Linux), on a thread of its own, and drawn with the face that has
  it from the next frame; one no installed font has stays a box.
- **Deleting a file goes to the trash on macOS and Linux too**, here and
  on remote hosts: macOS' ~/.Trash, the freedesktop.org trash that file
  managers show and put back from (a file on another disk to that
  disk's own). The dialog offers the trash or for good. A remote Windows
  deletes for good, said so (over ssh no one would be asked about what
  its Recycle Bin will not take).
- **A host's key not seen before**, for a connection weft gave a
  password to, is asked in weft's window with ssh's own words and the
  fingerprint, on every system (it was Windows' message box, and on
  macOS and Linux a no: the host could not be reached).
- **Ending a terminal on Windows** ends all it started by a job object it
  is put in as it starts, an orphan whose parent has gone too, at once
  (it ran taskkill, which took half a second and found no orphans).
- **The help in English or Korean**: the system's language at first,
  switched in the help window and kept. Its Korean said the Recycle Bin
  was Windows' only and Downloads a fixed folder.
- On a Mac, a terminal's LANG comes from the system's language and
  region asked of the system, not from a program run for it.

## 0.11.4

Fixed
- **A host that runs its own command was reconnected as a Windows one when
  left** (a ForceCommand's tmux, a bastion's menu: weft's command never
  runs there), and saved as a plain ssh the next time; since 0.11.1, which
  took a session ending with code 0 for a refusal. A refusal is told by its
  words (cmd and PowerShell name exec in quotes) or by an end at once,
  before any prompt: only that is a reason to try another shell now, not
  a menu left with one line (whatever its code), nor a note that ends
  with 0. The same for a host with no base64, openssl or python3 (its message
  is taken as it comes, not from what is left on screen), and for a chosen
  shell that did not start (where it was not there and the login shell ran,
  that one's end is not the chosen one's).
- **Copy mode at the bottom while the pane was made shorter** could make
  its scroll overflow (a crash in a debug build; "18446744… lines below"
  otherwise) until the new size was taken (since 0.11.1).
- **FreeBSD without fdescfs** (/dev/fd/0-2 only): ksh, mksh and sh were
  given their part of the integration on /dev/fd/3, which is not there, and
  `ENV` was left pointing at it. The shell starts as it is there now.
- **A Windows server's refusal after a slow password or a one-time code**
  was not taken for one. cmd and PowerShell name `exec` in quotes, their language's
  ("'exec' is not recognized", "'exec'은(는) …", German's "exec", French
  « exec » with its spaces): that
  tells a refusal now, whatever was typed before. (And the words a host
  without base64 says are no longer in weft's command itself: Windows
  PowerShell prints a refused command back, and they could be taken for
  weft's command having run.)
- **A terminal started again** (Enter once it had ended) with a shell
  chosen for its host that does not start: it just ended, again at each
  Enter. The login shell is used instead, as for a new terminal (a chosen
  bash too, which starts before weft says which shell it starts), also
  after a slow password and a one-time code (judged from when weft's
  command ran, not from when the terminal opened). One typed into and
  then ended with an error is left as it is, and so is a connection lost
  just then (ssh's 255).
- **ksh, mksh, dash, ash and sh:** Enters typed into a running program (cat,
  a password) moved the start of its block's output past what it had
  printed; an Enter on an empty line made a block with the last exit code
  (after a key a program took, `q` in less, or a line emptied with Ctrl-C
  or Ctrl-U too). (A command over several lines, `for` … `done`, now has
  its later lines in its output; a command typed ahead while another ran
  makes no block: these shells say nothing of either.)
- **FreeBSD's and NetBSD's sh** showed `` `__weft_p` `` in every prompt:
  FreeBSD's runs no command in a prompt, NetBSD's only with `promptcmds`
  (now set). Where none runs the prompt is left as it was, without weft's
  marks.
- **bash:** a user's `set -u` made bash before 4.4 (macOS') stop at every
  command ("unbound variable", no command start told); a PROMPT_COMMAND
  array (bash 5.1+) whose later element sets PS1 wiped weft's prompt marks.
  weft's functions now go first and last in the array (from bash 5.1, which
  runs it element by element; before, into its first element as ever).
  A PROMPT_COMMAND ending in `;` (`history -a; $PROMPT_COMMAND` with it
  empty) made a syntax error at every prompt and no marks at all: joined
  with newlines now.
- **Rerun, the palette's commands and Files' "cd here"** were always
  refused ("pane is busy") in a terminal whose shell weft has nothing for
  (tcsh, a plain ssh): its guessed blocks, out of sight since 0.11, were
  not counted as its prompt.
- **Rerun and the palette's commands in a local terminal on macOS and
  Linux** lost the command's first letter (`ls` ran as `s`): Esc, sent to
  empty the line as Windows' PowerShell wants, made a Meta key of it in
  bash and zsh. Ctrl-U there. In Windows' PowerShell and cmd what was
  on the line was never emptied (Esc and the command in one write: the
  console took Esc with the next key): Esc goes on its own now; in a
  Windows host's PowerShell over ssh too (Ctrl-U was sent there, and
  typed in).
- **ssh's password or one-time-code question** was guessed a prompt after
  the pause a person takes to answer it: weft's run or Files' cd could go
  in as the password; the shell counted as started (a chosen shell that
  then ended was not fallen back from; the agent's one retry was spent);
  and in a shell weft has nothing for, the shell's own prompt after it was
  never taken (Rerun and cd refused all session). A guessed prompt that
  is a question (it ends in `:` or `?`) does not count for these.
- **ksh, mksh, dash, ash and sh, while a command ran** (python3, mysql, an
  ssh to elsewhere): the terminal counted as at its prompt, so Files' cd
  here and the palette's commands went into what ran; closing asked
  nothing and the tab showed nothing running.
- **A host that runs its own command** and prints a quoted `"exec"` in its
  session (kubectl's JSON in a bastion's menu) was taken for a Windows
  host that refused weft's command. A refusal is looked for only before a
  prompt comes, within the lines a first connection takes (the host key's
  yes, a mistyped password, the password, a code).
- **A bad option in ~/.ssh/config** (ssh's "Bad configuration option")
  was taken for a shell that refused weft's command: the host was tried
  as Windows.
- **A mistyped password before** (ssh's "Permission denied, please try
  again" on the screen) stopped both the move to PowerShell on a Windows
  host and the fallback from a chosen shell that did not start: ssh's
  words were taken for ssh's own failure.
- **Files' "cd here" and "Insert path" in a local terminal on macOS and
  Linux** typed PowerShell (`Set-Location -LiteralPath …`, "command not
  found") and quoted paths as PowerShell does (a `$`, `*` or `'` in a path
  went in wrong); in a local cmd or nu on Windows too. They go in the
  words of the shell the terminal started now (nu or pwsh chosen on a
  host or on macOS and Linux too: nu has no `cd --`). Also: PowerShell's
  other single quotes (Bob’s) are doubled; cmd gets C:/… in quotes (a /
  is a switch to it); "Insert path" on a Windows host gives C:/…, not
  /C:/…; and the folder tree no longer sends a path to the active
  terminal when that is another host's.
- **A host or container with no $SHELL and no bash** (Alpine run by
  docker or kubectl exec) did not open: sh is used, with its integration.
- **ksh, mksh, dash, ash and sh blocks** offered copy, rerun and edit of a
  command whose text these shells do not tell: rerun sent an empty line.
  Not offered there now.
- **A password asked with a full-width colon** (密码：) was taken for a
  shell's prompt, as "password:" was before.
- **ksh, mksh, dash, ash and sh with a PS1 exported by the user's
  profile:** the shells started from them (`sh`, vi's `:sh`) said
  "__weft_p: not found" at each prompt.
- **A docker, podman or kubectl profile whose container was not running**
  was saved as a Windows host and then as "Other": their "Error response
  from daemon" and "Error from server" are the way in failing now.
- **weft's own runs (Rerun, the palette's commands, cd here) in a local
  cmd** made blocks without the command's text (read off the screen at
  the Enter, before cmd echoed it): not in History, no rerun of them. The
  text weft sends is the block's now; in the sh family too.
- **Records' folder for a quick command** was the next prompt's (a local
  `cd /tmp` was recorded as run in /tmp), in every shell: from bash's,
  zsh's and fish's second command on, and always in ksh, mksh, dash, ash,
  sh and cmd. It is taken at the Enter now.
- **Closing a terminal whose shell weft has nothing for** (tcsh, a plain
  ssh) while a command ran asked nothing, and its tab showed nothing
  running (since 0.11 hid guessed blocks).
- **Records** left out the first commands of a terminal started again
  (as many as it had run before: its blocks are numbered from 1 again).
- **Linux: the UI font, the Korean one and the symbols for TUIs** were
  looked for in Debian's folders only (Fedora's and Arch's keep DejaVu
  elsewhere: Braille showed as boxes). They are found by file name in any
  folder under the system's font folders now, and where none of the
  usual ones is there, as fontconfig picks one for the role (a sans, one
  with Hangul, one with Braille), whatever the distribution or the
  font.
- **Nothing told by another program's words or one system's layout**
  where the system can say it:
  - a folder opened as a file over the agent showed Windows' Python's
    PermissionError in an editor tab (it was told by Linux's exception's
    name): the agent says it is a folder, and Files shows it;
  - a serial port refused was told by the word "denied" (Windows says it
    in its own language) and the advice named the dialout group (Arch's
    is uucp): by the error's kind now, and the port's own group by name,
    locally and over the agent;
  - the terminals waiting at a password when it is given in the dialog
    were told by "password:" (English): by the question's shape, as a
    PAM's in any language;
  - a custom profile's way in failing (docker's container not running,
    kubectl's pod not found) was told by those two tools' words, and any
    other tool's was tried as Windows and saved as "Other": a custom
    profile now moves on only where a shell refused weft's command;
  - the local host name was "localhost" on Linux and macOS (COMPUTERNAME
    is Windows'), the user's name came from one variable and /etc/passwd
    was read by hand: the OS's own host name and user database;
  - downloads went to ~/Downloads (a Korean desktop's is ~/다운로드;
    Windows' may have been moved): the system's downloads folder (Windows'
    known folder, the XDG user dir);
  - serial ports were found by a list of driver names (ttyUSB, ttyAMA, …):
    every tty with a device behind it, and the call-out ones on macOS and
    the BSDs;
  - a remote grep without GNU's or a BSD's options failed the search: it
    is searched in Python then; ps' numbers are read in the C locale;
  - a rename that only changes the letters' case was allowed on Windows
    only (macOS' file system ignores case too): the file system is asked
    whether the two names are one file;
  - the local sh, Git for Windows' bash and weft's own file name were
    fixed paths: sh on PATH, the bash beside git, the system's executable
    suffix; bash's events no longer need the base64 program.
- **weft's runs (Rerun, cd here) in vi mode, or PowerShell's Emacs mode**
  sent Ctrl-U or Esc to empty the line, and the text there, or the
  command, went wrong (in vi's command mode `echo` was taken as vi
  commands). bash, zsh, fish and PowerShell bind a key of weft's own now
  (Ctrl+Alt+Shift+F12) in every editing mode, again at each prompt where
  a change of mode puts back the mode's own keys only (PowerShell's
  Enter mark was lost so too); the others keep Ctrl-U or Esc.
- **fish: a theme's prompt saw status 0** after a failing command (weft's
  marks ran first), and 4 at the first prompt.
- **fish in vi mode: its mode prompt ([I])** was drawn before weft's
  prompt mark and was taken for the last command's output: the prompt
  starts before it now.
- **bash's wls and wtable need no Python in the shell** (they ran a Python
  found at every start, a macOS stub left out by name): wls prints ls'
  text and weft lists the folder through the host's agent, wtable's text
  is split into columns by weft. The shell integration is smaller for it
  (the command that carries it, 6.4 KB from 6.9).
- **A remote shell chosen by name** is looked up on the PATH a command
  gets there, not the login's (Homebrew's on macOS and ~/.local/bin are
  often not on it, and ~ is not expanded): Shells… and the "is not there"
  note say so and ask for its full path.
- **Shells…** put back a local shell set elsewhere while it was open, when
  a remote one was saved; a change back, saved again without closing it,
  was not saved; after a Save it showed the local shell as it was when it
  opened, and a second Save put that back; a new local choice was tried again for the profile named
  "local" only; three of its notes had long runs of spaces.

## 0.11.3

Fixed
- **macOS: ✘ and other dingbats drew as boxes** in terminal programs:
  Apple Symbols has none. Every symbol font of the system is used in turn
  now: on macOS Zapf Dingbats and Apple Braille besides Apple Symbols.

CI, the rest of the first run on every system: the remote side's tests on
Rocky Linux 8 (its Python 3.6 checks the agent by compiling it; `ast`'s
`feature_version` is 3.8+) and Alpine (its login shell, busybox ash, has
part of weft's integration, and is checked as such); a folder's listing
compared in any order (ext4's is not by name); zsh given an empty
`.zshrc` (with none, its new-user menu comes first, for a user that is
not root); bash 3.2 and 4.2 built with their C on gcc 15 (C23).

## 0.11.2

Fixed
- **Linux: closing a terminal could end every program of the user.** weft
  ended a terminal's processes with `kill -TERM -<pid>`, and procps-ng 4.0.4
  (Ubuntu 24.04) reads that as the pid's first digit: -500 went to process
  group 5, and a pid starting with 1 to all of the user's processes
  (browser, editor, the desktop session). It is a system call now, to the
  terminal's own group only. Windows (taskkill) was not affected.
- **macOS' bash (3.2), and bash up to 4.3 (CentOS 7's 4.2):** commands ran
  without their start being told, so a command's text and its output were
  one, and its exit code was missing; on 3.2 the integration did not load
  at all (it cannot read its startup file from a pipe). The bash that
  starts is now the login shell's own, not the first on PATH (a Homebrew
  bash 5 gave way to /bin/bash 3.2 over ssh).
- **Linux drew weft's whole UI in a fixed-width font** (the icon font): the
  UI font (DejaVu Sans) and Korean fonts (Nanum, Noto Sans CJK) were looked
  for one folder too high. Without a Korean font on the system Hangul was
  drawn as boxes; the built-in D2Coding has it now. Braille and other
  symbols come from DejaVu Sans on Linux and Apple Symbols on macOS.
- **The editor's find bar** sized its field for Windows' font: with a wider
  one (or a long error in it) the bar's × went to a second line. The field
  now gives way to what the rest of the bar takes.
- **History's Records… button** was cut off at the edge of a narrow side
  panel; the row goes on to a second line.
- **A serial port that does not open** said "No such file or directory" on
  Linux and macOS, and "in use by another program" where the user is not
  in the port's group; weft says which (and how to join it on Linux).

CI runs on the Rust it is checked with here (1.96.0; a newer one is tried
apart, not failing the run), builds bash 3.2 and 4.2 for the ssh test, and
its Windows job makes its key without a passphrase (PowerShell 7 passed
`'""'` as two quote marks).

## 0.11.1

Fixed
- **A Windows server reached over ssh got "'exec' is not recognized" and
  the connection closed** (issue #3; from macOS or Linux, and from Windows
  too). Windows' OpenSSH ends a session that has a terminal with code 0
  whatever ran in it, so its cmd refusing weft's POSIX command looked like
  a shell that had ended well, and PowerShell was never tried. weft's
  command now says first thing that it runs; where it did not, the next
  shell is tried, whatever the code. Ctrl-C at ssh's password prompt is
  not taken for a refusal.
- **A terminal's first lines went out of sight under the tab bar**
  (issue #4, seen on macOS). A new pane size is taken once it has held for
  a moment; until then the text was fitted from its bottom, and where the
  height moved by half a pixel every frame (a Mac's layout) it stayed so.
  The same at every change of size: the font made larger (zoom), a split,
  the side panel opened. It is now shown as the terminal will be, its
  blank rows below the cursor left out first.

## 0.11.0

Added
- **"Shells…"** (command palette): what weft has in each shell (all of
  its integration, part of it and what is not there, none), where each can
  be used (local on Windows, local on macOS and Linux, remote), and the
  shell to use:
  - **on a remote host**, instead of its user's login shell (a name on its
    PATH, or a path; kept per host, settings `"remote_shells"`): zsh or
    fish where the login shell cannot be changed, say.
  - **on this computer**: Windows' choices as before; on macOS and Linux a
    name or path (settings `"unix_shell"`; empty: the login shell).
  A chosen shell that is not there, or does not start: the login shell is
  used instead, and weft says which was chosen and why not.
- **ksh, mksh, dash, busybox ash and a plain sh have part of weft's
  integration** (`shell/sh.sh`): each prompt, the exit code of what ran
  before it and the folder, all exact. These shells run nothing before a
  command, so the command's own text is not known: it is not made up,
  and no history is kept of it.
- **What a shell does not have is said**, once a run for a host and
  shell, with where to look ("Shells…"): the parts not there in a shell
  with a partial integration; all that a shell weft has nothing for goes
  without (tcsh, xonsh, elvish and the like: a terminal as any other).
  Where none of the shells weft supports starts on a host, that is said.

Changed
- **Command blocks guessed from the output are no longer shown** where a
  shell has no integration (unknown shells, a plain ssh, serial consoles):
  a guess can be wrong, and a wrong block or history line is worse than
  none. The folder is still followed through the agent where weft's
  command started the shell (not a plain ssh or a serial console). Settings
  `"guess_blocks": true` shows them as before.

Fixed
- **fish 4: a command was cut into two blocks and lost its exit code.**
  fish 4 sends the prompt and command marks (OSC 133) itself, and weft's
  script sends them too, so each came twice; the second "command started"
  began another block. A mark told twice at the same place is now one
  (so is one from a prompt tool beside a shell's own integration).

Tested with a real zsh (5.9.2) and fish (4.9.3) for the first time, as the
chosen shell and as the login shell: MSYS2's packages in a folder of their
own beside a copy of Git's shell (`WEFT_TEST_SH`, docs/shell-integration.md).

## 0.10.9

Fixed
- **A host with none of the tools weft's command unpacks itself with**
  (base64 or openssl, gzip and awk; or python3: bare containers, some
  BSDs) gets the user's shell as it is, and the terminal says why. The
  command ended at once without an error there, so the terminal closed
  without a word and nothing was tried next. The agent says so and ends
  as failed. `openssl base64` is one more decoder tried.
- **macOS' `/usr/bin/python3` is not run where the Command Line Tools are
  not installed**: there it is no Python, it only asks to install them, in
  a window on that Mac, each time weft looked for Python (the agent, and
  bash's w* commands). "No Python found" says what to do there
  (`xcode-select --install`, or a Python from python.org or Homebrew).

## 0.10.8

Fixed
- **A local terminal on macOS and Linux has weft's shell integration**
  (prompt marks, exit codes, the folder followed, `wopen` and the other
  w* commands), as a remote one has: the user's shell (`$SHELL`) is
  started by the same command as on a remote host, without ssh, with
  `shell/bash.sh`, `zsh.zsh` or `fish.fish`. It was the bare shell
  (`$SHELL -i`), only guessed at.

## 0.10.7

Fixed
- **bash on a remote host reads the startup files a plain ssh login
  does** (issue #5): the system's (`/etc/profile`, and the
  `/etc/profile.d` scripts it runs) first, then the first of
  `~/.bash_profile`, `~/.bash_login` and `~/.profile`, and
  `~/.bash_logout` as it ends. weft starts bash with its integration as
  the rc file, which takes the place of bash's own startup files, and read
  only the user's: where `~/.bashrc` is loaded from `/etc/profile.d` (no
  `~/.bash_profile` of the user's), it was not, and the prompt was bash's
  bare `bash-4.4$`.

## 0.10.6

Fixed
- **Windows ssh hosts get weft's terminal and its Files, editor and
  search** (issue #3), not only a plain ssh. A host's shell is found by
  trying, and kept with its profile (`"shell"`): weft's POSIX command
  first; where it cannot run (cmd says "'exec' is not recognized…" in its
  own language), Windows': PowerShell, with weft's integration (prompt
  and command marks, the folder followed, a tab's folder kept), started
  from cmd or PowerShell alike; where neither runs, a shell weft does not
  support: a plain ssh, said so. A kept shell that fails is not the end:
  the ones not tried yet in that run are (a host put in again with
  another system), and one kept as unsupported is tried again once a run,
  as its first terminal opens. Terminals of a host failing together are
  one failure, not one per shell. ssh's own failures (no connection, keys
  or password refused) are told by ssh's own words, which are English
  everywhere; its exit code could not tell them (cmd runs the pieces of
  weft's POSIX command it can and ends with whatever the last one did,
  255 too), read half a second after the terminal ends (Windows' console
  passes the last output on after the process has ended). A profile from
  0.10.5 marked "plain" is a Windows host.
- **The agent on a Windows host** finds Python by the same list as on any
  host (Python on remote hosts…; `py` is in the default list now) and
  says the same when there is none, with Windows' check (`where python`).
  There it searches without grep, lists processes, memory and disk the
  Windows way, and takes PowerShell's folders (`/C:/…`). The terminal's
  "go to this folder" and quoting speak PowerShell there. A serial console
  through a Windows host says it needs a POSIX one (the bridge is POSIX).
- What weft puts on a command line is checked to stay under 8000 bytes
  (Windows' console takes about 8 KB, cmd 8191 characters); the agent
  itself goes through ssh's stdin, of any size.

## 0.10.5

Fixed
- **ssh to a Windows host** ended at once with cmd's "'exec' is not
  recognized… not a batch file" (in its own language), and "Connection
  closed" (issue #3). weft's terminal starts its shell integration with a
  POSIX command (`exec sh -c …`), which Windows' cmd or PowerShell cannot
  run. When a terminal of an ssh host ends before that integration
  started, with the code of the command (not ssh's own 255: no
  connection, keys or password refused), the host's shell is taken to be
  not POSIX: the terminal starts again as a plain `ssh -tt`, which works
  (without weft's shell integration: prompt marks, the folder followed),
  and the profile keeps it (`"plain": true`), so it opens plain at once
  from then on. The Files panel, editor and search say they do not run
  there (they need sh and Python 3), instead of cmd's words. A profile
  made in Connect is kept then: it did connect.

## 0.10.4

Fixed
- **A terminal whose program ended hid its first line** under the tab bar,
  cut in two (issue #4). Its text is raised above the "process exited"
  notice so the notice covers none of it; it was raised as if the text
  reached the bottom, counting the blank rows below it, so three lines at
  the top were raised too, and by part of a row. Now only as far as the
  text reaches (its last line that is not blank, or the cursor's), and by
  whole rows: no line is left cut at the top.

## 0.10.3

Fixed
- **Black frames on Windows after the PC slept or locked.** weft drew
  with OpenGL, and after the PC had slept or locked a few times its
  window showed black frames whenever it redrew (a key, the mouse), until
  weft was started again; minimizing it did not always help. What weft
  drew was the same frame after frame: it was OpenGL's way to the screen.
  weft now draws with the steadiest graphics backend for the system:
  wgpu on DirectX 12 on Windows, wgpu on Metal on macOS (OpenGL there is
  deprecated), OpenGL on Linux (it runs where Vulkan has no driver).

Added
- **Graphics backend…** (palette): auto (the system's steadiest), wgpu
  (DirectX 12 / Metal / Vulkan) or OpenGL, kept in settings ("renderer")
  and used from the next start; "Save and restart" starts again at once.
  It shows what draws now. WEFT_RENDERER sets it for one start. If wgpu
  cannot start (no driver for it), weft starts again on OpenGL and says
  so in its log.
- The log says what draws (`graphics: wgpu Dx12: <GPU> ...`) and each
  change of the window's scale or monitor (`display: ...`).

## 0.10.2

Fixed
- **The wheel at the tab strip's bottom edge** (over its thin scroll bar,
  under the tabs) still only shook it in place: weft took the wheel over
  the tabs alone, so there egui moved the strip less than a tab and it
  went back to the nearest tab's edge. The strip's scroll bar is part of
  it now: a notch there moves one tab too.

## 0.10.1

Fixed
- **The mouse wheel over the tab strip** hardly moved it: a notch scrolled
  less than half a tab, and the strip, resting on the nearest tab's edge,
  went back where it was. A notch now moves it one tab; smaller turns (a
  touchpad's) add up to one. The scroll bar still drags it, to rest on an
  edge.

## 0.10.0

Added
- **Tabs out of sight are counted.** When there are more terminal tabs
  than fit (they scroll sideways), the tab bar shows how many are out of
  sight on each side, `‹ 3` and `5 ›`, and how many there are in all
  (`12 tabs`); the two counts are each side's own (`‹ 4`, `5 ›`). An
  arrow brings the next hidden tab into view without switching to it; an
  arrow with none to show is dimmed. The strip shows whole tabs and comes
  to rest on a tab's edge, after an arrow, a wheel or the scroll bar (a
  tab was left cut in two at an end, its dot gone). With room for every
  tab, none of it is shown.

## 0.9.3

Fixed
- **Markdown lists in a narrow pane.** A list item's wrapped rows went
  back to the left edge, and a first word too long for its row (a link)
  went whole to the next row there, leaving the item's marker on a row a
  hair tall with its bullet over the row above. A list item's text now
  wraps under itself (a hanging indent, as on GitHub), and a word too
  long is cut where it starts. Wide panes lay out as before.

## 0.9.2

Fixed
- **Connect keeps a profile once it has connected** (issue #2). A try was
  saved as a profile when Connect was pressed, whether or not it got in,
  and a profile is found again by its name: one made for
  `dream@10.20.20.54 -p 22008` (named by the host alone, before 0.8.1)
  was what the same address typed for a Mac on port 22 found, so it went
  to 22008, where the Mac's password was refused. Now a new profile is
  used at once but written to settings (with its tabs) only when a
  terminal of it shows its prompt or its agent comes up; a try that never
  does is gone after a restart. Profiles already saved are left as they
  are.
- **Enter in the password dialog** did nothing; it is Connect now.
- **One password for a session's terminals.** After a start, every
  terminal of a host that wants a password waited at ssh's own prompt,
  and the password given in the dialog went to the agent only: each had
  to be typed again. Those still waiting (or ended) now start again with
  it; one that got in is left as it is.
- **Hosts in "double quotes" in ~/.ssh/config** were handed to ssh quotes
  and all, which matched no Host; they are read as ssh reads them, a name
  with spaces as one name. A Host picked from Connect's list is that name
  whole (it was split at spaces like typed text).

## 0.9.1

Fixed
- **A host with no Python said only the end of why.** Its reason ran over
  several lines and the panels kept the last: "agent offline:
  /opt/homebrew/bin/python3). Check that…", and the status bar's
  "python…" button did not show. The agent's command is run as `eval` of
  it unquoted, which made the newlines written in it spaces (in `tr` and
  `printf`), so the list it had tried was printed a line each. The reason
  is one line now, what is wrong first: "weft: no Python 3.6 or newer
  found on this host: the Files panel, editor and search need one. Check
  python3 is installed there (command -v python3), or add its name or path
  in Python on remote hosts (command palette). Tried: …". weft's own
  reason stays over what ssh says after it. Finding Python was not
  affected.

## 0.9.0

Added
- **"Python on remote hosts…"** (palette): the Pythons tried on a remote
  host for the Files agent and serial consoles, in order, a line each: a
  name on its PATH or a path, `*` allowed (/opt/rh/*/root/usr/bin/python3).
  The first that runs as Python 3.6 or newer is used, so a Python 2 named
  python is passed over. Defaults: python3, python,
  /usr/libexec/platform-python, /usr/local/bin/python3,
  /opt/homebrew/bin/python3. Saving connects again the hosts whose agent
  failed.
- **When none is found** the agent says what it tried and what to do:
  check python3 is there (`command -v python3`) or add its name or path
  to the list. The status bar shows the whole reason on hover and a
  "python…" button that opens the list.

Fixed
- The agent and the serial bridge run on Python 3.6 (RHEL / Rocky 8's
  platform-python): lsof was called with 3.7's arguments.
- The windows Esc closes are kept in a 32-bit mask: the 8-bit one was full
  with this window and overflowed with one more.

## 0.8.3

Fixed
- **On a Mac, a host that wants a password showed "agent offline: send
  agent: Broken pipe" in Files** instead of asking for it (issue #1). With
  keys refused, ssh ends at once; a Mac's pipe holds 16 KB, less than the
  agent weft sends, so sending it failed and what ssh said was dropped.
  Now ssh's own words are the reason ("Permission denied…"), and the
  password dialog opens as on Windows and Linux.
- **Files on RHEL / Rocky Linux 8**: a minimal install has no `python3`
  command, only /usr/libexec/platform-python; the agent (and a serial
  console's bridge) now use it. With no python at all the panel says so
  instead of ending without a word.

## 0.8.2

Fixed
- **install.sh no longer installs an old build as the new version.**
  When cargo was not on the shell's PATH it built nothing and quietly
  installed the weft built before (a Mac had 0.6.1 after a pull). It now
  looks in ~/.cargo/bin too, says when it builds nothing, and refuses a
  target/release/weft whose `--version` is not the sources' version.
  When weft is running it says to quit it and open it again, since the
  one running stays the version it started as.

## 0.8.1

Fixed
- **The same host on another port connects to that port.** Connect named
  the profile it made by the host alone, so `user@host -p 22024` after
  `user@host -p 22008` found the first one's profile and went to 22008.
  Profiles made now are named `user@host:22024`; one already made for the
  same host and options is used again, whatever its name.
- **`ls` on a Mac shows Korean names, not `????`.** weft started from the
  Finder or the Dock has no locale, and neither had its terminals. When
  none is set, a terminal now gets the Mac's own (`ko_KR.UTF-8`, as
  Terminal.app does; en_US.UTF-8 if there is none), or C.UTF-8 on Linux.
  One that is set is left alone; Windows is unchanged.

## 0.8.0

Added
- **Split panes resize from the keyboard**: Alt+Shift+←/→/↑/↓ move the
  border of the active pane's nearest split that way, 5% at a time, as
  far as a drag goes (10% to 90%). Like moving between panes (Alt+arrows)
  they are weft's only while the tab is split; otherwise they reach the
  program. The size is kept with the session. Four commands ("Resize: move
  the pane border …"), so the keys can be changed in Keyboard shortcuts.

Changed
- **Next / previous tab is Ctrl+Tab / Ctrl+Shift+Tab** (and Alt+number).
  Alt+Shift+←/→ did the same and now resize panes. Keys you set yourself
  for these commands are kept.

## 0.7.0

Added
- **MIT license** (LICENSE, and `license = "MIT"` in the manifests); the
  README lists what comes with weft under its own license (the fonts,
  Catppuccin's colours).
- **Each search opens as it was last set**, also after a restart: the
  Search panel's mode, ignore case and hidden; the editor's find mode and
  match case; Aa of find in a terminal. (Only the Files filter, the
  History panel and the palette kept theirs.)

Fixed
- **Glob without a wildcard found only a whole name that was all of
  it**: "ssh" in the History panel did not find "ssh user@host", "main" in
  the Files filter not "main.rs". Text with no `*`, `?`, `[` or `{` is now
  found anywhere in it; a pattern is still the whole name (`*.rs`,
  `cargo*`).
- Names shown composed (0.6.3) now only put marks together with their
  letters; a character that already is one stays exactly the one it is
  (a full NFC also turned compatibility characters, such as the hanja of
  Korean names at U+F900, into others). Checked over many scripts.
- **Japanese voicing marks as a Mac writes them** (が as か and a mark)
  took a cell of their own in the terminal; they go on their letter.

## 0.6.3

Fixed
- **Korean names from a Mac showed letter by letter** ("응용프로그램" as
  ㅇㅡㅇㅇㅛㅇ…): macOS hands out file names with Hangul (and accents)
  decomposed, and they were drawn as given. Wherever a name or path is
  drawn (Files tree and path, filter and search results, terminal tabs
  and pane titles, the status bar, the editor's tabs and path, copies,
  the file and close dialogs, file lists in the terminal) it is now
  shown composed; the name itself is kept as the system gave it, so
  opening, renaming and copying still find the file.
- **In the terminal, such letters and accents were dropped**: `ls` on a
  Mac showed only each syllable's first consonant, and "café" written
  with a separate accent showed "cafe". They are now put together with
  the letter before them.

## 0.6.2

Changed
- **`./install.sh path` puts weft on the PATH itself**, as install.bat
  does on Windows: when `~/.local/bin` is not on it yet, one marked line
  goes into your shell's file (`~/.zshrc`; for bash `~/.bashrc`, on a
  Mac `~/.bash_profile`; for fish a file of its own; else `~/.profile`),
  and it says which file got which line. uninstall.sh takes that line
  out and leaves the rest of the file as it was. (It only said to add
  the folder somewhere.)
- Without `path`, install.sh says how to get the `weft` command.

## 0.6.1

Fixed
- **uninstall.sh after weft.app was put in the Trash** (or, on Linux,
  the program deleted) said weft was not installed and stopped, leaving
  the launchers, links and data. It now clears up what the install left
  (it knows from `.installed`, or from the data folder).

## 0.6.0

Added
- **install.sh / uninstall.sh for Linux and macOS**, as install.bat /
  uninstall.bat are for Windows: for this user, no root.
  - Linux: weft, its icon and `data/` in `~/.local/opt/weft`, and an
    entry in the applications menu.
  - macOS: `weft.app` and `data/` side by side in `~/Applications/weft`
    (Launchpad, Spotlight, Dock); the app keeps its data in that `data/`,
    not inside itself, so replacing the app leaves the data alone. Built
    on that Mac, it opens without a security warning; its icon is made
    with macOS's own sips and iconutil.
  - `desktop` adds a desktop launcher, `path` a `~/.local/bin/weft` link;
    running it again updates (a weft that is running goes on); data from
    `./weft.sh` is brought along the first time.
  - uninstall.sh takes away only what the install made (listed in the
    folder's `.installed`), waits for a running weft to close, and asks
    about the data (`purge` / `keep` to say beforehand).
- `tools/install_test.py`: both scripts, for both systems, in a home
  folder of their own with a stand-in weft.

Not yet seen
- weft itself has only been compiled for Linux and macOS, never run
  there: that the menu entry or weft.app opens a window is to be seen on
  a real desktop.

## 0.5.1

Changed
- **Bullets go ● ○ ■ □ by depth** (they went ● ■ ○), a little larger: at
  a small size a dot and a square are hard to tell apart, filled and
  hollow are not, and next to each other they now always differ.

Fixed
- **When a terminal's program ended, its notice covered the last line**
  ("process exited — Enter: restart …" over Claude Code's status line,
  after an ssh connection dropped). The text is drawn higher, its last
  line just above the notice; text that does not reach that far stays
  where it is.

## 0.5.0

Changed
- **Markdown lists show their nesting.** Each level is indented by the
  width its marker needs, the items inside an item starting under its
  text, whatever spaces the source has (a level was about one space
  wide). Bullets go ● ■ ○ by depth, numbers 1. i. a., and again. Items
  are numbered as they come from the first one's number, so `1.` written
  for each still shows 1, 2, 3. A task's box stands in for its bullet.
- **What nests is what nests on GitHub** (CommonMark): an item is inside
  the one before when indented to that one's text: two spaces under
  `- `, three under `1. `. One space under `- ` is the next item of the
  same list (it used to look nested, and on GitHub it is not).
- A paragraph of an item lines up under the item's text; a line that
  runs on from the one before no longer keeps its indent as a gap.
- On the cursor's line the source markers show (faint) in the same room:
  the text does not move when the cursor comes to its line.

## 0.4.2

Fixed
- **Focus mode's "click to leave" label covered a split pane's zoom and
  close buttons**, and with a pane zoomed it was drawn over zoom's
  "click to restore" label. It has a strip of its own above the panes
  now; a click on either label still does what it says.
- **The cursor a program hides while it redraws could still go off on a
  busy PC.** It is held where it was for 150 ms (0.3.4), counted from the
  last frame drawn with it shown; when the program showed it and hid it
  again between two frames, that count had run out. The hold now starts
  when the program hides it, taken as its output is read (the terminal
  keeps where and when: `Term::hidden_from`).

## 0.4.1

Fixed
- **The tab bar's buttons covered each other in a narrow window**: the
  tab's + and connect buttons were drawn under the command palette and
  split buttons (at 330 px wide). The tabs never take more than is left
  now, and the buttons on the right that do not fit beside one tab are
  left out, the least needed first (palette, splits, side panel, editor,
  theme, Aa, ?); every one of them is still in the ☰ menu.

## 0.4.0

Added
- **Record this terminal** (palette, Help group): writes what the program
  in the terminal sends, what you type into it, and where weft draws the
  cursor, each with its time in milliseconds, to
  `data/logs/terminal-<host>-<date>-<time>.log`; run it again to stop.
  The status bar says `● recording N KB` meanwhile. It stops by itself
  at 32 MB. The file holds what you typed and what was on screen: look
  at it before you pass it on.

Found with it
- On Windows the program's output does not reach weft as it was written:
  the console host draws the program's screen again and sends that. A
  cursor hidden and shown again around a redraw came as plain text with
  no hide or show at all. The tests of 0.3.4 and 0.3.6 sent hide and
  show as a program would and took it that weft gets them so; what
  Claude Code's screen comes to through the console host is still to be
  recorded.

## 0.3.6

Fixed
- **The cursor still flickered when scrolling while a program was
  working** (Claude Code with its spinner going). The cursor a program
  hides while it redraws is held for a moment (0.3.4), but that hold was
  kept as a place on the screen: every turn of the wheel dropped it, and
  it was not looked at when the line the program was drawing on (the
  spinner's) was out of view. In a test that scrolls back and forth over
  four lines during seventy redraws the cursor was missing in 180 of 603
  frames. The hold is now a line and column of the text, found again
  wherever the view has scrolled: 0 frames.

## 0.3.5

Fixed
- **The cursor went off and on while scrolling** near the end of the
  terminal: it was not drawn once the view was scrolled back by even one
  line, though its line was still on screen (Claude Code's input line
  has a few lines below it). It now stays on its line and moves with the
  text, as in other terminals, and goes only when that line leaves the
  screen.

## 0.3.4

Fixed
- **The cursor flickered while a program was working** (Claude Code's
  spinner, any program that redraws part of its screen many times a
  second). Such a program hides the cursor, draws, and shows it again
  where it was; a frame weft drew in between had no cursor. In a test
  that redraws forty times a second it was off in four frames of ten.
  A cursor the program hides now stays where it was for 150 ms; shown
  again within that, it never went. One that stays hidden goes, 150 ms
  later than before. weft's cursor does not blink: it is on or off.
- **Files did not follow the terminal when you went to another tab**
  (with "follow the terminal's directory" on): a click on a tab, Alt+1…9
  or Alt+Shift+←/→ left Files in the folder of the tab before; only a
  `cd` moved it. A tab click is handled while the panels are being
  drawn, and what it changed was undone at the end of that frame. Files
  now also goes to the terminal that has the keyboard after a tab or
  pane is closed, and to a new tab's folder; a shell that has not said
  where it is yet counts as being where it was started.
- **The Search tab's button was cut off** at the edge of a side panel as
  narrow as it starts out ("Se"). The row goes on to a second line.

## 0.3.3

Fixed
- **Windows: several widgets shown in one go were drawn on top of each
  other**: a script that runs `wls`, `wprogress` and `wchoose` one after
  the other, or any program that shows more than one, left only the last
  to be seen, on the first one's lines. Windows' pseudo console hands a
  program's events on at once and its text when it next draws, so each
  widget came with the cursor still on the first one's line. A widget
  that comes while the cursor is where the one before it is to be now
  goes below that one. (0.3.2 listed this as known and left it.)

## 0.3.2

Fixed (each found by a new test, and held by it)
- **Find in scrollback**: the first Enter after typing went to the second
  hit. It goes to the first.
- **Find in the editor**: Shift+F3 went to the next match. It goes to the
  one before, as its button says.
- **The editor's find bar in a narrow editor** ran on past the edge of
  the window, and its × with it. The field gives way first, and what
  still has no room goes on the next line.
- **Folding or unfolding a command while scrolled back** moved that
  command from under the pointer, or off the screen. What is above the
  fold now stays where it is. (At the end of the terminal nothing
  changes.)
- **Ctrl+click in the empty space right of a scrollback line** opened
  the path the line ended with. Nothing is there, and nothing opens.

Tests
- Twenty new screen tests and two unit tests, for parts no test reached:
  the Keyboard shortcuts window (binding a key by pressing it), the tab
  bar, the buttons beside it and the ☰ menu, the commands one by one,
  the status bar, the mouse in a terminal (wheel, selection by
  character, word, line and block, Ctrl+click), command blocks and their
  buttons, find in scrollback, widgets from the shell, what weft types
  into a terminal, the Files filter, the Search, Commands and History
  tabs, the memo picker, the theme menu, the Fonts and Privacy windows,
  find and replace in the editor, a file changed on disk, the editor's
  tabs, the terminal's plain escapes and the word under the pointer.
- The test harness finds an icon button by its tip, holds modifier
  keys, hovers, turns the wheel; a test terminal can run a script that
  behaves like a shell and writes down what it is sent.
- docs/regression.md: each of them listed; the numbers measured again.

## 0.3.1

Changed
- **Windows: terminals run on Windows' own console host** (conhost.exe).
  Until now weft used whichever `conpty.dll` was found on PATH, which on
  a PC with WezTerm installed was WezTerm's: weft behaved differently
  from one PC to the next, and changed when that program was updated.
  A `conpty.dll` is still used when you give one: put it (with its
  `OpenConsole.exe`) beside `weft.exe`, or name it in `settings.json`:
  `"console_host": "C:/path/to/folder-or-conpty.dll"`. weft ships none.
  About weft, Display test and the log say which one is in use.
  If something feels different in the terminal after this update (typing
  Korean, redrawing), the line above with WezTerm's folder brings back
  what you had.

Fixed
- The integration tests left a console host behind (they open terminals
  without an app around them). The terminal tests now count this
  process's console hosts themselves and fail if one is left.

Checked
- The whole workspace and its tests compile for Linux and macOS without
  a warning (`cargo check --workspace --all-targets --target …`); what is
  specific to Windows is built on Windows only. They have still not been
  run there.

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
