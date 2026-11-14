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
Measured 2026-11-14 at commit b878b9d by `python tools/testdoc.py` (it rewrites this part).

**How many tests**

| Kind | the app | weft-core | weft-draw | together |
|---|--:|--:|--:|--:|
| Unit | 58 | 60 | 31 | 149 |
| Screen (the whole app, no window) | 43 | 0 | 0 | 43 |
| Integration (real shells, the agent; run with `--ignored`) | 1 | 13 | 1 | 15 |
| **All** | 102 | 73 | 32 | **207** |

**How much of the code they run** (lines run / lines there are; test code itself left out)

| Crate | lines | unit | screen | integration | all together |
|---|--:|--:|--:|--:|--:|
| weft (the app) | 11282 | 33 % | 55 % | 1 % | 68 % |
| weft-core | 4331 | 73 % | 36 % | 28 % | 86 % |
| weft-draw | 2220 | 81 % | 44 % | 7 % | 88 % |
| **everything** | 17833 | **49 %** | **49 %** | **8 %** | **75 %** |

By file (all kinds together), least covered first:

| File | lines | run | not run |
|---|--:|--:|--:|
| `src/main.rs` | 135 | 2 % | 132 |
| `src/widgets.rs` | 412 | 28 % | 298 |
| `src/x11.rs` | 49 | 47 % | 26 |
| `src/restart.rs` | 61 | 48 % | 32 |
| `src/mermaid.rs` | 326 | 54 % | 151 |
| `src/media.rs` | 254 | 59 % | 104 |
| `src/serial_ui.rs` | 174 | 65 % | 61 |
| `src/ui_panels.rs` | 4333 | 66 % | 1468 |
| `src/ui_term.rs` | 2000 | 71 % | 585 |
| `crates/weft-draw/src/theme.rs` | 441 | 74 % | 113 |
| `src/app.rs` | 2118 | 74 % | 542 |
| `src/transfer.rs` | 509 | 78 % | 111 |
| `crates/weft-core/src/term.rs` | 1525 | 79 % | 320 |
| `crates/weft-core/src/pty.rs` | 295 | 80 % | 58 |
| `crates/weft-draw/src/icons.rs` | 165 | 81 % | 32 |
| `src/confirm.rs` | 233 | 84 % | 37 |
| `crates/weft-core/src/host.rs` | 1613 | 88 % | 196 |
| `crates/weft-draw/src/highlight.rs` | 428 | 88 % | 51 |
| `src/copymode.rs` | 211 | 89 % | 24 |
| `crates/weft-core/src/find.rs` | 186 | 92 % | 14 |
| `src/fileops.rs` | 179 | 93 % | 13 |
| `crates/weft-draw/src/md.rs` | 960 | 94 % | 62 |
| `crates/weft-core/src/records.rs` | 257 | 94 % | 16 |
| `crates/weft-core/src/secret.rs` | 274 | 95 % | 14 |
| `crates/weft-core/src/editfind.rs` | 181 | 96 % | 8 |
| `src/frame.rs` | 61 | 97 % | 2 |
| `crates/weft-draw/src/keys.rs` | 112 | 97 % | 3 |
| `src/doccheck.rs` | 59 | 98 % | 1 |
| `crates/weft-draw/src/fontfix.rs` | 114 | 99 % | 1 |
| `src/display_test.rs` | 148 | 100 % | 0 |
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
| Resize reflows, blocks move with lines | `term::tests::narrowing_then_widening_restores_the_prompt`, `reflow_moves_blocks_with_their_lines`, `resize_shrink_pushes_to_history` |
| Scrollback stores no trailing blanks (memory) | `term::tests::scrollback_keeps_no_trailing_blanks` |
| OSC 133 blocks; guessed without it | `term::tests::osc133_blocks_capture_command_and_exit`, `term::guess_tests::*` |
| OSC 7 / 9;9 directory, OSC 7777 events, OSC 52 | `term::tests::osc7777_base64_event_and_osc7`, `osc_9_9_is_the_current_directory`, `clipboard_support_is_reported` |
| Leading space keeps a command out of history | `term::tests::leading_space_makes_a_command_private` |
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
| Shell events become widgets in the terminal: table (sort), progress (updated in place), choice (types its number), file list (open, show in Files), a note | `regress::shell_events_become_widgets` |
| What weft types reaches the program: the composer's line, typing, a paste | `regress::what_weft_types_reaches_the_program` |
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
| Alt+1…9, Alt+Shift+←/→ | `regress::tab_keys` |
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
| Side panels and editors split, without overlapping | `regress::panels_split` |

## Files, search, transfer

| What | Test |
|---|---|
| Listing; protected Windows links hidden; paths | `host::tests::local_backend_roundtrip`, `listing_leaves_out_protected_windows_links`, `app::tests::paths` |
| Hidden / git-ignored switch | `host::tests::list_can_leave_out_what_git_ignores` |
| A folder of thousands of ignored files is listed (it hung) | `host::tests::a_folder_of_thousands_of_ignored_files_is_listed` |
| Fuzzy / glob / regex | `find::tests::*`, `editfind::tests::*` |
| Grep modes (local and agent) | `host::tests::local_grep_modes_and_hidden`, `local_grep_text_or_regex` |
| Tree like the agent | `host::tests::tree_lists_names_like_the_agent` |
| Transfer ops; any host to any host; no overwrite | `host::tests::transfer_ops_copy_a_folder_in_chunks`, `transfer::tests::*` |
| History search | `ui_panels::tests::history_search` |
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
| Table resize settles after 150 ms | none |

## Fonts, theme, security, records

| What | Test |
|---|---|
| Glyph advances follow the ink (no overlap) | `fontfix::tests::*`, `theme::tests::hangul_takes_its_width_in_text_runs`, `text_runs_stay_on_the_cell_grid` |
| No boxes for TUI symbols; Nerd icons | `theme::tests::tui_symbols_never_show_as_boxes`, `builtin_fonts_have_nerd_icons` |
| A font that can't be read falls back; only regular faces are offered; themes keep text readable; which theme a setting means | `theme::tests::unreadable_fonts_fall_back_to_the_default`, `only_regular_faces_are_offered`, `themes_keep_body_text_readable`, `resolve_choices` |
| Passwords in memory | `secret::tests::*`, `host::tests::askpass_answers`, `password_switches_agent_off_batch_mode` |
| Records and history limits | `records::tests::*` |

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
7. **install.bat / uninstall.bat**, and `weft --version` from a shell after installing.
8. **How it feels**: typing latency, flicker, a resize. Frame times are measured without a GPU (`frame_times`); what a screen does with them is not.
9. **Several widgets printed in one burst** (Windows): the pseudo console hands a program's events on at once and its text when it next draws, so events printed without a pause between land on one line and cover each other. One widget per command, as the helpers print them, is placed right. Not something weft can change; the widget test pauses between them.
10. **A conpty.dll of your own** (the `console_host` setting, or one beside weft.exe): the tests run on Windows' own; that a given one is loaded and used is checked by hand (About says which, and Task Manager shows OpenConsole.exe instead of conhost.exe).
11. **Linux and macOS**: everything compiles for them (see the commands at the top), and nothing has been run there. `cargo test --workspace` on such a machine tests their code; several tests start `python`, which there may be `python3`.

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
