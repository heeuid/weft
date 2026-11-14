# Regression tests: what is covered

`cargo test --workspace` runs all of these (three crates: the app, weft-core, weft-draw); each is a feature, or a bug that
was fixed, so a change that breaks it fails. Screen behaviour is tested
through the real app, headless (`src/testkit.rs`: the same `App::frame` the
window runs, fed clicks, drags and keys, checked by state and by the texts
it drew). `host::loopback` tests need Git Bash and run with `--ignored`.

When something is added or fixed, a test for it goes in with it, and a row
here. "none" marks what still has no automated test.

## Terminal

| What | Test |
|---|---|
| Output, wrapping, scrollback, absolute lines | `term::tests::prints_wraps_and_scrolls_into_history`, `clear_screen_keeps_absolute_lines` |
| Hangul two cells; never split at a row end | `term::tests::hangul_is_two_cells_wide`, `reflow_never_splits_a_wide_char` |
| Resize reflows, blocks move with lines | `term::tests::narrowing_then_widening_restores_the_prompt`, `reflow_moves_blocks_with_their_lines`, `resize_shrink_pushes_to_history` |
| Scrollback stores no trailing blanks (memory) | `term::tests::scrollback_keeps_no_trailing_blanks` |
| OSC 133 blocks; guessed without it | `term::tests::osc133_blocks_capture_command_and_exit`, `term::guess_tests::*` |
| OSC 7 / 9;9 directory, OSC 7777 events, OSC 52 | `term::tests::osc7777_base64_event_and_osc7`, `osc_9_9_is_the_current_directory`, `clipboard_support_is_reported` |
| Leading space keeps a command out of history | `term::tests::leading_space_makes_a_command_private` |
| Synchronized output (DEC 2026), DECRQM | `term::sync_tests::*` |
| Typing flicker: output drawn 4 ms later | none (timing; the 2026 part is tested) |
| Selection modes (char, word, line, block) | `ui_term::tests::selection_modes` |
| Copy mode keys; Ctrl+Shift+X (arrives as Cut) | `copymode::tests::*`, `keys::tests::clipboard_events_find_their_bindings`, `regress::copy_mode_key_arrives_as_cut` |
| Korean IME (composition, Enter, Backspace) | `ui_term::tests::text_during_composition_follows_the_commit`, `enter_during_composition_is_sent_once_after_the_syllable`, `hangul_backspace_edits_the_syllable_not_the_app`, `ime_position_is_never_missing_and_frozen_while_composing` |
| Folds | `ui_term::tests::folds_collapse_rows` |
| Pane split junctions; zoom | `ui_term::tests::junction_where_three_panes_meet`, `zoom_shows_only_active_pane_full_size` |
| Drag a pane header to move it | `ui_term::tests::dragging_a_pane_header_moves_the_pane`, `app::node_tests::*` |
| A closed terminal's console host goes too (it stayed for good, using a core) | `pty::tests::a_dropped_terminal_is_closed_before_the_end`, `cursor_reports_are_told_from_other_input`, `the_first_cursor_report_fits_the_screen_it_was_opened_with` |
| Serial console: bridge, command, missing port | `host::tests::serial_bridge_and_command`, `regress::serial_tab_explains_a_missing_port` |
| X11 forwarding (-Y, DISPLAY, old settings) | `x11::tests::*` |
| Block characters drawn, not from the font (no black band) | `ui_term::tests::blocks_fill_their_cells`, `a_full_block_hides_its_background` |
| Letters stay in their cells in any font | `ui_term::tests::letters_keep_their_cells_in_any_font` |
| Esc closes composer and scrollback search | `regress::escape_closes_composer_search_and_find` |
| Closing a busy terminal asks | `regress::closing_a_busy_terminal_asks` |

## Keys, tabs, palette, dialogs

| What | Test |
|---|---|
| Default keys valid and distinct | `ui_panels::tests::default_keys_are_valid_and_distinct`, `keys::tests::*` |
| Alt+1…9, Alt+Shift+←/→ | `regress::tab_keys` |
| Ctrl+Shift+Z is redo in a text field | `ui_panels::tests::redo_key_stays_with_a_text_field` |
| Tab drag reorder; narrow tab rows | `ui_panels::tests::dragged_tab_lands_between_tabs`, `narrow_tab_rows_keep_the_open_tab` |
| Palette reopens full height, at the top | `regress::palette_reopens_full_height_at_the_top` |
| Palette: fuzzy by default; text, glob, regex; mode kept | `ui_panels::tests::the_palette_matches_fuzzy_text_glob_or_regex`, `regress::the_palette_is_fuzzy_and_its_mode_is_kept` |
| Version in Help and About; the version line; CHANGELOG has it | `regress::the_version_is_shown`, `version::tests::the_version_line` |
| Closing weft with unsaved files asks, then closes | `regress::closing_weft_with_unsaved_files_asks` |
| A restart brings back what was open | `regress::a_restart_brings_back_what_was_open` |
| The timed save writes settings.json only when something changed | `regress::the_timed_save_writes_only_changes` |
| Keyboard shortcuts: rows hold still on remove | `regress::keys_window_rows_hold_still` |
| Dialogs fit a small window | `regress::dialogs_fit_a_small_window` |
| Esc closes every dialog, menu and picker | `regress::every_dialog_closes_on_escape` |
| The composer sends on Enter | `regress::the_composer_sends_on_enter` |
| A memo is typed and saved by its button | `regress::a_memo_is_typed_and_saved` |
| Privacy: "never" reads 0 while typed in | `ui_panels::tests::never_reads_zero_once_clicked` |
| Restart: a newer weft lights the button | `regress::a_newer_weft_offers_a_restart` |

