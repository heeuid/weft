# Tests: what is covered, and what is not

Every feature, and every bug that was fixed, has a row here with the tests
that would fail if it broke. A new test goes in with a row: the test
`doccheck::every_test_is_in_the_document` fails when a `#[test]` in the
source is not named in this file.

```
cargo test --workspace                                  # everything but the integration tests: about 20 s
cargo test --workspace -- --ignored --test-threads=1    # the integration tests (real shells; need Git Bash): about 2 min
cargo clippy --workspace --all-targets                  # must report nothing
cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu   # compiles for Linux, without a warning
cargo check --workspace --all-targets --target aarch64-apple-darwin       # and for macOS
```

(The last two need the targets: `rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`. They check that what is specific to one system is built only there; they do not run anything.)

## The kinds of test

| Kind | What it is | Where |
|---|---|---|
| Unit | One function or type at a time: the terminal's parser and grid, markdown layout, search, file operations, key names… | `mod tests` in each file |
| Screen | The whole app without a window: the frame the window runs (`App::frame`), fed clicks, drags and keys, checked by the app's state and by what it drew (texts and their places) | `src/regress.rs` with `src/testkit.rs`, and a few in `app.rs`, `ui_panels.rs`, `ui_term.rs`, `transfer.rs` |
| Pixel | Screen tests that look at the picture: the frame is filled in as an image (`src/shot.rs`), no window or GPU needed, and colours are checked (a logo's eyes, lines that must join) | in `src/regress.rs`: the logo, Display test, box drawing |
| Integration | Real shells and the remote agent end to end: integration scripts sent in memory, prompts and command blocks recognised, the agent fed through stdin | `host::loopback::*`, ignored by default |
| Measurement | Not pass or fail: frame times in heavy situations (`regress::frame_times`), eighteen screens as pictures to compare two versions (`scenes::scenes`) | run by hand, see Tools below |

## In numbers

<!-- numbers:begin -->
Measured 2026-11-14 at commit 2f4e62e by `python tools/testdoc.py` (it rewrites this part).

**How many tests**

| Kind | the app | weft-core | weft-draw | together |
|---|--:|--:|--:|--:|
| Unit | 59 | 64 | 31 | 154 |
| Screen (the whole app, no window) | 64 | 0 | 0 | 64 |
| Integration (real shells, the agent; run with `--ignored`) | 1 | 13 | 1 | 15 |
| **All** | 124 | 77 | 32 | **233** |

**How much of the code they run** (lines run / lines there are; test code itself left out)

| Crate | lines | unit | screen | integration | all together |
|---|--:|--:|--:|--:|--:|
| weft (the app) | 11320 | 33 % | 71 % | 1 % | 83 % |
| weft-core | 4476 | 75 % | 43 % | 28 % | 88 % |
| weft-draw | 2238 | 82 % | 49 % | 7 % | 93 % |
| **everything** | 18034 | **50 %** | **61 %** | **8 %** | **85 %** |

By file (all kinds together), least covered first:

| File | lines | run | not run |
|---|--:|--:|--:|
| `src/main.rs` | 135 | 2 % | 132 |
| `src/x11.rs` | 49 | 47 % | 26 |
| `src/restart.rs` | 61 | 48 % | 32 |
| `src/mermaid.rs` | 326 | 54 % | 151 |
| `src/media.rs` | 254 | 59 % | 104 |
| `src/serial_ui.rs` | 174 | 65 % | 61 |
| `src/transfer.rs` | 509 | 79 % | 106 |
| `src/ui_term.rs` | 2005 | 83 % | 337 |
| `crates/weft-core/src/pty.rs` | 375 | 84 % | 61 |
| `crates/weft-core/src/term.rs` | 1590 | 84 % | 253 |
| `src/ui_panels.rs` | 4334 | 86 % | 603 |
| `src/app.rs` | 2149 | 86 % | 294 |
| `src/widgets.rs` | 412 | 87 % | 55 |
| `crates/weft-draw/src/theme.rs` | 441 | 88 % | 52 |
| `crates/weft-core/src/host.rs` | 1613 | 88 % | 189 |
| `src/confirm.rs` | 233 | 88 % | 27 |
| `src/copymode.rs` | 211 | 89 % | 24 |
| `crates/weft-draw/src/highlight.rs` | 428 | 91 % | 40 |
| `src/fileops.rs` | 179 | 93 % | 13 |
| `crates/weft-draw/src/md.rs` | 960 | 94 % | 62 |
| `crates/weft-core/src/find.rs` | 186 | 95 % | 10 |
| `crates/weft-core/src/secret.rs` | 274 | 95 % | 14 |
| `crates/weft-draw/src/icons.rs` | 183 | 95 % | 9 |
| `crates/weft-core/src/records.rs` | 257 | 96 % | 9 |
| `crates/weft-core/src/editfind.rs` | 181 | 97 % | 6 |
| `crates/weft-draw/src/keys.rs` | 112 | 98 % | 2 |
| `src/doccheck.rs` | 59 | 98 % | 1 |
| `crates/weft-draw/src/fontfix.rs` | 114 | 99 % | 1 |
| `src/display_test.rs` | 149 | 100 % | 0 |
| `src/frame.rs` | 61 | 100 % | 0 |
| `src/version.rs` | 20 | 100 % | 0 |
<!-- numbers:end -->

What the numbers don't show:

- **A line that ran is not a line that was checked.** Coverage says the tests went through the code; whether they would notice it going wrong is what the tables below are for (each row names the test that fails).
- **UI code is covered by the screen tests almost alone.** `ui_panels.rs` and `ui_term.rs` are drawn by the tests, but many branches are a menu entry or a state seen only with a real host; those show as lines not run.
- **Nothing here runs on Linux or macOS.** The code for them (ptys, signals, paths) is compiled out on Windows and is not in these numbers at all.
- **Pictures are checked where a test looks, not everywhere.** The pixel tests assert on a logo, blocks, columns and box lines; the rest of a screen is compared only when the scenes of two versions are diffed by hand.

## Terminal

| What | Test |
|---|---|
| Output, wrapping, scrollback, absolute lines | `term::tests::prints_wraps_and_scrolls_into_history`, `clear_screen_keeps_absolute_lines` |
| Colours and attributes (SGR); the alternate screen; cursor reports | `term::tests::sgr_colors`, `alt_screen_keeps_primary`, `dsr_reply_and_token` |
| Hangul two cells; never split at a row end | `term::tests::hangul_is_two_cells_wide`, `reflow_never_splits_a_wide_char` |
| Letters and the marks after them (Hangul as macOS writes names, accents) make one character in the grid; a mark with nothing to go on is left out | `term::tests::decomposed_letters_are_put_together` |
| Text shown with marks put on their letters (Hangul and kana as a Mac writes them, accents), a character at a time with the given ones it stands for | `tests::decomposed_text_is_shown_composed` |
| Text already composed, of every script (CJK, Arabic, Hebrew, Devanagari, Thai, Vietnamese, emoji with joiners, compatibility characters), is left exactly as it is | `tests::composed_text_of_every_script_is_left_alone` |
| Names from a Mac show as syllables in the Files tree and path, the terminal's tab, the editor's tab and path and the filter's results; the file opens by its own name | `regress::names_from_a_mac_show_as_syllables` |
| Resize reflows, blocks move with lines | `term::tests::narrowing_then_widening_restores_the_prompt`, `reflow_moves_blocks_with_their_lines`, `resize_shrink_pushes_to_history` |
| Scrollback stores no trailing blanks (memory) | `term::tests::scrollback_keeps_no_trailing_blanks` |
| OSC 133 blocks; guessed without it | `term::tests::osc133_blocks_capture_command_and_exit`, `term::guess_tests::*` |
| OSC 7 / 9;9 directory, OSC 7777 events, OSC 52 | `term::tests::osc7777_base64_event_and_osc7`, `osc_9_9_is_the_current_directory`, `clipboard_support_is_reported` |
| Leading space keeps a command out of history | `term::tests::leading_space_makes_a_command_private` |
| Plain escapes: save and restore the cursor, index, next line, reverse index (the lines move down at the top, in a region only its lines), CSI T, the line drawing set, reset | `term::tests::plain_escapes_move_the_cursor_and_the_lines` |
| The word under the pointer: up to a space, quote or bracket, without the full stop or colon after it; Hangul by either cell; nothing on a space | `term::tests::the_word_under_the_pointer` |
| The empty space right of a scrollback line is not its last word (a Ctrl+click there opened the path the line ended with) | `term::tests::the_word_under_the_pointer` |
| Synchronized output (DEC 2026), DECRQM | `term::sync_tests::*` |
| Typing flicker: output drawn 4 ms later | none (timing; the 2026 part is tested) |
| Selection modes (char, word, line, block); words and paths under the pointer | `ui_term::tests::selection_modes`, `tokens` |
| Copy mode keys; Ctrl+Shift+X (arrives as Cut) | `copymode::tests::*`, `keys::tests::clipboard_events_find_their_bindings`, `regress::copy_mode_key_arrives_as_cut` |
| Korean IME (composition, Enter, Backspace) | `ui_term::tests::text_during_composition_follows_the_commit`, `enter_during_composition_is_sent_once_after_the_syllable`, `hangul_backspace_edits_the_syllable_not_the_app`, `ime_position_is_never_missing_and_frozen_while_composing` |
| Folds | `ui_term::tests::folds_collapse_rows` |
| Pane split junctions; zoom | `ui_term::tests::junction_where_three_panes_meet`, `zoom_shows_only_active_pane_full_size` |
| Drag a pane header to move it | `ui_term::tests::dragging_a_pane_header_moves_the_pane`, `app::node_tests::*` |
| Windows: terminals run on Windows' own console host, or a conpty.dll the setting names or that is beside weft.exe; never one found on PATH; About and Display test say which | `pty::tests::the_console_host_is_the_settings_or_the_one_beside_or_windows_own`, `terminals_run_on_windows_own_console_host_by_default`, `regress::the_version_is_shown` |
| A closed terminal's console host goes too (it stayed for good, using a core) | `pty::tests::a_dropped_terminal_is_closed_before_the_end`, `cursor_reports_are_told_from_other_input`, `the_first_cursor_report_fits_the_screen_it_was_opened_with` |
| A restart's new weft, started while the terminals are open, holds none of them: they close and their console hosts go while it runs on | `pty::tests::a_process_started_meanwhile_keeps_no_terminal_open` |
| Shell events become widgets in the terminal: table (sort), progress (updated in place), choice (types its number), file list (open, show in Files), a note | `regress::shell_events_become_widgets` |
| Several widgets shown by one program in one go are one below the other (on Windows they were drawn on top of each other: only the last could be seen); each takes its clicks | `regress::widgets_printed_in_one_go_each_get_their_lines`, `app::tests::a_widget_goes_below_one_whose_text_has_not_come` |
| What weft types reaches the program: the composer's line, typing, a paste | `regress::what_weft_types_reaches_the_program` |
| The cursor stays on, in its place, while a program redraws many times a second (hide, draw, show: it was off in four frames of ten, and flickered); one hidden for good goes | `regress::the_cursor_holds_still_while_a_program_redraws` |
| Scrolled back a little, the cursor stays on its line and moves with the text in every frame; it goes when its line leaves the screen (it went at the first line scrolled) | `regress::the_cursor_moves_with_the_text_while_scrolling` |
| Scrolling while a program redraws (spinner above the input line): the cursor is on the input line in every frame (it was missing in 180 of 603) | `regress::the_cursor_holds_still_while_scrolling_during_redraws` |
| Record this terminal: what the program sent (control bytes readable), what was typed, where the cursor was drawn, with times, to data/logs; the status bar says so; again to stop | `regress::recording_a_terminal_writes_its_traffic` |
| The mouse in a terminal: wheel scrolls back and stops at the end; drag selects characters (over lines too), double click a word, triple click a line, Alt+drag a block; a click drops the selection; Ctrl+click opens a path at its line, not a plain word; the menu's Copy selection and Split right | `regress::the_mouse_in_a_terminal` |
| A command block: found from the shell's marks with its exit code; its buttons copy the command, copy the output, fold and unfold, edit in the composer, run again, open the output in the editor; fold all, unfold all, previous and next command, clear scrollback | `regress::a_command_block_has_its_buttons` |
| Folding or unfolding while scrolled back leaves the command where it is (it slid from under the pointer, or off the screen) | `regress::a_command_block_has_its_buttons` |
| Find in scrollback: the count, Enter and Shift+Enter step and wrap and bring the hit into view, the arrow, Aa for case, × closes | `regress::find_in_scrollback_steps_through_the_hits` |
| Find: the first Enter after typing goes to the first hit (it went to the second) | `regress::find_in_scrollback_steps_through_the_hits` |
| Serial console: bridge, command, missing port | `host::tests::serial_bridge_and_command`, `regress::serial_tab_explains_a_missing_port` |
| X11 forwarding (-Y, DISPLAY, old settings) | `x11::tests::*` |
| Block characters drawn, not from the font (no black band) | `ui_term::tests::blocks_fill_their_cells`, `a_full_block_hides_its_background` |
| Box drawing drawn, not from the font: frames and trees are whole (pixel) | `ui_term::tests::box_drawing_characters_have_their_arms`, `regress::box_drawing_lines_join` |
| Letters stay in their cells in any font | `ui_term::tests::letters_keep_their_cells_in_any_font` |
| A logo, blocks and columns look right in four fonts (pixel) | `regress::the_logo_and_the_grid_look_right_in_every_font` |
| Esc closes composer and scrollback search | `regress::escape_closes_composer_search_and_find` |
| Closing a busy terminal asks | `regress::closing_a_busy_terminal_asks` |

## Shells and hosts

| What | Test |
|---|---|
| The command that starts a remote shell: any argv parser, fits a command line; the bundle of integration scripts | `host::tests::bootstrap_commands_survive_any_argv_parser`, `bootstrap_fits_the_command_line`, `bundle_sections_are_intact` |
| Shell integration end to end (integration): bash with its rc in memory, PowerShell, cmd, nushell, login shells, unknown shells guessed, the agent over stdin, closing a pane is quick, PowerShell's block time and cursor | `host::loopback::*` |

## Keys, tabs, palette, dialogs

| What | Test |
|---|---|
| Default keys valid and distinct | `ui_panels::tests::default_keys_are_valid_and_distinct`, `keys::tests::*` |
| Alt+1…9, Ctrl+Tab / Ctrl+Shift+Tab | `regress::tab_keys` |
| Split panes resize from the keyboard: Alt+Shift+arrow moves the nearest border that way by 5%, stops at 10%/90%, says when there is none, takes other keys, kept with the session; with no split the keys are the shell's | `regress::split_panes_resize_from_the_keyboard` |
| The same host on two ports (`-p 22008`, `-p 22024`) is two profiles, each ssh with its own port; typed again, the same one; an old profile named by the host alone is found by its options | `weft_core::host::tests::one_host_on_two_ports_is_two_profiles` |
| A terminal started with no locale (from the Finder) gets a UTF-8 one (the Mac's, en_US, or C.UTF-8), so `ls` shows Korean names; a set LANG/LC_ALL/LC_CTYPE is left alone | `weft_core::pty::tests::a_terminal_gets_a_utf8_locale_when_none_is_set` |
| Sending the agent to an ssh that already ended is no error: ssh's words become the reason (a Mac showed "Broken pipe" and never asked for the password) | `host::loopback::shipping_to_an_ended_ssh_is_no_error` |
| An agent refused before it is sent: the reason is ssh's "Permission denied", so a password is asked for | `host::loopback::an_agent_refused_before_it_is_sent_says_why` |
| The agent finds Python by the list: a Python under an odd name by a `*` pattern, a Python 2 passed over; none: what was tried and what to do (Git Bash with its own PATH) | `host::loopback::the_agent_finds_python_by_the_list` |
| agent.py and serial.py stay Python 3.6 (Rocky 8's platform-python): grammar and subprocess arguments | `host::tests::remote_python_stays_3_6` |
| "Python on remote hosts…": the list edited a line each, saved to settings and carried by the agent's command; Defaults; nine Esc-closed windows fit the mask | `regress::the_pythons_tried_on_remote_hosts_can_be_listed` |
| A remote host with no Python: the status bar says why with a python… button that opens the list, Files and the editor say why and what to do, nothing waits for good, the terminal works; the reason is one line (it ran over lines and the end was kept) | `regress::a_host_without_python_says_so_where_the_gui_needs_it` |
| Connect keeps a new profile once it has connected (a prompt, or its agent): before, it is used but neither it nor its tab is in settings.json; a try that never connects is gone after a restart; a Host of ~/.ssh/config picked from the list is its name whole | `regress::connect_keeps_a_profile_once_it_connects` |
| Enter in the password dialog is Connect (it did nothing), and the password is the host's | `regress::enter_in_the_password_dialog_connects` |
| A password given in the dialog starts again the host's terminals waiting at ssh's password prompt; one that got in is left alone | `regress::a_password_given_once_serves_the_waiting_terminals` |
| ~/.ssh/config Host names: quoted ones without their quotes (spaces kept), `Host=`, wildcards and negations left out | `host::tests::ssh_config_host_names` |
| Ctrl+Shift+Z is redo in a text field | `ui_panels::tests::redo_key_stays_with_a_text_field` |
| Tab drag reorder; narrow tab rows; which tabs a tab's menu closes | `ui_panels::tests::dragged_tab_lands_between_tabs`, `narrow_tab_rows_keep_the_open_tab`, `app::close_tests::*` |
| Palette reopens full height, at the top | `regress::palette_reopens_full_height_at_the_top` |
| Palette: fuzzy by default; text, glob, regex; mode kept | `ui_panels::tests::the_palette_matches_fuzzy_text_glob_or_regex`, `regress::the_palette_is_fuzzy_and_its_mode_is_kept` |
| Version in Help and About; the version line; CHANGELOG has it | `regress::the_version_is_shown`, `version::tests::the_version_line` |
| Display test: its pages fit, open from Help, show their lines; the logo in pixels | `display_test::tests::the_pages_fit_their_terminal`, `regress::the_display_test_window_shows_its_pages` |
| Closing weft with unsaved files asks, then closes | `regress::closing_weft_with_unsaved_files_asks` |
| A restart brings back what was open | `regress::a_restart_brings_back_what_was_open` |
| The timed save writes settings.json only when something changed | `regress::the_timed_save_writes_only_changes` |
| Keyboard shortcuts: rows hold still on remove | `regress::keys_window_rows_hold_still` |
| Keyboard shortcuts: + takes the next key (Esc: none, and the window stays), the key then runs the command, ↺ and Reset all go back; the changed filter, search by key, the clash count | `regress::a_key_is_bound_by_pressing_it` |
| The tab bar: click switches, drag moves (the dragged tab stays active), + opens one on the same host, × and middle click close, the menu closes the others | `regress::the_tab_bar_switches_moves_and_closes` |
| The buttons right of the tabs: editor, side panel, split right and down, palette, ?, Aa, theme, and the ☰ menu of every command with its keys | `regress::the_top_bar_buttons` |
| In a narrow window (330, 420, 520, 700 px) nothing in the tab bar covers anything else or runs off the window (the tab's + was under the palette button) | `regress::the_tab_bar_fits_a_narrow_window` |
| Commands one by one: focus mode, cursor shape, font and UI size with their limits, hidden files, markdown breaks, theme: system, local shell, zoom, focus left/right/up/down, history, memos, privacy, features, open memo, theme, fonts, settings.json | `regress::commands_do_what_they_say` |
| Each search opens as last set, also after a restart: the Search panel (mode, ignore case, hidden), the editor's find (mode, match case), find in a terminal (Aa) | `regress::searches_open_as_they_were_last_set` |
| When a terminal's program ends, its notice covers none of the text: the text is raised above it (it covered the last line); text that does not reach it stays put | `regress::the_exit_notice_covers_no_text` |
| Focus mode's label has its own strip: it covers no button and no pane (it covered a split pane's close button); a click on it leaves | `regress::focus_mode_label_covers_nothing` |
| Zoom's and focus mode's labels, alone and together, cover neither each other nor a button (in focus mode they were drawn over each other); each still works | `regress::zoom_and_focus_labels_cover_nothing` |
| The status bar: text size and UI scale buttons, the terminal's size, an agent that is not there with reconnect and password… | `regress::the_status_bar` |
| Dialogs fit a small window | `regress::dialogs_fit_a_small_window` |
| Esc closes every dialog, menu and picker; it goes to what has the keyboard (the shell, a composer or the palette on top) | `regress::every_dialog_closes_on_escape`, `regress::escape_goes_to_what_has_the_keyboard` |
| The composer sends on Enter; Tab completes | `regress::the_composer_sends_on_enter`, `app::tests::completion_lcp_then_cycle` |
| A memo is typed and saved by its button; its tab is named after its first line; the list, newest first | `regress::a_memo_is_typed_and_saved`, `ui_panels::tests::memo_tab_is_named_after_the_first_line`, `app::tests::memo_list_newest_first_with_titles` |
| Privacy: "never" reads 0 while typed in | `ui_panels::tests::never_reads_zero_once_clicked` |
| Restart: a newer weft lights the button | `regress::a_newer_weft_offers_a_restart` |

## Panels and layout

| What | Test |
|---|---|
| Panels move anywhere; junctions; hidden keep place | `app::layout_tests::*`, `app::tests::layout_split_and_remove` |
| Split side panels / editors; old settings load | `app::layout_tests::a_second_editor_beside_the_first`, `settings_from_before_split_panels_still_load`, `ui_term::tests::dragging_a_pane_header_moves_the_pane` (second editor by its grip) |
| Side panels following the terminal | `ui_panels::tests::side_panels_that_follow_the_terminal_follow_it` |
| Files follows the terminal that has the keyboard: a click on a tab, Alt+number, Ctrl+Shift+Tab, the tab left after one is closed, a click in a pane of a split; switched on it goes there at once, switched off it stays (a tab click left it in the folder of the tab before) | `regress::files_follow_the_terminal_that_has_the_keyboard` |
| Side panels and editors split, without overlapping | `regress::panels_split` |

## Files, search, transfer

| What | Test |
|---|---|
| Listing; protected Windows links hidden; paths | `host::tests::local_backend_roundtrip`, `listing_leaves_out_protected_windows_links`, `app::tests::paths` |
| Hidden / git-ignored switch | `host::tests::list_can_leave_out_what_git_ignores` |
| A folder of thousands of ignored files is listed (it hung) | `host::tests::a_folder_of_thousands_of_ignored_files_is_listed` |
| Fuzzy / glob / regex | `find::tests::*`, `editfind::tests::*` |
| Glob: text with no wildcard is found anywhere in the name (or a command); a pattern is the whole name | `find::tests::glob_without_a_wildcard_is_contained` |
| Grep modes (local and agent) | `host::tests::local_grep_modes_and_hidden`, `local_grep_text_or_regex` |
| Tree like the agent | `host::tests::tree_lists_names_like_the_agent` |
| Transfer ops; any host to any host; no overwrite | `host::tests::transfer_ops_copy_a_folder_in_chunks`, `transfer::tests::*` |
| History search | `ui_panels::tests::history_search` |
| The Files filter: fuzzy and regex (said when it can't be read), a click selects, a double click or Enter opens, the menu copies the path and shows the file in the tree | `regress::the_files_filter_finds_names` |
| The Search tab (from the » menu of a narrow panel): matches by file, ignore case, a click opens at the line, the menus copy path:line and the line, and show the file in Files | `regress::the_search_tab_finds_text_in_files` |
| The Search tab's controls are inside a narrow side panel (the Search button was cut off at its edge) | `regress::the_search_tab_finds_text_in_files` |
| The Commands tab: newest first, how each ended, a click goes to it, copy command and output. History: what was recorded, found by text, in the output when asked, opened with its output; Records… | `regress::the_commands_and_history_tabs` |
| New folder, new file, rename, delete (this PC) | `host::tests::file_operations`, `fileops::tests::parents_and_names` |
| The same as the remote agent does them (agent.py, run by python) | `host::tests::agent_file_operations_match_the_local_ones` |
| A file dragged onto a folder is copied; Send to the other Files panel | `regress::dragging_a_file_onto_a_folder_copies_it`, `regress::send_to_the_other_files_panel` |
| The eye menu shows and hides hidden files | `regress::files_show_hidden_from_the_eye_menu` |
| The same from a row's menu; Esc cancels a delete | `regress::files_new_folder_rename_delete` |
| The hint beside a dragged file leaves its row readable | `transfer::tests::the_drop_hint_leaves_its_row_readable` |
| Procs on this PC: list and kill; in its panel | `host::tests::local_processes_list_and_kill`, `regress::procs_lists_this_pc` |
| A process list from another host is dropped | `app::tests::a_process_list_from_another_host_is_dropped` |

## Editor and markdown

| What | Test |
|---|---|
| Markdown rendering kept byte for byte | `md::tests::*` |
| Lists nest as on GitHub (two spaces under "- ", three under "1. "; one is the next item); mixed kinds count every level | `md::tests::lists_nest_as_on_github` |
| Ordered items: 1. i. a. by depth, numbered from the first one's number ("1." for each counts up), roman and letters past 26 | `md::tests::ordered_items_are_numbered_by_depth` |
| Bullets know their depth (drawn ● ○ ■ □); a task's box stands for its bullet | `md::tests::bullets_know_their_depth` |
| An item's text is further in than its parent's whatever the spaces, a paragraph of it under its text, and nothing moves when the cursor comes to the line | `md::tests::list_text_keeps_its_place` |
| A list item wraps under its own text (hanging indent), a link too long for the row is cut where it starts with its marker on the same row (it went to the left edge, the bullet over the row above); every char, row end and cursor place kept; wide: as egui lays it out | `md::tests::list_items_wrap_under_their_text` |
| Tables: fit the width, wrap in columns, rows' source | `md::tests::tables_hide_pipes_and_align_columns`, `a_wide_table_wraps_inside_its_columns` |
| Unsaved / saved / copied marks; Rich ⇄ Source; Read mode | `regress::editor_marks_buttons_and_read_mode` |
| Double-click selects a word (Hangul, line ends) | `widgets::word_tests::*` |
| Memos never ask to overwrite their own save | `ui_panels::tests::memos_never_conflict_with_their_own_saves` |
| Find / replace | `editfind::tests::*` |
| Syntax colours: many languages, a changed line gives what the whole file gives, find marks; speed (ignored) | `highlight::tests::*` |
| Hidden images leave the GPU | `regress::hidden_images_leave_the_gpu` |
| A file is laid out when it changes, not once a frame; again with new fonts | `regress::the_editor_lays_a_file_out_once` |
| Tables far down a document are drawn when scrolled to | `regress::tables_far_down_a_document_are_drawn_when_reached` |
| Images, PDF pages | `media::tests::*` |
| Mermaid; where mermaid-cli is found on Windows | `mermaid::tests::*`, `mermaid::win::*` |
| A table selection stays with its table when another scrolls out of view | `regress::a_table_selection_stays_with_its_table` |
| Table cells: letters, a cell, a block; Ctrl+C; the menu | `ui_panels::tests::a_table_selection_copies_letters_a_cell_or_a_block`, `regress::table_cells_select_and_copy` |
| Closing an unsaved file asks: save, drop, cancel, off | `regress::closing_an_unsaved_file_asks` |
| Saving then closing asks about what changed meanwhile; Cancel closes nothing later | `regress::saving_then_closing_asks_about_what_changed_meanwhile` |
| A save finds its file after a rename | `app::tests::a_save_finds_its_file_after_a_rename` |
| What waited for an agent that died fails, and says so (a save stayed "saving" for good) | `app::tests::what_waited_for_a_dead_agent_fails` |
| Typed on while a save was on its way: still unsaved when the answer comes | `app::tests::a_save_answered_after_more_typing_leaves_the_file_unsaved` |
| A few laid-out files are kept, not every one ever opened | `regress::laid_out_files_are_not_kept_for_ever` |
| Esc closes the find bar | `regress::escape_closes_composer_search_and_find` |
| Find in the editor: the count with and without the case, Enter / Shift+Enter / F3 / Shift+F3 / the arrows step and go round, replace one and all (the file is then unsaved), a regex with groups, one that can't be read, no match | `regress::find_and_replace_in_the_editor` |
| Shift+F3 goes to the match before (it went to the next) | `regress::find_and_replace_in_the_editor` |
| The find bar fits a narrow editor: its × is inside the window, on the bar's line (it was past the edge) | `regress::find_and_replace_in_the_editor` |
| A file changed on disk since it was opened is not written over: Reload takes what is there, Overwrite what is here | `regress::a_file_changed_on_disk_is_not_written_over` |
| The editor's tabs: a click shows the file, × and a middle click close it, the menu closes to the right, the others, all | `regress::the_editor_tabs_switch_and_close` |
| Open memo…: by first line, found by text, opened by a click, deleted from the menu, New memo | `regress::the_memo_picker_lists_finds_and_deletes` |
| Table resize settles after 150 ms | none |

## Fonts, theme, security, records

| What | Test |
|---|---|
| Glyph advances follow the ink (no overlap) | `fontfix::tests::*`, `theme::tests::hangul_takes_its_width_in_text_runs`, `text_runs_stay_on_the_cell_grid` |
| No boxes for TUI symbols; Nerd icons | `theme::tests::tui_symbols_never_show_as_boxes`, `builtin_fonts_have_nerd_icons` |
| A font that can't be read falls back; only regular faces are offered; themes keep text readable; which theme a setting means | `theme::tests::unreadable_fonts_fall_back_to_the_default`, `only_regular_faces_are_offered`, `themes_keep_body_text_readable`, `resolve_choices` |
| Passwords in memory | `secret::tests::*`, `host::tests::askpass_answers`, `password_switches_agent_off_batch_mode` |
| Records and history limits | `records::tests::*` |
| The theme menu: every theme offered, a click picks one, Follow system, closed by ×, Esc and a click elsewhere | `regress::the_theme_menu_picks_a_theme` |
| The Fonts window: built-in fonts, the filter, the terminal font and the icon font each from its tab, a file that is no font is said and the built-in one used | `regress::the_font_window_sets_fonts` |
| Privacy: what is kept and where, a rule switched off and on, Delete all commands, the passwords switch, Esc | `regress::the_privacy_window_keeps_and_deletes` |

## The test tools themselves

| What | Test |
|---|---|
| The picture a frame is filled in as: shapes and text come out where they were put | `shot::tests::shapes_come_out_as_pixels` |
| Every test is named in this document | `doccheck::every_test_is_in_the_document`, `tests_are_found_in_source` |
| Measurements, run by hand (see Tools) | `regress::frame_times`, `scenes::scenes` |

## To check by hand

What no test here can see. The first is the one worth doing after changing how anything is drawn.

1. **Display test** (Help's link, the palette, or the ☰ menu): four test screens drawn with your font in the real window. Each says what to look for. The tests check the same screens in pixels, but with the fonts and the scale of the test, not your monitor's.
2. **Typing Korean** in the terminal and the editor (the IME's composition is the system's; tests feed its events, not the IME).
3. **Things that come from other programs**: files dropped from Explorer, copy and paste with other apps, a second monitor or another scale.
4. **Real hosts**: files and Procs over ssh, a serial port, X11 forwarding. (The agent's operations are run here by python; the wire to a real host is covered by the integration tests for listing and reading only.)
5. **Deleting on this PC** goes to the Recycle Bin, and Windows warns when something can't go there (tests delete for good: the bin is yours).
6. **After weft is closed, no `OpenConsole.exe` of its own is left** in Task Manager (the tests check the same path; `python tools/leak.py` counts them around a test run).
7. **install.bat / uninstall.bat**, and `weft --version` from a shell after installing. **install.sh / uninstall.sh** on a real Linux desktop and a Mac: that the menu entry or weft.app starts weft, with its data in `data/` beside it (`python tools/install_test.py` checks where every file goes, and that a weft built from older sources is refused, with a stand-in weft; it cannot open a window, nor make the macOS icon, which needs a Mac).
8. **How it feels**: typing latency, flicker, a resize. Frame times are measured without a GPU (`frame_times`); what a screen does with them is not.
9. **A conpty.dll of your own** (the `console_host` setting, or one beside weft.exe): the tests run on Windows' own; that a given one is loaded and used is checked by hand (About says which, and Task Manager shows OpenConsole.exe instead of conhost.exe).
10. **Linux and macOS**: everything compiles for them (see the commands at the top), and nothing has been run there. `cargo test --workspace` on such a machine tests their code; several tests start `python`, which there may be `python3`.

## Tools

```
python tools/testdoc.py                                  # measure, and rewrite "In numbers" above
cargo test -p weft frame_times -- --ignored --nocapture  # how long a frame takes in heavy situations
cargo test -p weft scenes -- --ignored                   # eighteen screens as PNGs in target/shots/scenes
python tools/scenediff.py <folder a> <folder b>          # two sets of scenes, pixel by pixel
python tools/leak.py <test binary> [test filter]         # console hosts left behind by a run (Windows)
```

`scenes.rs` uses only what the test harness has had from its first
version, so it can be copied into an older checkout (with `shot.rs`) to
draw the same screens there and compare.