## Panels and layout

| What | Test |
|---|---|
| Panels move anywhere; junctions; hidden keep place | `app::layout_tests::*` |
| Split side panels / editors; old settings load | `app::layout_tests::a_second_editor_beside_the_first`, `settings_from_before_split_panels_still_load`, `ui_term::tests::dragging_a_pane_header_moves_the_pane` (second editor by its grip) |
| Side panels following the terminal | `ui_panels::tests::side_panels_that_follow_the_terminal_follow_it` |
| Side panels and editors split, without overlapping | `regress::panels_split` |

## Files, search, transfer

| What | Test |
|---|---|
| Listing; protected Windows links hidden | `host::tests::local_backend_roundtrip`, `listing_leaves_out_protected_windows_links` |
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
| Procs lists this PC, in the panel | `regress::procs_lists_this_pc` |
| The same from a row's menu; Esc cancels a delete | `regress::files_new_folder_rename_delete` |
| The hint beside a dragged file leaves its row readable | `transfer::tests::the_drop_hint_leaves_its_row_readable` |
| Procs on this PC: list and kill | `host::tests::local_processes_list_and_kill` |
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
| Hidden images leave the GPU | `regress::hidden_images_leave_the_gpu` |
| A file is laid out when it changes, not once a frame; again with new fonts | `regress::the_editor_lays_a_file_out_once` |
| Tables far down a document are drawn when scrolled to | `regress::tables_far_down_a_document_are_drawn_when_reached` |
| Images, PDF pages | `media::tests::*` |
| Mermaid | `mermaid::tests::*` |
| Table cells: letters, a cell, a block; Ctrl+C; the menu | `ui_panels::tests::a_table_selection_copies_letters_a_cell_or_a_block`, `regress::table_cells_select_and_copy` |
| Closing an unsaved file asks: save, drop, cancel, off | `regress::closing_an_unsaved_file_asks` |
| Saving then closing asks about what changed meanwhile; Cancel closes nothing later | `regress::saving_then_closing_asks_about_what_changed_meanwhile` |
| A save finds its file after a rename | `app::tests::a_save_finds_its_file_after_a_rename` |
| Esc closes the find bar | `regress::escape_closes_composer_search_and_find` |
| Table resize settles after 150 ms | none |

## Fonts, theme, security, records

| What | Test |
|---|---|
| Glyph advances follow the ink (no overlap) | `fontfix::tests::*`, `theme::tests::hangul_takes_its_width_in_text_runs`, `text_runs_stay_on_the_cell_grid` |
| No boxes for TUI symbols; Nerd icons | `theme::tests::tui_symbols_never_show_as_boxes`, `builtin_fonts_have_nerd_icons` |
| Passwords in memory | `secret::tests::*`, `host::tests::askpass_answers`, `password_switches_agent_off_batch_mode` |
| Records and history limits | `records::tests::*` |

## Not automated

- install.bat / uninstall.bat (cmd can't run from the test sandbox): run them by hand after changing them.
- `weft --version` printed to a shell (the test binary isn't weft.exe): run it after a release build.
- Delete to the Recycle Bin, and Windows' warning when something can't go there (tests delete for good: the bin is the user's).
- Linux / macOS: a closed terminal ends what ran in it (the reader lets go of the master).
- No console host left behind by a whole run: `python target/leak.py <test binary> [filter]` counts `OpenConsole.exe` without a parent before and after (a script of this checkout, not a test: it lists every process of the PC).
- The loopback tests (`-- --ignored`) time a PowerShell command under 50 ms: run them with `--test-threads=1`, or the shells starting together make the first one slow.
- File operations over a real ssh connection (the agent's ops are tested by running agent.py here; the wire is the same as for listing and reading, which the loopback tests cover).
- Dropping files from Explorer (the system gives no position; the cursor is asked).
- The app icon in Explorer (checked: the exe carries one).
