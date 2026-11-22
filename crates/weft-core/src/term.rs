//! VT/xterm grid emulator with a block model.
//!
//! The primary screen keeps a scrollback addressed by *absolute line numbers*
//! (they never change once assigned, even when old lines are dropped), so
//! command blocks (OSC 133) and inline GUI widgets (OSC 7777) can anchor to
//! lines and scroll together with the text.

use base64::Engine;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use vte::{Params, Perform};

pub const F_BOLD: u8 = 1;
pub const F_DIM: u8 = 2;
pub const F_ITALIC: u8 = 4;
pub const F_UNDER: u8 = 8;
pub const F_INVERSE: u8 = 16;
pub const F_STRIKE: u8 = 32;
pub const F_HIDDEN: u8 = 64;

pub const WIDE_LEAD: u8 = 1;
pub const WIDE_TAIL: u8 = 2;
/// Blank left at a row end because the next wide char did not fit; not content.
pub const WIDE_PAD: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attr {
    pub fg: Color,
    pub bg: Color,
    pub flags: u8,
}

impl Default for Attr {
    fn default() -> Self {
        Attr { fg: Color::Default, bg: Color::Default, flags: 0 }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cell {
    pub ch: char,
    pub attr: Attr,
    pub wide: u8,
}

impl Cell {
    fn blank(attr: Attr) -> Self {
        Cell { ch: ' ', attr: Attr { fg: Color::Default, bg: attr.bg, flags: 0 }, wide: 0 }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line continues on the next row (soft wrap).
    pub wrapped: bool,
}

impl Row {
    fn new(cols: usize, attr: Attr) -> Self {
        Row { cells: vec![Cell::blank(attr); cols], wrapped: false }
    }

    pub fn is_blank(&self) -> bool {
        self.cells.iter().all(|c| c.ch == ' ' && c.attr.bg == Color::Default)
    }

    /// Text of columns `[from, to)`, wide-char tails and pads skipped, trailing
    /// blanks trimmed (except where the row soft-wraps: those spaces are text).
    pub fn text_range(&self, from: usize, to: usize) -> String {
        let to = to.min(self.cells.len());
        let mut s: String = self.cells[from.min(to)..to]
            .iter()
            .filter(|c| c.wide != WIDE_TAIL && c.wide != WIDE_PAD)
            .map(|c| c.ch)
            .collect();
        if !(self.wrapped && to == self.cells.len()) {
            s.truncate(s.trim_end().len());
        }
        s
    }

    pub fn text(&self) -> String {
        self.text_range(0, self.cells.len())
    }

    /// (char, column) pairs, used for searching and hit-testing.
    pub fn chars_with_cols(&self) -> Vec<(char, usize)> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.wide != WIDE_TAIL && c.wide != WIDE_PAD)
            .map(|(i, c)| (c.ch, i))
            .collect()
    }

    fn resize(&mut self, cols: usize, attr: Attr) {
        self.cells.resize(cols, Cell::blank(attr));
        if let Some(last) = self.cells.last_mut() {
            if last.wide == WIDE_LEAD {
                *last = Cell::blank(attr);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum TermEvent {
    Bell,
    Clipboard(String),
    Notify(String),
    CwdChanged,
    /// A guessed command block ended (no integration): the app may look up
    /// the shell's directory from outside (agent `cwd`).
    GuessedDone,
    /// OSC 7777 weft protocol message, anchored at the cursor's absolute line.
    Weft { anchor: u64, value: serde_json::Value },
}

/// See `Term::hidden_from`.
#[derive(Clone, Copy, Debug)]
pub struct HiddenCursor {
    pub alt: bool,
    pub line: u64,
    pub col: usize,
    pub at: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockState {
    Prompt,
    Running,
    Done,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: u64,
    pub prompt_line: u64,
    pub input: Option<(u64, usize)>,
    pub out_line: Option<u64>,
    /// Exclusive end of the output.
    pub end_line: Option<u64>,
    pub exit: Option<i32>,
    pub command: String,
    pub started: Option<Instant>,
    pub duration: Option<Duration>,
    pub folded: bool,
    /// Found by guessing (the shell sends no OSC 133): no exit code.
    pub guessed: bool,
    /// Typed with a leading space: kept out of history and records, as
    /// bash's HISTCONTROL=ignorespace does.
    pub private: bool,
}

impl Block {
    pub fn state(&self) -> BlockState {
        if self.end_line.is_some() {
            BlockState::Done
        } else if self.out_line.is_some() {
            BlockState::Running
        } else {
            BlockState::Prompt
        }
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.duration.or_else(|| self.started.map(|s| s.elapsed()))
    }
}

#[derive(Clone, Copy, Default)]
struct SavedCursor {
    x: usize,
    y: usize,
    attr: Attr,
    line_drawing: bool,
}

pub struct Term {
    pub cols: usize,
    pub rows: usize,
    pub history: VecDeque<Row>,
    /// Absolute line number of `history[0]`.
    pub dropped: u64,
    pub max_history: usize,
    pub screen: Vec<Row>,
    primary_saved: Option<Vec<Row>>,
    pub alt_active: bool,
    pub cx: usize,
    pub cy: usize,
    pending_wrap: bool,
    pub attr: Attr,
    saved: SavedCursor,
    top: usize,
    bot: usize,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    /// 0 = off, otherwise 1000 / 1002 / 1003.
    pub mouse_mode: u16,
    pub mouse_sgr: bool,
    pub cursor_visible: bool,
    /// Where the cursor was when the program last hid it, and when: an
    /// absolute line (or a row of the alternate screen, `alt`) and a
    /// column. Taken as the output is read, so it holds however late the
    /// screen is drawn after (see ui_term's CURSOR_HOLD).
    pub hidden_from: Option<HiddenCursor>,
    autowrap: bool,
    insert_mode: bool,
    line_drawing: bool,
    last_char: char,
    pub title: String,
    pub cwd: Option<String>,
    pub cwd_host: Option<String>,
    pub replies: Vec<u8>,
    /// An XTGETTCAP request (DCS + q) being received.
    xtgettcap: Option<Vec<u8>>,
    pub events: Vec<TermEvent>,
    pub blocks: Vec<Block>,
    next_block: u64,
    /// Bumped on every mutation; lets the UI know something changed.
    pub generation: u64,
    /// The shell marks its prompts (OSC 133); until then blocks are guessed.
    pub integrated: bool,
    /// Guessed blocks are shown (in `blocks`). Off: a guess can be wrong,
    /// so none is shown, kept or recorded; the guessing goes on out of
    /// sight (`unseen`) only to tell that a prompt came (`prompted`) and
    /// when a command probably ended (`TermEvent::GuessedDone`).
    pub show_guesses: bool,
    unseen: Vec<Block>,
    /// The shell also marks command starts (133;C); without them Enter does.
    saw_start: bool,
    /// Where the cursor was at the last 133;C: a second C at the same place
    /// is the same command told twice (fish 4 sends its own marks, and so
    /// does weft's script; a prompt tool beside a shell's own integration).
    start_at: Option<(u64, usize)>,
    last_output: Option<Instant>,
    /// Guessing: the last character of the prompt ('$', '>', '#', '%', '❯' ...).
    prompt_end: Option<char>,
    /// Enter at a marked prompt before the shell has ever sent 133;C: what
    /// was typed, where output starts, when. Used only if a D comes without a
    /// C (cmd marks prompts but not command starts).
    pending_start: Option<(String, u64, Instant)>,
    /// The command weft itself sends with the next Enter (a rerun, the
    /// palette's, a cd here): its text, where the screen has none of it
    /// yet when the Enter is noted (cmd: its block had no command).
    sent_command: Option<String>,
    /// Something was typed at this prompt (told by the app, which sends
    /// it; Ctrl-C and Ctrl-U take it back): an Enter on an empty line starts
    /// nothing. (Not kept past a prompt: keys a running program took, q or
    /// y, made an empty Enter after it a block. A command typed ahead while
    /// another ran makes none so: these shells tell nothing to know it by.) A shell without
    /// a mark for where its input begins (the sh family) gives no text to
    /// tell it by, and an empty Enter became a block with the last exit
    /// code.
    typed: bool,
    /// Set by a reflow: (first old absolute line, new absolute line per old line).
    line_remap: Option<(u64, Vec<u64>)>,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(1);
        Term {
            cols,
            rows,
            history: VecDeque::new(),
            dropped: 0,
            max_history: 20_000,
            screen: (0..rows).map(|_| Row::new(cols, Attr::default())).collect(),
            primary_saved: None,
            alt_active: false,
            cx: 0,
            cy: 0,
            pending_wrap: false,
            attr: Attr::default(),
            saved: SavedCursor::default(),
            top: 0,
            bot: rows - 1,
            app_cursor: false,
            bracketed_paste: false,
            mouse_mode: 0,
            mouse_sgr: false,
            cursor_visible: true,
            hidden_from: None,
            autowrap: true,
            insert_mode: false,
            line_drawing: false,
            last_char: ' ',
            title: String::new(),
            cwd: None,
            cwd_host: None,
            replies: Vec::new(),
            xtgettcap: None,
            events: Vec::new(),
            blocks: Vec::new(),
            next_block: 1,
            generation: 0,
            integrated: false,
            show_guesses: true,
            unseen: Vec::new(),
            saw_start: false,
            start_at: None,
            last_output: None,
            prompt_end: None,
            pending_start: None,
            sent_command: None,
            typed: false,
            line_remap: None,
        }
    }

    pub fn feed(&mut self, parser: &mut vte::Parser, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.last_output = Some(Instant::now());
        }
        parser.advance(self, bytes);
        self.generation = self.generation.wrapping_add(1);
    }

    // ---- addressing -------------------------------------------------------

    pub fn first_line(&self) -> u64 {
        self.dropped
    }

    /// Absolute line of screen row 0 of the primary screen.
    pub fn screen_base(&self) -> u64 {
        self.dropped + self.history.len() as u64
    }

    pub fn end_line(&self) -> u64 {
        self.screen_base() + self.rows as u64
    }

    pub fn cursor_abs(&self) -> u64 {
        self.screen_base() + self.cy as u64
    }

    /// A line of what is on screen: the alternate screen while a full-screen
    /// app runs, else the primary buffer.
    pub fn visible_row(&self, abs: u64) -> Option<&Row> {
        if self.alt_active {
            self.screen.get(abs.checked_sub(self.screen_base())? as usize)
        } else {
            self.line(abs)
        }
    }

    /// A line of the primary buffer (scrollback + primary screen).
    pub fn line(&self, abs: u64) -> Option<&Row> {
        if abs < self.dropped {
            return None;
        }
        let idx = (abs - self.dropped) as usize;
        if idx < self.history.len() {
            return self.history.get(idx);
        }
        let primary = self.primary_saved.as_ref().unwrap_or(&self.screen);
        primary.get(idx - self.history.len())
    }

    pub fn text_between(&self, from: (u64, usize), to: (u64, usize)) -> String {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        let mut out = String::new();
        for abs in from.0..=to.0 {
            let Some(row) = self.line(abs) else { continue };
            let start = if abs == from.0 { from.1 } else { 0 };
            let end = if abs == to.0 { to.1 } else { row.cells.len() };
            out.push_str(&row.text_range(start, end));
            if abs < to.0 && !row.wrapped {
                out.push('\n');
            }
        }
        out
    }

    pub fn lines_text(&self, from: u64, to_exclusive: u64) -> String {
        if to_exclusive <= from {
            return String::new();
        }
        let mut s = self.text_between((from, 0), (to_exclusive - 1, usize::MAX));
        while s.ends_with('\n') {
            s.pop();
        }
        s
    }

    pub fn block_output(&self, b: &Block) -> String {
        match b.out_line {
            Some(o) => self.lines_text(o, b.end_line.unwrap_or(self.cursor_abs() + 1)),
            None => String::new(),
        }
    }

    pub fn at_prompt(&self) -> bool {
        // Guessed blocks out of sight count too: a shell without marks
        // (tcsh, a plain ssh) was never at its prompt, and Rerun, the
        // palette's commands and Files' "cd here" were always refused.
        // A guessed one not where it is a question: a pause after ssh's
        // "password: " or "Verification code: " was guessed a prompt too,
        // and a cd or a rerun went in as the password.
        let last = if !self.integrated && !self.show_guesses { self.unseen.last() } else { self.blocks.last() };
        let shell_like = |b: &Block| !b.guessed || !self.prompt_end.is_some_and(question_end);
        // A command started by an Enter, its start not marked (the sh
        // family): running, though its block is still the prompt's.
        !self.alt_active && self.pending_start.is_none() && last.is_some_and(|b| b.state() == BlockState::Prompt && shell_like(b))
    }

    /// A command runs, as far as can be told: one its shell marked, one an
    /// Enter started where the shell marks no start, or one guessed where
    /// there are no marks (out of sight too: closing a tcsh's `tail -f`
    /// asked nothing, and its tab showed nothing running).
    pub fn running(&self) -> bool {
        let guessed = if !self.integrated && !self.show_guesses { self.unseen.last() } else { None };
        self.pending_start.is_some() || self.blocks.last().or(guessed).is_some_and(|b| b.state() == BlockState::Running)
    }

    /// A command an Enter started where the shell marks no start (the sh
    /// family), not ended yet: its text ("" when not known).
    pub fn pending_command(&self) -> Option<&str> {
        self.pending_start.as_ref().map(|p| p.0.as_str())
    }

    /// Whitespace/bracket-delimited token under a cell (for clickable paths/URLs).
    pub fn token_at(&self, abs: u64, col: usize) -> Option<(String, usize, usize)> {
        let row = self.line(abs)?;
        let chars = row.chars_with_cols();
        let idx = chars.iter().rposition(|&(_, c)| c <= col)?;
        // Past the end of the line there is nothing (a line in the
        // scrollback ends at its last character; one cell more is the
        // second half of a wide one): the empty space to the right of a
        // path used to be the path.
        if idx + 1 == chars.len() && col > chars[idx].1 + 1 {
            return None;
        }
        let delim = |c: char| c.is_whitespace() || "\"'`<>()[]{}|;,".contains(c);
        if delim(chars[idx].0) {
            return None;
        }
        let mut s = idx;
        while s > 0 && !delim(chars[s - 1].0) {
            s -= 1;
        }
        let mut e = idx;
        while e + 1 < chars.len() && !delim(chars[e + 1].0) {
            e += 1;
        }
        let mut tok: String = chars[s..=e].iter().map(|c| c.0).collect();
        while tok.ends_with(['.', ':', ',']) {
            tok.pop();
            e -= 1;
        }
        if tok.is_empty() {
            return None;
        }
        let end_col = chars[e].1 + 1;
        Some((tok, chars[s].1, end_col))
    }

    /// All matches of `query` in the primary buffer: (abs line, start col, end col).
    pub fn search(&self, query: &str, icase: bool) -> Vec<(u64, usize, usize)> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        let q: Vec<char> = if icase { query.to_lowercase().chars().collect() } else { query.chars().collect() };
        for abs in self.first_line()..self.end_line() {
            let Some(row) = self.line(abs) else { continue };
            let chars = row.chars_with_cols();
            let hay: Vec<char> = chars
                .iter()
                .map(|&(c, _)| if icase { c.to_lowercase().next().unwrap_or(c) } else { c })
                .collect();
            if hay.len() < q.len() {
                continue;
            }
            let mut i = 0;
            while i + q.len() <= hay.len() {
                if hay[i..i + q.len()] == q[..] {
                    let end = chars[i + q.len() - 1].1 + 1;
                    out.push((abs, chars[i].1, end));
                    i += q.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    // ---- geometry -----------------------------------------------------------

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(2);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let attr = Attr::default();
        if cols != self.cols {
            // Width changed: rewrap the primary buffer (scrollback + screen) so
            // nothing is cut off and shrinking then growing restores the text.
            self.reflow_primary(cols, rows);
            if self.alt_active {
                // Full-screen apps redraw themselves; just fit the alt screen.
                for r in self.screen.iter_mut() {
                    r.resize(cols, attr);
                }
                self.screen.resize(rows, Row::new(cols, attr));
            }
            self.cols = cols;
            self.rows = rows;
            self.top = 0;
            self.bot = rows - 1;
            self.cx = self.cx.min(cols - 1);
            self.cy = self.cy.min(rows - 1);
            self.saved.x = self.saved.x.min(cols - 1);
            self.saved.y = self.saved.y.min(rows - 1);
            self.pending_wrap = false;
            self.generation = self.generation.wrapping_add(1);
            return;
        }
        if self.alt_active {
            self.screen.resize(rows, Row::new(cols, attr));
            let mut primary = self.primary_saved.take().unwrap_or_default();
            let mut y = self.saved.y;
            self.resize_primary_rows(&mut primary, rows, cols, &mut y);
            self.saved.y = y;
            self.primary_saved = Some(primary);
        } else {
            let mut screen = std::mem::take(&mut self.screen);
            let mut y = self.cy;
            self.resize_primary_rows(&mut screen, rows, cols, &mut y);
            self.cy = y;
            self.screen = screen;
        }
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bot = rows - 1;
        self.cx = self.cx.min(cols - 1);
        self.cy = self.cy.min(rows - 1);
        self.saved.x = self.saved.x.min(cols - 1);
        self.saved.y = self.saved.y.min(rows - 1);
        self.pending_wrap = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Rewrap scrollback + primary screen to `cols` columns and fit `rows`.
    /// Absolute line numbers change, so blocks are remapped here and the map
    /// is left in `line_remap` for anchors kept outside (inline widgets).
    fn reflow_primary(&mut self, cols: usize, rows: usize) {
        let alt = self.alt_active;
        let screen = if alt { self.primary_saved.take().unwrap_or_default() } else { std::mem::take(&mut self.screen) };
        let (cur_y, cur_x) = if alt { (self.saved.y, self.saved.x) } else { (self.cy, self.cx) };
        let base = self.dropped;
        let hist_len = self.history.len();
        let mut all: Vec<Row> = self.history.drain(..).collect();
        all.extend(screen);
        let (mut new, (mut ny, nx), map, end_map) = reflow(all, cols, (hist_len + cur_y, cur_x));
        // Blank rows below the cursor are not content.
        while new.len() > ny + 1 && new.last().map(|r| r.is_blank()).unwrap_or(false) {
            new.pop();
        }
        let start = new.len().saturating_sub(rows);
        let mut screen: Vec<Row> = new.split_off(start);
        while screen.len() < rows {
            screen.push(Row::new(cols, Attr::default()));
        }
        ny -= start;
        self.history = new.into();
        while self.history.len() > self.max_history {
            self.history.pop_front();
            self.dropped += 1;
        }
        if alt {
            self.primary_saved = Some(screen);
            self.saved.y = ny.min(rows - 1);
            self.saved.x = nx.min(cols - 1);
        } else {
            self.screen = screen;
            self.cy = ny.min(rows - 1);
            self.cx = nx.min(cols - 1);
        }
        let new_abs: Vec<u64> = map.iter().map(|&r| base + r as u64).collect();
        let remap = |abs: u64| -> u64 {
            if abs < base {
                abs
            } else {
                let i = (abs - base) as usize;
                match new_abs.get(i) {
                    Some(&n) => n,
                    None => new_abs.last().map(|&l| l + 1 + (i - new_abs.len()) as u64).unwrap_or(abs),
                }
            }
        };
        // (The guessed ones out of sight too: their lines were left as
        // they were.)
        for b in self.blocks.iter_mut().chain(self.unseen.iter_mut()) {
            b.prompt_line = remap(b.prompt_line);
            b.input = b.input.map(|(l, c)| (remap(l), c.min(cols - 1)));
            b.out_line = b.out_line.map(remap);
            b.end_line = b.end_line.map(|e| match e.checked_sub(base + 1).map(|i| i as usize) {
                Some(i) if i < end_map.len() => base + end_map[i] as u64 + 1,
                Some(_) => remap(e - 1) + 1,
                None => e,
            });
        }
        let dropped = self.dropped;
        self.blocks.retain(|b| b.prompt_line >= dropped);
        self.unseen.retain(|b| b.prompt_line >= dropped);
        self.line_remap = Some((base, new_abs));
    }

    /// New absolute line for an old one after the last reflow (for anchors
    /// kept outside the terminal, e.g. inline widgets).  Identity otherwise.
    pub fn take_line_remap(&mut self) -> Option<(u64, Vec<u64>)> {
        self.line_remap.take()
    }

    fn resize_primary_rows(&mut self, screen: &mut Vec<Row>, rows: usize, cols: usize, cy: &mut usize) {
        while screen.len() > rows {
            // Drop blank rows below the cursor first, then scroll the top into history.
            if screen.len() - 1 > *cy && screen.last().map(|r| r.is_blank()).unwrap_or(false) {
                screen.pop();
            } else {
                let r = screen.remove(0);
                self.push_history(r);
                *cy = cy.saturating_sub(1);
            }
        }
        while screen.len() < rows {
            screen.push(Row::new(cols, Attr::default()));
        }
    }

    fn push_history(&mut self, mut row: Row) {
        // Trailing plain blanks take no room in scrollback: every row held the
        // full width (16 bytes a column), so 20k short lines at 200 columns
        // cost ~60 MB. A soft-wrapped row keeps them (those spaces are text),
        // and so does a blank with an attribute (inverse, underline) that shows.
        if !row.wrapped {
            let plain = Cell::blank(Attr::default());
            let keep = row.cells.iter().rposition(|c| *c != plain).map_or(0, |i| i + 1);
            row.cells.truncate(keep);
            row.cells.shrink_to_fit();
        }
        self.history.push_back(row);
        while self.history.len() > self.max_history {
            self.history.pop_front();
            self.dropped += 1;
        }
        let dropped = self.dropped;
        self.blocks.retain(|b| b.prompt_line >= dropped);
        self.unseen.retain(|b| b.prompt_line >= dropped);
    }

    pub fn clear_history(&mut self) {
        self.dropped += self.history.len() as u64;
        self.history.clear();
        let dropped = self.dropped;
        self.blocks.retain(|b| b.prompt_line >= dropped);
        self.unseen.retain(|b| b.prompt_line >= dropped);
    }

    // ---- primitive operations --------------------------------------------

    fn blank_row(&self) -> Row {
        Row::new(self.cols, self.attr)
    }

    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.bot + 1 - self.top) {
            let row = self.screen.remove(self.top);
            if self.top == 0 && !self.alt_active {
                self.push_history(row);
            }
            let blank = self.blank_row();
            self.screen.insert(self.bot, blank);
        }
    }

    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.bot + 1 - self.top) {
            self.screen.remove(self.bot);
            let blank = self.blank_row();
            self.screen.insert(self.top, blank);
        }
    }

    fn linefeed(&mut self) {
        self.pending_wrap = false;
        if self.cy == self.bot {
            self.scroll_up(1);
        } else if self.cy + 1 < self.rows {
            self.cy += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.pending_wrap = false;
        if self.cy == self.top {
            self.scroll_down(1);
        } else if self.cy > 0 {
            self.cy -= 1;
        }
    }

    fn clear_wide_at(&mut self, x: usize) {
        let blank = Cell::blank(self.attr);
        let cols = self.cols;
        let row = &mut self.screen[self.cy].cells;
        if x >= cols {
            return;
        }
        match row[x].wide {
            WIDE_TAIL if x > 0 => row[x - 1] = blank,
            WIDE_LEAD if x + 1 < cols => row[x + 1] = blank,
            _ => {}
        }
    }

    fn erase_cells(&mut self, y: usize, from: usize, to: usize) {
        let blank = Cell::blank(self.attr);
        let to = to.min(self.cols);
        if from >= to {
            return;
        }
        let row = &mut self.screen[y];
        for c in &mut row.cells[from..to] {
            *c = blank;
        }
        if from > 0 && row.cells[from - 1].wide == WIDE_LEAD {
            row.cells[from - 1] = blank;
        }
        if to < self.cols && row.cells[to].wide == WIDE_TAIL {
            row.cells[to] = blank;
        }
        if to == self.cols {
            row.wrapped = false;
        }
    }

    fn put_char(&mut self, c: char) {
        let c = if self.line_drawing { dec_special(c) } else { c };
        let w = char_width(c);
        if w == 0 {
            self.combine(c);
            return;
        }
        if self.pending_wrap {
            if self.autowrap {
                self.screen[self.cy].wrapped = true;
                self.cx = 0;
                self.linefeed();
            }
            self.pending_wrap = false;
        }
        if w == 2 && self.cx + 1 >= self.cols {
            if !self.autowrap {
                return;
            }
            let pad = Cell { wide: WIDE_PAD, ..Cell::blank(self.attr) };
            self.screen[self.cy].cells[self.cx] = pad;
            self.screen[self.cy].wrapped = true;
            self.cx = 0;
            self.linefeed();
        }
        if self.insert_mode {
            let blank = Cell::blank(self.attr);
            let row = &mut self.screen[self.cy].cells;
            for _ in 0..w {
                row.insert(self.cx, blank);
                row.pop();
            }
        }
        self.clear_wide_at(self.cx);
        if w == 2 {
            self.clear_wide_at(self.cx + 1);
        }
        let attr = self.attr;
        let row = &mut self.screen[self.cy].cells;
        row[self.cx] = Cell { ch: c, attr, wide: if w == 2 { WIDE_LEAD } else { 0 } };
        if w == 2 {
            row[self.cx + 1] = Cell { ch: ' ', attr, wide: WIDE_TAIL };
        }
        self.last_char = c;
        let nx = self.cx + w;
        if nx >= self.cols {
            self.cx = self.cols - 1;
            self.pending_wrap = true;
        } else {
            self.cx = nx;
        }
    }

    /// A mark that goes on the letter before it (a combining accent, or
    /// Hangul's vowel and final consonant as macOS writes file names: 응
    /// as ㅇ ㅡ ㅇ, which `ls` there prints): put together with that letter
    /// where Unicode has the two as one. They were dropped, and a Korean
    /// name from a Mac showed as its first consonants.
    fn combine(&mut self, c: char) {
        let row = &mut self.screen[self.cy];
        let x = if self.pending_wrap { Some(self.cx) } else { self.cx.checked_sub(1) };
        let Some(mut x) = x else { return };
        if row.cells[x].wide == WIDE_TAIL && x > 0 {
            x -= 1;
        }
        // Only on the letter just printed (the cursor may have moved since).
        if row.cells[x].ch != self.last_char {
            return;
        }
        if let Some(joined) = unicode_normalization::char::compose(row.cells[x].ch, c) {
            row.cells[x].ch = joined;
            self.last_char = joined;
        }
    }

    fn goto(&mut self, x: usize, y: usize) {
        self.cx = x.min(self.cols - 1);
        self.cy = y.min(self.rows - 1);
        self.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor { x: self.cx, y: self.cy, attr: self.attr, line_drawing: self.line_drawing };
    }

    fn restore_cursor(&mut self) {
        let s = self.saved;
        self.attr = s.attr;
        self.line_drawing = s.line_drawing;
        self.goto(s.x, s.y);
    }

    fn enter_alt(&mut self) {
        if self.alt_active {
            return;
        }
        let blank: Vec<Row> = (0..self.rows).map(|_| Row::new(self.cols, Attr::default())).collect();
        self.primary_saved = Some(std::mem::replace(&mut self.screen, blank));
        self.alt_active = true;
        self.top = 0;
        self.bot = self.rows - 1;
    }

    fn leave_alt(&mut self) {
        if let Some(p) = self.primary_saved.take() {
            self.screen = p;
        }
        self.alt_active = false;
        self.top = 0;
        self.bot = self.rows - 1;
    }

    fn reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        self.leave_alt();
        self.screen = (0..rows).map(|_| Row::new(cols, Attr::default())).collect();
        self.attr = Attr::default();
        self.goto(0, 0);
        self.top = 0;
        self.bot = rows - 1;
        self.app_cursor = false;
        self.bracketed_paste = false;
        self.mouse_mode = 0;
        self.mouse_sgr = false;
        self.cursor_visible = true;
        self.autowrap = true;
        self.insert_mode = false;
        self.line_drawing = false;
    }

    // ---- shell integration (OSC 133) -----------------------------------------

    fn mark_prompt(&mut self) {
        if self.alt_active {
            return;
        }
        let line = self.cursor_abs();
        if let Some(b) = self.blocks.last_mut() {
            match b.state() {
                BlockState::Prompt => {
                    self.blocks.pop();
                }
                BlockState::Running => {
                    b.end_line = Some(line);
                    b.duration = b.started.map(|s| s.elapsed());
                }
                BlockState::Done => {}
            }
        }
        let id = self.next_block;
        self.next_block += 1;
        self.blocks.push(Block {
            guessed: false,
            private: false,
            id,
            prompt_line: line,
            input: None,
            out_line: None,
            end_line: None,
            exit: None,
            command: String::new(),
            started: None,
            duration: None,
            folded: false,
        });
    }

    fn mark_input(&mut self) {
        let pos = (self.cursor_abs(), self.cx);
        if let Some(b) = self.blocks.last_mut() {
            if b.state() == BlockState::Prompt {
                b.input = Some(pos);
            }
        }
    }

    fn mark_output(&mut self) {
        if self.alt_active {
            return;
        }
        let here = (self.cursor_abs(), self.cx);
        if self.start_at == Some(here) && self.blocks.last().is_some_and(|b| b.state() == BlockState::Running) {
            return;
        }
        self.start_at = Some(here);
        if self.blocks.last().map(|b| b.state() != BlockState::Prompt).unwrap_or(true) {
            self.mark_prompt();
        }
        let cmd = match self.blocks.last().and_then(|b| b.input) {
            Some(inp) => self.text_between(inp, here),
            None => String::new(),
        };
        let b = self.blocks.last_mut().unwrap();
        b.private = cmd.starts_with(char::is_whitespace);
        b.command = cmd.trim().to_string();
        b.out_line = Some(here.0 + if here.1 > 0 { 1 } else { 0 });
        b.started = Some(Instant::now());
    }

    fn mark_done(&mut self, exit: Option<i32>) {
        let end = self.cursor_abs() + if self.cx > 0 { 1 } else { 0 };
        self.finish_block(end, exit);
    }

    fn finish_block(&mut self, end: u64, exit: Option<i32>) {
        if let Some(b) = self.blocks.last_mut() {
            if b.state() == BlockState::Running {
                b.end_line = Some(end.max(b.out_line.unwrap_or(end)));
                b.exit = exit;
                b.duration = b.started.map(|s| s.elapsed());
            }
        }
    }

    // ---- guessing blocks for shells without OSC 133 -------------------------
    //
    // A prompt: output stopped with the cursor after some text on its line
    // (command output normally ends with a newline, leaving the cursor at
    // column 0). A command starts when the user presses Enter at that prompt,
    // and ends when output stops again with the cursor after text ending in
    // the same character as the prompt did. "Password: " or a REPL's ">>> "
    // end differently, so they don't end the command. No exit code.

    /// Guessing with the guessed blocks out of sight, where they are not
    /// shown.
    fn guessing<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let hide = !self.integrated && !self.show_guesses;
        if hide {
            std::mem::swap(&mut self.blocks, &mut self.unseen);
        }
        let r = f(self);
        if hide {
            std::mem::swap(&mut self.blocks, &mut self.unseen);
            let n = self.unseen.len();
            if n > 4 {
                self.unseen.drain(..n - 4);
            }
        }
        r
    }

    /// A prompt has come: marked by the shell, or guessed (shown or not).
    pub fn prompted(&self) -> bool {
        !self.blocks.is_empty() || !self.unseen.is_empty()
    }

    /// The last prompt is a guessed one that ends as a question (`:`,
    /// `?`): ssh's for a password, or a menu's.
    pub fn at_prompt_question(&self) -> bool {
        let last = if !self.integrated && !self.show_guesses { self.unseen.last() } else { self.blocks.last() };
        !self.integrated && last.is_some_and(|b| b.state() == BlockState::Prompt && b.guessed) && self.prompt_end.is_some_and(question_end)
    }

    /// What was typed at a guessed question is on the screen: the answer
    /// was echoed. A password, a one-time code or a PIN is asked with echo
    /// off, a menu's choice is seen as it is typed: that tells them apart
    /// in any language.
    pub fn question_answer_shown(&self) -> bool {
        let last = if !self.integrated && !self.show_guesses { self.unseen.last() } else { self.blocks.last() };
        let Some(input) = last.filter(|b| b.state() == BlockState::Prompt && b.guessed).and_then(|b| b.input) else { return false };
        !self.text_between(input, (self.cursor_abs(), self.cx)).trim().is_empty()
    }

    /// A shell's prompt has come: marked, or guessed where it is not a
    /// question (ssh's "password: " guessed one is not: the shell had not
    /// started).
    pub fn shell_prompted(&self) -> bool {
        self.integrated || (self.prompted() && !self.prompt_end.is_some_and(question_end))
    }

    /// The user sent Enter to the shell (call before writing it).
    pub fn user_enter(&mut self) {
        self.guessing(|t| t.user_enter_now())
    }

    /// The user typed something (not Enter alone) into this terminal.
    pub fn user_typed(&mut self) {
        self.typed = true;
    }

    /// weft sends `cmd` with the next Enter (it is not on the screen yet
    /// as that Enter is noted). None: that send was dropped.
    pub fn user_runs(&mut self, cmd: Option<&str>) {
        self.sent_command = cmd.map(str::to_string);
    }

    /// The user emptied the line (Ctrl-C, Ctrl-U): nothing typed at it.
    pub fn user_cleared(&mut self) {
        self.typed = false;
    }

    fn user_enter_now(&mut self) {
        let typed = self.typed;
        if self.alt_active || self.saw_start {
            return;
        }
        let Some(b) = self.blocks.last() else { return };
        if b.state() != BlockState::Prompt {
            return;
        }
        if self.integrated {
            // The shell may yet send 133;C (this may be its first command):
            // only note what was typed. The first Enter only: later ones go
            // to what runs (cat, a password), and moved the block's output
            // down past what it had printed.
            if self.pending_start.is_some() {
                return;
            }
            let here = (self.cursor_abs(), self.cx);
            let mut cmd = b.input.map(|inp| self.text_between(inp, here)).unwrap_or_default();
            // (Before what the screen has: what was left on the line is
            // still there as the Enter is noted, and it was taken.)
            if let Some(sent) = self.sent_command.take() {
                cmd = sent;
            }
            if cmd.trim().is_empty() && !typed {
                return;
            }
            self.pending_start = Some((cmd.trim_end().to_string(), here.0 + if here.1 > 0 { 1 } else { 0 }, Instant::now()));
            return;
        }
        // (The text weft sends: the screen has none of it yet.)
        let sent = self.sent_command.take();
        self.mark_output();
        if let Some(b) = self.blocks.last_mut() {
            b.guessed = true;
            if let Some(cmd) = sent {
                b.command = cmd;
            }
        }
    }

    /// Text before the cursor on its line, and whether the rest is blank.
    fn at_prompt_like(&self) -> Option<char> {
        let row = self.line(self.cursor_abs())?;
        if self.cx == 0 || !row.text_range(self.cx, row.cells.len()).trim().is_empty() {
            return None;
        }
        row.text_range(0, self.cx).trim_end().chars().last()
    }

    /// Advance the guesses; true while output is recent (call again soon).
    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    fn tick_at(&mut self, now: Instant) -> bool {
        self.guessing(|t| t.tick_now(now))
    }

    fn tick_now(&mut self, now: Instant) -> bool {
        if self.integrated || self.alt_active {
            return false;
        }
        let Some(last) = self.last_output else { return false };
        let idle = now.saturating_duration_since(last);
        let state = self.blocks.last().map(|b| b.state());
        match state {
            None | Some(BlockState::Done) => {
                if idle < Duration::from_millis(250) {
                    return true;
                }
                if let Some(c) = self.at_prompt_like() {
                    self.prompt_end = Some(c);
                    self.mark_prompt();
                    self.mark_input();
                    if let Some(b) = self.blocks.last_mut() {
                        b.guessed = true;
                    }
                }
                false
            }
            Some(BlockState::Running) => {
                if !self.blocks.last().map(|b| b.guessed).unwrap_or(false) {
                    return false;
                }
                if idle < Duration::from_millis(400) {
                    return true;
                }
                // The same end as the guessed prompt's; or, where that one
                // was a question (ssh's "password: " was guessed first, and
                // the shell's "$ " never ended its block), any other.
                let after_question = self.prompt_end.is_some_and(question_end);
                match self.at_prompt_like() {
                    Some(c) if Some(c) == self.prompt_end || (after_question && !question_end(c)) => {
                        self.prompt_end = Some(c);
                        // The prompt line itself is not output.
                        let end = self.cursor_abs();
                        self.finish_block(end, None);
                        self.events.push(TermEvent::GuessedDone);
                        self.mark_prompt();
                        self.mark_input();
                        if let Some(b) = self.blocks.last_mut() {
                            b.guessed = true;
                        }
                        false
                    }
                    _ => idle < Duration::from_secs(5),
                }
            }
            Some(BlockState::Prompt) => false,
        }
    }

    // ---- CSI helpers -----------------------------------------------------------

    fn sgr(&mut self, params: &Params) {
        let groups: Vec<&[u16]> = params.iter().collect();
        if groups.is_empty() {
            self.attr = Attr::default();
            return;
        }
        let mut i = 0;
        while i < groups.len() {
            let g = groups[i];
            match g[0] {
                0 => self.attr = Attr::default(),
                1 => self.attr.flags |= F_BOLD,
                2 => self.attr.flags |= F_DIM,
                3 => self.attr.flags |= F_ITALIC,
                4 => {
                    if g.get(1) == Some(&0) {
                        self.attr.flags &= !F_UNDER
                    } else {
                        self.attr.flags |= F_UNDER
                    }
                }
                7 => self.attr.flags |= F_INVERSE,
                8 => self.attr.flags |= F_HIDDEN,
                9 => self.attr.flags |= F_STRIKE,
                21 => self.attr.flags |= F_UNDER,
                22 => self.attr.flags &= !(F_BOLD | F_DIM),
                23 => self.attr.flags &= !F_ITALIC,
                24 => self.attr.flags &= !F_UNDER,
                27 => self.attr.flags &= !F_INVERSE,
                28 => self.attr.flags &= !F_HIDDEN,
                29 => self.attr.flags &= !F_STRIKE,
                n @ 30..=37 => self.attr.fg = Color::Idx((n - 30) as u8),
                39 => self.attr.fg = Color::Default,
                n @ 40..=47 => self.attr.bg = Color::Idx((n - 40) as u8),
                49 => self.attr.bg = Color::Default,
                n @ 90..=97 => self.attr.fg = Color::Idx((n - 90 + 8) as u8),
                n @ 100..=107 => self.attr.bg = Color::Idx((n - 100 + 8) as u8),
                n @ (38 | 48 | 58) => {
                    let (color, used) = parse_ext_color(&groups, i);
                    i += used;
                    if let Some(c) = color {
                        match n {
                            38 => self.attr.fg = c,
                            48 => self.attr.bg = c,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn set_mode(&mut self, private: bool, params: &Params, on: bool) {
        for g in params.iter() {
            let n = g[0];
            if !private {
                if n == 4 {
                    self.insert_mode = on;
                }
                continue;
            }
            match n {
                1 => self.app_cursor = on,
                7 => self.autowrap = on,
                25 => {
                    if !on && self.cursor_visible {
                        let line = if self.alt_active { self.cy as u64 } else { self.cursor_abs() };
                        self.hidden_from = Some(HiddenCursor { alt: self.alt_active, line, col: self.cx, at: Instant::now() });
                    }
                    self.cursor_visible = on;
                }
                47 | 1047 => {
                    if on {
                        self.enter_alt()
                    } else {
                        self.leave_alt()
                    }
                }
                1049 => {
                    if on {
                        self.save_cursor();
                        self.enter_alt();
                    } else {
                        self.leave_alt();
                        self.restore_cursor();
                    }
                }
                1000 | 1002 | 1003 => self.mouse_mode = if on { n } else { 0 },
                1006 => self.mouse_sgr = on,
                2004 => self.bracketed_paste = on,
                _ => {}
            }
        }
    }

    fn osc7(&mut self, url: &str) {
        let Some(rest) = url.strip_prefix("file://") else { return };
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let mut path = percent_decode(path);
        // Windows: file://host/C:/Users -> C:/Users
        if path.len() >= 3 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':' {
            path.remove(0);
        }
        self.set_cwd(Some(host.to_string()), path);
    }

    /// The directory found from outside the shell (its process, by the agent).
    pub fn set_cwd_from_process(&mut self, path: String) {
        self.set_cwd(None, path);
    }

    fn set_cwd(&mut self, host: Option<String>, path: String) {
        if host.is_some() {
            self.cwd_host = host;
        }
        if self.cwd.as_deref() != Some(path.as_str()) {
            self.cwd = Some(path);
            self.events.push(TermEvent::CwdChanged);
        }
    }
}

/// Cells of a row that matter: up to the last non-blank cell.
fn content_len(r: &Row) -> usize {
    r.cells.iter().rposition(|c| c.wide != WIDE_PAD && (c.ch != ' ' || c.attr.bg != Color::Default || c.wide != 0)).map(|i| i + 1).unwrap_or(0)
}

/// Rewrap rows to `cols`: soft-wrapped rows are joined into their logical
/// line and split again at the new width (a wide char never straddles a row
/// end).  Returns the rows, the cursor's new (row, col), and for every old row
/// the new row its first cell landed on.
fn reflow(rows: Vec<Row>, cols: usize, cursor: (usize, usize)) -> (Vec<Row>, (usize, usize), Vec<usize>, Vec<usize>) {
    let mut out: Vec<Row> = Vec::new();
    let mut map = vec![0usize; rows.len()];
    let mut end_map = vec![0usize; rows.len()];
    let mut new_cursor = (0, 0);
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j + 1 < rows.len() && rows[j].wrapped {
            j += 1;
        }
        // Join the logical line; remember where each old row started.
        let mut cells: Vec<Cell> = Vec::new();
        let mut starts = Vec::with_capacity(j - i + 1);
        for (k, row) in rows.iter().enumerate().take(j + 1).skip(i) {
            starts.push(cells.len());
            let take = if k < j { row.cells.len() } else { content_len(row) };
            cells.extend(row.cells[..take].iter().filter(|c| c.wide != WIDE_PAD));
        }
        let cursor_off = (i..=j).contains(&cursor.0).then(|| starts[cursor.0 - i] + cursor.1);
        // Split at the new width.
        let first = out.len();
        let mut row = Row::new(cols, Attr::default());
        let mut col = 0;
        let mut pos_of = vec![(first, 0usize); cells.len()];
        let mut k = 0;
        while k < cells.len() {
            let c = cells[k];
            if c.wide == WIDE_TAIL {
                // Orphan tail (its lead was cut off earlier): drop it.
                pos_of[k] = (first + (out.len() - first), col);
                k += 1;
                continue;
            }
            let w = if c.wide == WIDE_LEAD { 2 } else { 1 };
            if col + w > cols {
                for pad in &mut row.cells[col..] {
                    *pad = Cell { wide: WIDE_PAD, ..Cell::blank(Attr::default()) };
                }
                row.wrapped = true;
                out.push(std::mem::replace(&mut row, Row::new(cols, Attr::default())));
                col = 0;
            }
            pos_of[k] = (out.len(), col);
            row.cells[col] = c;
            if w == 2 {
                if k + 1 < cells.len() && cells[k + 1].wide == WIDE_TAIL {
                    pos_of[k + 1] = (out.len(), col + 1);
                    row.cells[col + 1] = cells[k + 1];
                    k += 1;
                } else {
                    row.cells[col + 1] = Cell { ch: ' ', attr: c.attr, wide: WIDE_TAIL };
                }
            }
            col += w;
            k += 1;
        }
        out.push(row);
        let last = out.len() - 1;
        for (n, &s) in starts.iter().enumerate() {
            map[i + n] = pos_of.get(s).map(|p| p.0).unwrap_or(last);
            // The new row holding this old row's last cell (the whole rest of the
            // logical line for its final row).
            let next = starts.get(n + 1).copied().unwrap_or(cells.len());
            end_map[i + n] = if n + 1 == starts.len() { last } else { next.checked_sub(1).and_then(|e| pos_of.get(e)).map(|p| p.0).unwrap_or(last) };
        }
        if let Some(off) = cursor_off {
            new_cursor = match pos_of.get(off) {
                Some(&p) => p,
                // Past the text (e.g. after the prompt): same distance after the last cell.
                None => {
                    let extra = off - cells.len();
                    let (r, c) = cells.len().checked_sub(1).map(|l| (pos_of[l].0, pos_of[l].1 + 1 + (cells[l].wide == WIDE_LEAD) as usize)).unwrap_or((first, 0));
                    let x = c + extra;
                    (r + x / cols.max(1), x % cols.max(1))
                }
            };
            while out.len() <= new_cursor.0 {
                out.push(Row::new(cols, Attr::default()));
            }
        }
        i = j + 1;
    }
    (out, new_cursor, map, end_map)
}

fn parse_ext_color(groups: &[&[u16]], i: usize) -> (Option<Color>, usize) {
    let g = groups[i];
    if g.len() > 1 {
        // Colon form: 38:5:n or 38:2:[cs]:r:g:b
        return match g[1] {
            5 if g.len() > 2 => (Some(Color::Idx(g[2] as u8)), 0),
            2 if g.len() >= 6 => (Some(Color::Rgb(g[3] as u8, g[4] as u8, g[5] as u8)), 0),
            2 if g.len() == 5 => (Some(Color::Rgb(g[2] as u8, g[3] as u8, g[4] as u8)), 0),
            _ => (None, 0),
        };
    }
    let at = |k: usize| groups.get(i + k).map(|g| g[0]);
    match at(1) {
        Some(5) => (at(2).map(|n| Color::Idx(n as u8)), 2),
        Some(2) => match (at(2), at(3), at(4)) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(r as u8, g as u8, b as u8)), 4),
            _ => (None, 4),
        },
        _ => (None, 0),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn dec_special(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

/// Terminal column width of a character (0, 1 or 2).
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 {
        return 0;
    }
    if u < 0x300 {
        return 1;
    }
    match u {
        // (0x3099, 0x309A: the kana voicing marks, as a Mac writes が.)
        0x300..=0x36F | 0x483..=0x489 | 0x1160..=0x11FF | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200B..=0x200F | 0x3099..=0x309A
        | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F => 0,
        0x1100..=0x115F
        | 0x231A..=0x231B
        | 0x2329..=0x232A
        | 0x23E9..=0x23EC
        | 0x23F0
        | 0x23F3
        | 0x25FD..=0x25FE
        | 0x2614..=0x2615
        | 0x2648..=0x2653
        | 0x267F
        | 0x2693
        | 0x26A1
        | 0x26AA..=0x26AB
        | 0x26BD..=0x26BE
        | 0x26C4..=0x26C5
        | 0x26CE
        | 0x26D4
        | 0x26EA
        | 0x26F2..=0x26F3
        | 0x26F5
        | 0x26FA
        | 0x26FD
        | 0x2705
        | 0x270A..=0x270B
        | 0x2728
        | 0x274C
        | 0x274E
        | 0x2753..=0x2755
        | 0x2757
        | 0x2795..=0x2797
        | 0x27B0
        | 0x27BF
        | 0x2B1B..=0x2B1C
        | 0x2B50
        | 0x2B55
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F900..=0x1F9FF
        | 0x1FA70..=0x1FAFF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

impl Perform for Term {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.events.push(TermEvent::Bell),
            0x08 => {
                if self.pending_wrap {
                    self.pending_wrap = false;
                } else if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            0x09 => {
                let next = ((self.cx / 8) + 1) * 8;
                self.cx = next.min(self.cols - 1);
                self.pending_wrap = false;
            }
            0x0A..=0x0C => self.linefeed(),
            0x0D => {
                self.cx = 0;
                self.pending_wrap = false;
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let ps: Vec<u16> = params.iter().map(|g| g[0]).collect();
        // `n`-th parameter, where 0/missing means `default`.
        let p = |i: usize, d: usize| -> usize {
            match ps.get(i) {
                Some(&v) if v > 0 => v as usize,
                _ => d,
            }
        };
        let private = intermediates.first() == Some(&b'?');
        match (action, intermediates) {
            // DECRQM: is a private mode known, and set? 1 set, 2 reset, 0 unknown.
            // Apps ask before using synchronized output (2026, see SyncOutput).
            ('p', [b'?', b'$']) => {
                let mode = ps.first().copied().unwrap_or(0);
                let state = match mode {
                    2026 => 2,
                    25 => if self.cursor_visible { 1 } else { 2 },
                    1049 | 47 | 1047 => if self.alt_active { 1 } else { 2 },
                    2004 => if self.bracketed_paste { 1 } else { 2 },
                    1 => if self.app_cursor { 1 } else { 2 },
                    _ => 0,
                };
                self.replies.extend(format!("\x1b[?{mode};{state}$y").into_bytes());
            }
            ('h', _) => self.set_mode(private, params, true),
            ('l', _) => self.set_mode(private, params, false),
            ('m', []) => self.sgr(params),
            ('@', []) => {
                let n = p(0, 1).min(self.cols - self.cx);
                let blank = Cell::blank(self.attr);
                let row = &mut self.screen[self.cy].cells;
                for _ in 0..n {
                    row.insert(self.cx, blank);
                    row.pop();
                }
            }
            ('A', []) => {
                let y = self.cy.saturating_sub(p(0, 1));
                let y = if self.cy >= self.top { y.max(self.top) } else { y };
                self.goto(self.cx, y);
            }
            ('B', []) | ('e', []) => {
                let y = self.cy + p(0, 1);
                let y = if self.cy <= self.bot { y.min(self.bot) } else { y };
                self.goto(self.cx, y);
            }
            ('C', []) | ('a', []) => self.goto(self.cx + p(0, 1), self.cy),
            ('D', []) => self.goto(self.cx.saturating_sub(p(0, 1)), self.cy),
            ('E', []) => self.goto(0, self.cy + p(0, 1)),
            ('F', []) => self.goto(0, self.cy.saturating_sub(p(0, 1))),
            ('G', []) | ('`', []) => self.goto(p(0, 1) - 1, self.cy),
            ('H', []) | ('f', []) => self.goto(p(1, 1) - 1, p(0, 1) - 1),
            ('d', []) => self.goto(self.cx, p(0, 1) - 1),
            ('I', []) => {
                for _ in 0..p(0, 1) {
                    self.cx = (((self.cx / 8) + 1) * 8).min(self.cols - 1);
                }
            }
            ('Z', []) => {
                for _ in 0..p(0, 1) {
                    self.cx = if self.cx == 0 { 0 } else { ((self.cx - 1) / 8) * 8 };
                }
            }
            ('J', _) => match ps.first().copied().unwrap_or(0) {
                0 => {
                    self.erase_cells(self.cy, self.cx, self.cols);
                    for y in self.cy + 1..self.rows {
                        self.erase_cells(y, 0, self.cols);
                    }
                }
                1 => {
                    for y in 0..self.cy {
                        self.erase_cells(y, 0, self.cols);
                    }
                    self.erase_cells(self.cy, 0, self.cx + 1);
                }
                2 => {
                    if !self.alt_active {
                        // Keep the cleared content in scrollback (and keep absolute lines stable).
                        let used = self.screen.iter().rposition(|r| !r.is_blank()).map(|i| i + 1).unwrap_or(0);
                        let (t, b) = (self.top, self.bot);
                        self.top = 0;
                        self.bot = self.rows - 1;
                        self.scroll_up(used);
                        self.top = t;
                        self.bot = b;
                    }
                    for y in 0..self.rows {
                        self.erase_cells(y, 0, self.cols);
                    }
                }
                3 => self.clear_history(),
                _ => {}
            },
            ('K', _) => match ps.first().copied().unwrap_or(0) {
                0 => self.erase_cells(self.cy, self.cx, self.cols),
                1 => self.erase_cells(self.cy, 0, self.cx + 1),
                2 => self.erase_cells(self.cy, 0, self.cols),
                _ => {}
            },
            ('L', []) => {
                if self.cy >= self.top && self.cy <= self.bot {
                    let n = p(0, 1).min(self.bot + 1 - self.cy);
                    for _ in 0..n {
                        self.screen.remove(self.bot);
                        let blank = self.blank_row();
                        self.screen.insert(self.cy, blank);
                    }
                    self.cx = 0;
                }
            }
            ('M', []) => {
                if self.cy >= self.top && self.cy <= self.bot {
                    let n = p(0, 1).min(self.bot + 1 - self.cy);
                    for _ in 0..n {
                        self.screen.remove(self.cy);
                        let blank = self.blank_row();
                        self.screen.insert(self.bot, blank);
                    }
                    self.cx = 0;
                }
            }
            ('P', []) => {
                let n = p(0, 1).min(self.cols - self.cx);
                let blank = Cell::blank(self.attr);
                let row = &mut self.screen[self.cy].cells;
                for _ in 0..n {
                    row.remove(self.cx);
                    row.push(blank);
                }
            }
            ('X', []) => {
                let n = p(0, 1);
                self.erase_cells(self.cy, self.cx, self.cx + n);
            }
            ('S', []) => self.scroll_up(p(0, 1)),
            ('T', []) => self.scroll_down(p(0, 1)),
            ('b', []) => {
                let c = self.last_char;
                for _ in 0..p(0, 1).min(4096) {
                    self.put_char(c);
                }
            }
            ('r', []) => {
                let top = p(0, 1) - 1;
                let bot = p(1, self.rows).min(self.rows) - 1;
                if top < bot {
                    self.top = top;
                    self.bot = bot;
                }
                self.goto(0, 0);
            }
            ('s', []) => self.save_cursor(),
            ('u', []) => self.restore_cursor(),
            ('n', _) => match ps.first().copied().unwrap_or(0) {
                5 => self.replies.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let r = if private {
                        format!("\x1b[?{};{}R", self.cy + 1, self.cx + 1)
                    } else {
                        format!("\x1b[{};{}R", self.cy + 1, self.cx + 1)
                    };
                    self.replies.extend_from_slice(r.as_bytes());
                }
                _ => {}
            },
            // 52: clipboard writes (OSC 52) work. Neovim 0.10+ over ssh
            // then yanks "+ into this computer's clipboard on its own.
            ('c', []) => self.replies.extend_from_slice(b"\x1b[?62;22;52c"),
            ('c', [b'>']) => self.replies.extend_from_slice(b"\x1b[>0;10;1c"),
            ('t', [])
                if ps.first() == Some(&18) => {
                    let r = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.replies.extend_from_slice(r.as_bytes());
                }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.cx = 0;
                self.linefeed();
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            ([b'('], b'0') => self.line_drawing = true,
            ([b'('], _) => self.line_drawing = false,
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let Some(code) = params.first() else { return };
        let arg = |i: usize| params.get(i).map(|p| String::from_utf8_lossy(p).into_owned()).unwrap_or_default();
        let rest = || {
            params[1..].iter().map(|p| String::from_utf8_lossy(p).into_owned()).collect::<Vec<_>>().join(";")
        };
        match *code {
            b"0" | b"2" => self.title = rest(),
            b"7" => self.osc7(&rest()),
            b"52" => {
                let data = arg(2);
                if data != "?" {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.trim()) {
                        self.events.push(TermEvent::Clipboard(String::from_utf8_lossy(&bytes).into_owned()));
                    }
                }
            }
            b"133" => {
                if !self.integrated {
                    // Guessed blocks so far give way to the shell's own marks.
                    self.blocks.retain(|b| !(b.guessed && b.state() != BlockState::Done));
                    self.unseen.clear();
                    self.integrated = true;
                }
                match arg(1).as_str() {
                    "A" => {
                        self.pending_start = None;
                        self.sent_command = None;
                        self.typed = false;
                        self.mark_prompt()
                    }
                    "B" => self.mark_input(),
                    "C" => {
                        self.saw_start = true;
                        self.pending_start = None;
                        self.mark_output()
                    }
                    "D" => {
                        // No C from this shell: the command started at Enter.
                        if let Some((cmd, out, at)) = self.pending_start.take() {
                            if let Some(b) = self.blocks.last_mut().filter(|b| b.state() == BlockState::Prompt) {
                                b.private = cmd.starts_with(char::is_whitespace);
                                b.command = cmd.trim().to_string();
                                b.out_line = Some(out);
                                b.started = Some(at);
                            }
                        }
                        // Shells without exit codes (cmd) send a bare D.
                        self.mark_done(arg(2).trim().parse().ok())
                    }
                    _ => {}
                }
            }
            b"7777" => {
                let raw = rest();
                let raw = raw.trim();
                let parsed = if raw.starts_with('{') {
                    serde_json::from_str(raw).ok()
                } else {
                    base64::engine::general_purpose::STANDARD
                        .decode(raw)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                };
                if let Some(value) = parsed {
                    self.events.push(TermEvent::Weft { anchor: self.cursor_abs(), value });
                }
            }
            b"9" => match arg(1).as_str() {
                // ConEmu / Windows Terminal: the current directory (cmd, oh-my-posh).
                "9" => {
                    let path = arg(2).trim().trim_matches('"').replace('\\', "/");
                    if !path.is_empty() {
                        self.set_cwd(None, path);
                    }
                }
                // Progress: not shown.
                "4" => {}
                _ => self.events.push(TermEvent::Notify(rest())),
            },
            b"777"
                if arg(1) == "notify" => {
                    let msg = params[2..].iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>().join(": ");
                    self.events.push(TermEvent::Notify(msg));
                }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if intermediates == b"+" && action == 'q' {
            self.xtgettcap = Some(Vec::new());
        }
    }

    fn put(&mut self, byte: u8) {
        if let Some(b) = self.xtgettcap.as_mut().filter(|b| b.len() < 1024) {
            b.push(byte);
        }
    }

    /// XTGETTCAP: apps ask for terminfo capabilities by hex-encoded name.
    /// Only Ms (clipboard writes through OSC 52) is answered: Neovim 0.10+
    /// asks for it to decide on its OSC 52 clipboard. Others: not known.
    fn unhook(&mut self) {
        let Some(req) = self.xtgettcap.take() else { return };
        for name in req.split(|&b| b == b';') {
            let hex = String::from_utf8_lossy(name).to_uppercase();
            let reply = match hex.as_str() {
                // "Ms" = \E]52;%p1%s;%p2%s\007
                "4D73" => format!("\x1bP1+r{hex}={}\x1b\\", hex_encode(b"\x1b]52;%p1%s;%p2%s\x07")),
                _ => format!("\x1bP0+r{hex}\x1b\\"),
            };
            self.replies.extend_from_slice(reply.as_bytes());
        }
    }
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|c| format!("{c:02X}")).collect()
}

/// Synchronized output (DEC mode 2026): an app starts an update with
/// `ESC[?2026h` and ends it with `ESC[?2026l`; what comes between is fed to
/// the terminal at once, so a frame never shows it half done (lines cleared
/// and not yet redrawn flickered as they came in separate reads). An update
/// that never ends is let through after `LIMIT`.
#[derive(Default)]
pub struct SyncOutput {
    held: Vec<u8>,
    since: Option<Instant>,
}

impl SyncOutput {
    const BEGIN: &'static [u8] = b"\x1b[?2026h";
    const END: &'static [u8] = b"\x1b[?2026l";
    pub const LIMIT: std::time::Duration = std::time::Duration::from_millis(150);

    /// New output in; what can be fed now out.
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<u8> {
        self.held.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            match self.since {
                None => match find(&self.held, Self::BEGIN) {
                    Some(i) => {
                        out.extend(self.held.drain(..i + Self::BEGIN.len()));
                        self.since = Some(now);
                    }
                    None => {
                        // A begin cut between two reads waits for the rest.
                        let keep = (1..Self::BEGIN.len()).rev().find(|&k| self.held.ends_with(&Self::BEGIN[..k])).unwrap_or(0);
                        let n = self.held.len() - keep;
                        out.extend(self.held.drain(..n));
                        return out;
                    }
                },
                Some(at) => match find(&self.held, Self::END) {
                    Some(i) => {
                        out.extend(self.held.drain(..i + Self::END.len()));
                        self.since = None;
                    }
                    None => {
                        if now.duration_since(at) >= Self::LIMIT {
                            out.append(&mut self.held);
                            self.since = None;
                        }
                        return out;
                    }
                },
            }
        }
    }

    /// All that is held, at once (the program has ended: no end of its
    /// update is coming).
    pub fn take(&mut self) -> Vec<u8> {
        self.since = None;
        std::mem::take(&mut self.held)
    }

    /// While an update is held: when to look again (it may time out).
    pub fn due(&self, now: Instant) -> Option<std::time::Duration> {
        self.since.map(|at| Self::LIMIT.saturating_sub(now.duration_since(at)))
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A guessed prompt that ends so is a question something else asks
/// ("password: ", "Verification code: ", "(yes/no/[fingerprint])? "), not
/// a shell's. (A list of how shells' prompts end left many out: `~`,
/// `)`, oh-my-zsh's.)
fn question_end(c: char) -> bool {
    // (And their full-width forms: a password asked in Chinese, 密码：)
    matches!(c, ':' | '?' | '：' | '？')
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    #[test]
    fn an_update_comes_in_whole() {
        let mut s = SyncOutput::default();
        let t = Instant::now();
        // Before an update: through at once.
        assert_eq!(s.push(b"plain", t), b"plain");
        // Begun: the clear is held until the redraw has come too.
        assert_eq!(s.push(b"\x1b[?2026h\x1b[2K", t), b"\x1b[?2026h");
        assert_eq!(s.push(b"new line", t), b"");
        assert_eq!(s.push(b"\x1b[?2026lafter", t), b"\x1b[2Knew line\x1b[?2026lafter");
        // A begin split between reads waits for its end.
        assert_eq!(s.push(b"x\x1b[?20", t), b"x");
        assert_eq!(s.push(b"26hy", t), b"\x1b[?2026h");
        // Never ended: let through after the limit.
        assert_eq!(s.push(b"", t + SyncOutput::LIMIT), b"y");
        assert_eq!(s.due(t), None);
    }

    #[test]
    fn apps_can_ask_for_it() {
        let mut t = Term::new(20, 4);
        let mut p = vte::Parser::new();
        t.feed(&mut p, b"\x1b[?2026$p");
        assert_eq!(t.replies, b"\x1b[?2026;2$y");
    }
}

#[cfg(test)]
mod guess_tests {
    use super::*;

    fn feed(t: &mut Term, s: &str) {
        let mut p = vte::Parser::new();
        t.feed(&mut p, s.as_bytes());
    }

    fn later(t: &Term, ms: u64) -> Instant {
        t.last_output.unwrap() + Duration::from_millis(ms)
    }

    /// Guesses not shown: no block at all, yet the prompt and the probable
    /// end of a command are still told; the shell's own marks show as ever.
    #[test]
    fn guesses_can_be_kept_out_of_sight() {
        let mut t = Term::new(80, 24);
        t.show_guesses = false;
        feed(&mut t, "welcome\r\n$ ");
        assert!(!t.prompted());
        t.tick_at(later(&t, 300));
        assert!(t.blocks.is_empty() && t.prompted(), "a prompt, and no block");
        assert!(t.at_prompt(), "at its prompt (Rerun, cd here): the guess out of sight counts");
        assert!(!t.running(), "nothing runs at the prompt");
        feed(&mut t, "ls");
        t.user_enter();
        // Running, out of sight: the close check and the tab know it.
        feed(&mut t, "\r\na.txt");
        t.tick_at(later(&t, 100));
        assert!(t.running() && !t.at_prompt(), "a guessed command runs");
        feed(&mut t, "  b.txt\r\n$ ");
        t.events.clear();
        t.tick_at(later(&t, 500));
        assert!(t.blocks.is_empty(), "nothing guessed is shown");
        assert!(t.events.iter().any(|e| matches!(e, TermEvent::GuessedDone)), "the command probably ended: told");
        assert!(t.at_prompt(), "at the prompt again");
        // ssh's password prompt, or a one-time code's, guessed a prompt
        // after its pause: not one to send a command to.
        // Shown or not. Not a shell's prompt either (the shell has not
        // started); the shell's, after the answer, is (its block ended at
        // it, though the first guess ended in ':').
        for shown in [false, true] {
            for asked in ["u@h's password: ", "Verification code: ", "Are you sure you want to continue connecting (yes/no/[fingerprint])? ", "密码：", "是否继续？ "] {
                let mut t = Term::new(24, 80);
                t.show_guesses = shown;
                feed(&mut t, asked);
                t.tick_at(later(&t, 300));
                assert!(t.prompted() && !t.at_prompt() && !t.shell_prompted(), "{asked} (shown: {shown})");
                t.user_typed();
                t.user_enter();
                // oh-my-zsh's prompt: it ends in `~`, as many do.
                feed(&mut t, "\r\nLast login: Mon\r\n➜  ~ ");
                t.tick_at(later(&t, 500));
                assert!(t.at_prompt() && t.shell_prompted(), "{asked}, then the shell's prompt (shown: {shown})");
                // A program that pauses on "45%": still running (the
                // prompt's end is the shell's now, not a question's).
                t.user_typed();
                t.user_enter();
                feed(&mut t, "wget x\r\n 45%");
                t.tick_at(later(&t, 500));
                assert!(!t.at_prompt(), "{asked}: a program's 45% is no prompt (shown: {shown})");
                feed(&mut t, "\r\ndone\r\n➜  ~ ");
                t.tick_at(later(&t, 500));
                assert!(t.at_prompt(), "{asked}: its prompt again (shown: {shown})");
            }
        }
        // Many commands: what is kept out of sight stays small.
        for _ in 0..20 {
            feed(&mut t, "ls");
            t.user_enter();
            feed(&mut t, "\r\nx\r\n$ ");
            t.tick_at(later(&t, 500));
        }
        assert!(t.unseen.len() <= 4 && t.blocks.is_empty());
        // The shell marks its prompts after all: shown.
        feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07");
        assert!(t.integrated && t.blocks.len() == 1 && t.unseen.is_empty());
    }

    #[test]
    fn blocks_are_guessed_without_osc_133() {
        let mut t = Term::new(80, 24);
        feed(&mut t, "welcome\r\n$ ");
        assert!(t.tick_at(later(&t, 100)), "still waiting for output to settle");
        assert!(!t.tick_at(later(&t, 300)));
        assert_eq!(t.blocks.len(), 1, "prompt found");
        // The user types `ls` and Enter.
        feed(&mut t, "ls");
        t.user_enter();
        assert_eq!(t.blocks[0].state(), BlockState::Running);
        assert_eq!(t.blocks[0].command, "ls");
        feed(&mut t, "\r\na.txt  b.txt\r\n");
        assert!(t.tick_at(later(&t, 500)), "output ended at column 0: not a prompt");
        feed(&mut t, "$ ");
        t.tick_at(later(&t, 500));
        let b = &t.blocks[0];
        assert_eq!(b.state(), BlockState::Done);
        assert!(b.guessed && b.exit.is_none(), "no exit code when guessed");
        assert_eq!(t.line(b.out_line.unwrap()).unwrap().text().trim_end(), "a.txt  b.txt");
        assert_eq!(b.end_line, Some(t.cursor_abs()), "the next prompt is not output");
        assert_eq!(t.blocks.len(), 2, "next prompt");
    }

    #[test]
    fn other_waits_dont_end_a_command() {
        let mut t = Term::new(80, 24);
        feed(&mut t, "$ ");
        t.tick_at(later(&t, 300));
        feed(&mut t, "sudo true");
        t.user_enter();
        feed(&mut t, "\r\n[sudo] password: ");
        t.tick_at(later(&t, 1000));
        assert_eq!(t.blocks[0].state(), BlockState::Running, "':' is not the prompt's '$'");
    }

    /// fish 4 sends OSC 133 itself, and weft's script sends it too: each
    /// mark comes twice. One command is still one block, with its exit code.
    #[test]
    fn marks_told_twice_are_one_block() {
        let mut t = Term::new(80, 24);
        let prompt = "\x1b]133;A;click_events=1\x1b\\\x1b]133;D;0\x07\x1b]133;A\x07> \x1b]133;B\x07\x1b]133;B\x1b\\";
        feed(&mut t, prompt);
        feed(&mut t, "echo hi\r\n\x1b]133;C;cmdline_url=echo%20hi\x1b\\\x1b]133;C\x07hi\r\n\x1b]133;D;0\x1b\\");
        feed(&mut t, prompt);
        let got: Vec<_> = t.blocks.iter().map(|b| (b.command.as_str(), b.exit, b.state())).collect();
        assert_eq!(got, [("echo hi", Some(0), BlockState::Done), ("", None, BlockState::Prompt)]);
        assert_eq!(t.block_output(&t.blocks[0]).trim(), "hi");
        // A failing one that prints nothing.
        feed(&mut t, "false\r\n\x1b]133;C;cmdline_url=false\x1b\\\x1b]133;C\x07\x1b]133;D;1\x1b\\");
        feed(&mut t, &prompt.replace("D;0", "D;1"));
        let got: Vec<_> = t.blocks.iter().map(|b| (b.command.as_str(), b.exit, b.state())).collect();
        assert_eq!(got[1..], [("false", Some(1), BlockState::Done), ("", None, BlockState::Prompt)]);
    }

    /// A shell that marks prompts and exit codes but not where input begins
    /// or a command starts (the sh family: D, A, no B or C): its command
    /// starts at the first Enter typed at its prompt. Enters typed into
    /// what runs (cat, a password) leave the block's output where it began;
    /// an Enter on an empty line starts nothing (it was a block with the
    /// last exit code).
    #[test]
    fn a_shell_with_start_marks_stays_so_after_an_empty_enter() {
        // zsh and bash before 4.4 send no C for an empty line, only D and
        // A. Their next Enter that runs nothing (a line continued, a menu's
        // choice) is not a command's start: still at the prompt, and no
        // block for it when Ctrl-C ends that line. (Deciding "marks its
        // starts" anew at each prompt broke this; see the sh family's.)
        let mut t = Term::new(80, 24);
        let prompt = |code: i32| format!("\x1b]133;D;{code}\x07\x1b]133;A\x07% \x1b]133;B\x07");
        feed(&mut t, &prompt(0));
        feed(&mut t, "ls");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "\r\n\x1b]133;C\x07file\r\n");
        feed(&mut t, &prompt(0));
        let done = t.blocks.iter().filter(|b| b.state() == BlockState::Done).count();
        t.user_enter();
        feed(&mut t, "\r\n");
        feed(&mut t, &prompt(0));
        feed(&mut t, "for f in *; do");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "\r\nfor> ");
        assert!(t.at_prompt() && t.pending_command().is_none(), "a continued line is not a running command");
        t.user_cleared();
        feed(&mut t, "^C\r\n");
        feed(&mut t, &prompt(130));
        assert_eq!(t.blocks.iter().filter(|b| b.state() == BlockState::Done && !b.command.is_empty()).count(), done, "nothing ran: no block");
    }

    #[test]
    fn a_menus_answer_is_seen_a_passwords_is_not() {
        // A password, a code, a PIN: asked with echo off, in any language.
        for asked in ["Passwort: ", "パスワード: ", "Пароль: ", "Verification code: ", "Enter PIN for 'PIV Card': "] {
            let mut t = Term::new(80, 24);
            t.show_guesses = false;
            feed(&mut t, asked);
            t.tick_at(later(&t, 300));
            assert!(t.at_prompt_question(), "{asked}");
            t.user_typed(); // the secret, not shown
            assert!(!t.question_answer_shown(), "{asked}");
        }
        // A menu's choice: shown as it is typed.
        for (asked, answer) in [("Select host: ", "web1"), ("Choice [1-3]? ", "2")] {
            let mut t = Term::new(80, 24);
            t.show_guesses = false;
            feed(&mut t, asked);
            t.tick_at(later(&t, 300));
            t.user_typed();
            feed(&mut t, answer);
            assert!(t.at_prompt_question() && t.question_answer_shown(), "{asked}");
        }
    }

    #[test]
    fn a_run_weft_sends_is_its_blocks_command() {
        // cmd: B and D marks, no C. Something was left on the line, and
        // weft runs `make` (Esc, then the command): as the Enter is noted
        // the screen still has what was left, or nothing of `make` yet.
        let mut t = Term::new(80, 24);
        feed(&mut t, "\x1b]133;A\x07C:> \x1b]133;B\x07junk");
        t.user_typed();
        t.user_runs(Some("make"));
        t.user_enter();
        assert_eq!(t.pending_command(), Some("make"), "the run's text, not what was left");
        feed(&mut t, "\r\nbuilt\r\n\x1b]133;D;0\x07\x1b]133;A\x07C:> \x1b]133;B\x07");
        assert!(t.blocks.iter().any(|b| b.state() == BlockState::Done && b.command == "make"));
        // The user's own Enter after: what is on the screen, as ever.
        feed(&mut t, "dir");
        t.user_typed();
        t.user_enter();
        assert_eq!(t.pending_command(), Some("dir"));
        // A shell without marks, its guesses shown: the run's text too.
        let mut t = Term::new(80, 24);
        feed(&mut t, "$ ");
        t.tick_at(later(&t, 300));
        feed(&mut t, "junk");
        t.user_typed();
        t.user_runs(Some("make"));
        t.user_enter();
        assert_eq!(t.blocks.last().map(|b| b.command.as_str()), Some("make"), "a guessed block's command");
    }

    #[test]
    fn a_shell_without_start_marks_takes_the_first_enter() {
        let prompt = |code: i32| format!("\x1b]133;D;{code}\x07\x1b]133;A\x07$ ");
        let mut t = Term::new(80, 24);
        feed(&mut t, &prompt(0));
        // cat, then two lines typed into it, then its end.
        feed(&mut t, "cat");
        t.user_typed();
        assert!(t.at_prompt() && t.pending_command().is_none(), "at the prompt, cat typed");
        t.user_enter();
        // Running (its block is still the prompt's until its end is told):
        // not at the prompt, for weft's run, cd here and the close check.
        assert!(!t.at_prompt() && t.pending_command().is_some(), "cat runs (its text not told by these shells)");
        feed(&mut t, "\r\none\r\n");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "one\r\ntwo\r\n");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "two\r\n");
        feed(&mut t, &prompt(0));
        let b = t.blocks.iter().find(|b| b.state() == BlockState::Done).expect("cat's block");
        assert_eq!(t.block_output(b).trim(), "one\none\ntwo\ntwo", "all of what cat printed");
        // A failing command, then Enter alone: no block for that.
        feed(&mut t, "false");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "\r\n");
        feed(&mut t, &prompt(1));
        let done = t.blocks.iter().filter(|b| b.state() == BlockState::Done).count();
        t.user_enter();
        feed(&mut t, "\r\n");
        feed(&mut t, &prompt(1));
        assert_eq!(t.blocks.iter().filter(|b| b.state() == BlockState::Done).count(), done, "an empty Enter made no block");
        assert_eq!(t.blocks.iter().rfind(|b| b.state() == BlockState::Done).and_then(|b| b.exit), Some(1), "false's code, once");
        // A key a running program took (less's q), then Enter alone at the
        // next prompt: no block (what was typed does not outlive a prompt).
        feed(&mut t, "less x");
        t.user_typed();
        t.user_enter();
        feed(&mut t, "\r\n");
        t.user_typed(); // q, to less
        feed(&mut t, &prompt(0));
        let done = t.blocks.iter().filter(|b| b.state() == BlockState::Done).count();
        t.user_enter();
        feed(&mut t, "\r\n");
        feed(&mut t, &prompt(0));
        assert_eq!(t.blocks.iter().filter(|b| b.state() == BlockState::Done).count(), done, "q was less's: no block");
        // Typed at the prompt, then emptied with Ctrl-C: Enter makes none.
        feed(&mut t, "oops");
        t.user_typed();
        t.user_cleared();
        feed(&mut t, "^C\r\n");
        t.user_enter();
        feed(&mut t, &prompt(130));
        assert_eq!(t.blocks.iter().filter(|b| b.state() == BlockState::Done).count(), done, "an emptied line: no block");
    }

    #[test]
    fn osc_133_replaces_guessing() {
        let mut t = Term::new(80, 24);
        feed(&mut t, "$ ");
        t.tick_at(later(&t, 300));
        feed(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07");
        assert!(t.integrated);
        assert_eq!(t.blocks.len(), 1);
        assert!(!t.blocks[0].guessed);
        // cmd style: no C, a bare D. The command dates from Enter; no exit code.
        feed(&mut t, "dir");
        t.user_enter();
        feed(&mut t, "\r\nout\r\n\x1b]133;D\x07");
        let b = &t.blocks[0];
        assert_eq!(b.state(), BlockState::Done);
        assert_eq!((b.command.as_str(), b.exit), ("dir", None));
        assert_eq!(t.line(b.out_line.unwrap()).unwrap().text().trim_end(), "out");
        // A shell that does send C: Enter before its first C changes nothing.
        let mut t = Term::new(80, 24);
        feed(&mut t, "\x1b]133;A\x07> \x1b]133;B\x07ls");
        t.user_enter();
        feed(&mut t, "\r\n\x1b]133;C\x07a\r\n\x1b]133;D;0\x07");
        assert_eq!(t.blocks.len(), 1);
        assert_eq!((t.blocks[0].command.as_str(), t.blocks[0].exit), ("ls", Some(0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut Term, s: &str) {
        let mut p = vte::Parser::new();
        t.feed(&mut p, s.as_bytes());
    }

    #[test]
    fn prints_wraps_and_scrolls_into_history() {
        let mut t = Term::new(5, 2);
        run(&mut t, "abcdefg\r\nxy\r\nz");
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.history[0].text(), "abcde");
        assert!(t.history[0].wrapped);
        assert_eq!(t.screen[0].text(), "xy");
        assert_eq!(t.screen[1].text(), "z");
        assert_eq!(t.lines_text(0, 4), "abcdefg\nxy\nz");
    }

    #[test]
    fn scrollback_keeps_no_trailing_blanks() {
        let mut t = Term::new(200, 3);
        run(&mut t, "short\r\n\x1b[7m \x1b[0m\r\n\r\nabc\r\nd\r\ne\r\nf");
        let lens: Vec<usize> = t.history.iter().map(|r| r.cells.len()).collect();
        // "short", an inverse blank (shows), an empty line, "abc".
        assert_eq!(lens, [5, 1, 0, 3]);
        // A soft-wrapped row keeps its spaces: they are text.
        let mut w = Term::new(4, 2);
        run(&mut w, "ab  cd\r\nx\r\ny");
        assert_eq!(w.history[0].cells.len(), 4);
        assert_eq!(w.lines_text(0, 2), "ab  cd");
        // Still whole after a resize rewraps it all.
        t.resize(10, 3);
        t.resize(200, 3);
        assert_eq!(t.lines_text(0, 7), "short\n\n\nabc\nd\ne\nf");
    }

    #[test]
    fn hangul_is_two_cells_wide() {
        let mut t = Term::new(10, 2);
        run(&mut t, "가a나");
        assert_eq!(t.cx, 5);
        assert_eq!(t.screen[0].cells[1].wide, WIDE_TAIL);
        assert_eq!(t.screen[0].text(), "가a나");
    }

    #[test]
    fn sgr_colors() {
        let mut t = Term::new(10, 1);
        run(&mut t, "\x1b[1;31mR\x1b[38;2;1;2;3mT\x1b[48;5;200mB\x1b[0mn");
        let c = &t.screen[0].cells;
        assert_eq!(c[0].attr.fg, Color::Idx(1));
        assert!(c[0].attr.flags & F_BOLD != 0);
        assert_eq!(c[1].attr.fg, Color::Rgb(1, 2, 3));
        assert_eq!(c[2].attr.bg, Color::Idx(200));
        assert_eq!(c[3].attr, Attr::default());
    }

    #[test]
    fn alt_screen_keeps_primary() {
        let mut t = Term::new(10, 3);
        run(&mut t, "hello\x1b[?1049h\x1b[2Jvim\x1b[?1049l");
        assert!(!t.alt_active);
        assert_eq!(t.screen[0].text(), "hello");
        assert!(t.history.is_empty());
    }

    #[test]
    fn osc133_blocks_capture_command_and_exit() {
        let mut t = Term::new(20, 4);
        run(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07ls -l\r\n\x1b]133;C\x07a\r\nb\r\n\x1b]133;D;2\x07\x1b]133;A\x07$ ");
        assert_eq!(t.blocks.len(), 2);
        let b = &t.blocks[0];
        assert_eq!(b.command, "ls -l");
        assert_eq!(b.exit, Some(2));
        assert_eq!(t.block_output(b), "a\nb");
        assert!(t.at_prompt());
        assert!(!b.private);
    }

    #[test]
    fn leading_space_makes_a_command_private() {
        // Integrated shell (OSC 133) and a guessed one (Enter, then D without C).
        let mut t = Term::new(30, 4);
        run(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07 export TOKEN=x\r\n\x1b]133;C\x07\x1b]133;D;0\x07");
        assert_eq!(t.blocks[0].command, "export TOKEN=x");
        assert!(t.blocks[0].private);
        let mut t = Term::new(30, 4);
        run(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07  secret");
        t.user_enter();
        run(&mut t, "\r\n\x1b]133;D;0\x07");
        assert_eq!(t.blocks[0].command, "secret");
        assert!(t.blocks[0].private);
    }

    #[test]
    fn clear_screen_keeps_absolute_lines() {
        let mut t = Term::new(10, 3);
        run(&mut t, "one\r\ntwo\x1b[H\x1b[2J");
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.line(0).unwrap().text(), "one");
        run(&mut t, "\x1b[3J");
        assert!(t.line(0).is_none());
        assert_eq!(t.screen_base(), 2);
    }

    #[test]
    fn osc7777_base64_event_and_osc7() {
        let mut t = Term::new(10, 3);
        let b64 = base64::engine::general_purpose::STANDARD.encode(r#"{"t":"notify","msg":"한글"}"#);
        run(&mut t, &format!("\r\n\x1b]7777;{}\x07\x1b]7;file://h/home/a%20b\x07", b64));
        assert_eq!(t.cwd.as_deref(), Some("/home/a b"));
        let ev = t.events.iter().find_map(|e| match e {
            TermEvent::Weft { anchor, value } => Some((*anchor, value.clone())),
            _ => None,
        });
        let (anchor, v) = ev.unwrap();
        assert_eq!(anchor, 1);
        assert_eq!(v["msg"], "한글");
    }

    /// An event as plain JSON (the shells send it so, with no base64 tool):
    /// any UTF-8 text survives, bytes 0x80-0xBF inside it included (ST's
    /// 0x9C among them, as in many Hangul syllables).
    #[test]
    fn osc7777_plain_json_keeps_any_text() {
        let text: String = ('가'..='힣').step_by(37).chain("é—…".chars()).collect();
        assert!(text.as_bytes().contains(&0x9c));
        let mut t = Term::new(10, 3);
        let js = serde_json::json!({"t": "notify", "msg": text}).to_string();
        run(&mut t, &format!("]7777;{js}"));
        let v = t.events.iter().find_map(|e| match e {
            TermEvent::Weft { value, .. } => Some(value.clone()),
            _ => None,
        });
        assert_eq!(v.expect("the event")["msg"], text.as_str());
    }

    #[test]
    fn osc_9_9_is_the_current_directory() {
        let mut t = Term::new(40, 3);
        run(&mut t, "\x1b]9;9;C:\\Users\\me\x1b\\\x1b]9;9;\"D:\\a b\"\x07");
        assert_eq!(t.cwd.as_deref(), Some("D:/a b"));
        assert!(t.events.iter().all(|e| !matches!(e, TermEvent::Notify(_))), "not a notification");
    }

    #[test]
    fn clipboard_support_is_reported() {
        // DA1 lists 52 (OSC 52 clipboard), as Neovim 0.11 checks.
        let mut t = Term::new(40, 3);
        run(&mut t, "\x1b[c");
        assert_eq!(t.replies, b"\x1b[?62;22;52c");
        // XTGETTCAP Ms (Neovim 0.10) and an unknown name, in one request.
        let mut t = Term::new(40, 3);
        run(&mut t, "\x1bP+q4d73;5a5a\x1b\\");
        let r = String::from_utf8(t.replies.clone()).unwrap();
        assert_eq!(r, format!("\x1bP1+r4D73={}\x1b\\\x1bP0+r5A5A\x1b\\", hex_encode(b"\x1b]52;%p1%s;%p2%s\x07")));
    }

    #[test]
    fn dsr_reply_and_token() {
        let mut t = Term::new(40, 3);
        run(&mut t, "see src/main.rs:12: here\x1b[6n");
        assert_eq!(t.replies, b"\x1b[1;25R");
        let (tok, s, _) = t.token_at(0, 6).unwrap();
        assert_eq!(tok, "src/main.rs:12");
        assert_eq!(s, 4);
        assert_eq!(t.search("main", false), vec![(0, 8, 12)]);
    }

    #[test]
    fn narrowing_then_widening_restores_the_prompt() {
        let mut t = Term::new(60, 5);
        let prompt = "sunghee in ssw06 in SC26-device-mode on HEAD (e207490)";
        run(&mut t, &format!("{prompt}\r\n> "));
        t.resize(20, 5);
        assert!(t.history.is_empty() || t.line(t.first_line()).is_some());
        let joined = t.lines_text(t.first_line(), t.end_line());
        assert!(joined.contains(prompt), "text lost while narrow: {joined:?}");
        t.resize(60, 5);
        assert_eq!(t.screen[0].text(), prompt);
        assert_eq!(t.screen[1].text(), ">");
        assert_eq!((t.cy, t.cx), (1, 2), "cursor stays after the prompt");
    }

    #[test]
    fn reflow_never_splits_a_wide_char() {
        let mut t = Term::new(10, 4);
        run(&mut t, "abc한글한글"); // 3 + 4*2 = 11 cells: wraps once
        t.resize(5, 4);
        for r in &t.screen {
            assert!(r.cells.last().map(|c| c.wide != WIDE_LEAD).unwrap_or(true), "wide char split at a row end");
        }
        t.resize(20, 4);
        assert_eq!(t.screen[0].text(), "abc한글한글");
    }

    #[test]
    fn reflow_moves_blocks_with_their_lines() {
        let mut t = Term::new(20, 6);
        run(&mut t, "\x1b]133;A\x07$ \x1b]133;B\x07echo aaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\x1b]133;C\x07aaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        let before = t.block_output(&t.blocks[0]);
        t.resize(8, 6);
        assert_eq!(t.blocks[0].command, "echo aaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(t.block_output(&t.blocks[0]).replace('\n', ""), before.replace('\n', ""));
        let remap = t.take_line_remap().expect("reflow reports the line map");
        assert_eq!(remap.0, t.first_line());
        t.resize(40, 6);
        assert_eq!(t.block_output(&t.blocks[0]), before);
    }

    #[test]
    fn resize_shrink_pushes_to_history() {
        let mut t = Term::new(10, 4);
        run(&mut t, "1\r\n2\r\n3\r\n4");
        t.resize(10, 2);
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.screen[1].text(), "4");
        assert_eq!(t.cy, 1);
    }

    /// The plain escapes: save and restore the cursor, index, next line,
    /// reverse index (at the top of the region the lines move down), the
    /// line drawing set, reset.
    #[test]
    fn plain_escapes_move_the_cursor_and_the_lines() {
        let rows = |t: &Term| t.screen.iter().map(|r| r.text()).collect::<Vec<_>>();
        let mut t = Term::new(10, 4);
        run(&mut t, "a\r\nb\r\nc\r\nd");
        // ESC 7 remembers where the cursor is, ESC 8 goes back there.
        run(&mut t, "\x1b[1;1H\x1b7\x1b[3;5H\x1b8X");
        assert_eq!(rows(&t), ["X", "b", "c", "d"]);
        // ESC D: a line down, the column kept. ESC E: the start of the next.
        run(&mut t, "\x1b[1;4H\x1bD");
        assert_eq!((t.cy, t.cx), (1, 3));
        run(&mut t, "\x1bE");
        assert_eq!((t.cy, t.cx), (2, 0));
        // ESC M: a line up…
        run(&mut t, "\x1bM");
        assert_eq!(t.cy, 1);
        // … and at the top the lines move down: the last one is lost, an
        // empty one comes in, nothing goes to the scrollback.
        run(&mut t, "\x1b[1;1H\x1bM");
        assert_eq!(rows(&t), ["", "X", "b", "c"]);
        assert!(t.history.is_empty());
        // In a scroll region only its lines move.
        run(&mut t, "\x1b[2;3r\x1b[2;1H\x1bM");
        assert_eq!(rows(&t), ["", "", "X", "c"]);
        // Several at once (CSI T), no more than the region has.
        run(&mut t, "\x1b[r\x1b[1;1Htop\x1b[9T");
        assert_eq!(rows(&t), ["", "", "", ""]);
        // The line drawing set: q is a horizontal line until ESC ( B.
        run(&mut t, "\x1b[1;1H\x1b(0q\x1b(Bq");
        assert_eq!(rows(&t)[0], "\u{2500}q");
        // ESC c: everything as at the start.
        run(&mut t, "\x1b[?1h\x1b[2;3r\x1b(0\x1bc");
        assert_eq!(rows(&t), ["", "", "", ""]);
        assert_eq!((t.cy, t.cx, t.top, t.bot), (0, 0, 0, 3));
        assert!(!t.app_cursor && !t.line_drawing);
        run(&mut t, "q");
        assert_eq!(rows(&t)[0], "q");
    }

    /// Letters and the marks after them (as macOS writes Korean file
    /// names, and as `ls` there prints them) make one character each in
    /// the grid: "응용프로그램" in six wide cells, not its first consonants.
    #[test]
    fn decomposed_letters_are_put_together() {
        let nfd: String = unicode_normalization::UnicodeNormalization::nfd("응용프로그램 파일.txt").collect();
        let mut t = Term::new(40, 3);
        run(&mut t, &nfd);
        assert_eq!(t.screen[0].text(), "응용프로그램 파일.txt");
        assert_eq!(t.cx, 6 * 2 + 1 + 2 * 2 + 4, "each syllable two cells wide");
        // An accent on a Latin letter.
        let mut t = Term::new(20, 2);
        run(&mut t, "cafe\u{301} ok");
        assert_eq!(t.screen[0].text(), "caf\u{e9} ok");
        // At the end of a line (the cursor waiting to wrap) as well.
        let mut t = Term::new(4, 3);
        run(&mut t, &unicode_normalization::UnicodeNormalization::nfd("ab한").collect::<String>());
        assert_eq!(t.screen[0].text(), "ab한");
        // Japanese: が as か and its voicing mark, in one wide cell.
        let mut t = Term::new(20, 2);
        run(&mut t, &unicode_normalization::UnicodeNormalization::nfd("がパ").collect::<String>());
        assert_eq!(t.screen[0].text(), "がパ");
        assert_eq!(t.cx, 4);
        // Text already composed, of any script, goes in as it is.
        let mut t = Term::new(60, 2);
        let s = "한글 日本語 がパ Tiếng Việt Ññ \u{F900}";
        run(&mut t, s);
        assert_eq!(t.screen[0].text(), s);
        // A mark with nothing to go on, or none it makes one with, is left out.
        let mut t = Term::new(20, 2);
        run(&mut t, "\u{301}x\u{1161}");
        assert_eq!(t.screen[0].text(), "x");
    }

    /// The word under the pointer, for Ctrl+click: up to a space, a quote
    /// or a bracket, without the full stop or colon after it; nothing on
    /// a space, and nothing past the end of the line.
    #[test]
    fn the_word_under_the_pointer() {
        let mut t = Term::new(70, 3);
        let line = "see (src/main.rs:12:3), or \"~/notes.md\". Then a.txt: 한글/파일.txt";
        run(&mut t, line);
        let abs = t.cursor_abs();
        let col = |s: &str| line.find(s).map(|b| line[..b].chars().count()).unwrap();
        let tok = |c: usize| t.token_at(abs, c);
        // Anywhere in the word gives the whole word and where it is.
        let path = col("src/");
        for c in [path, path + 7, path + 15] {
            assert_eq!(tok(c), Some(("src/main.rs:12:3".into(), path, path + 16)));
        }
        assert_eq!(tok(col("see")), Some(("see".into(), 0, 3)));
        // Not on a space, a bracket, a comma or a quote.
        for c in [3, col("("), col(")"), col(","), col("\"")] {
            assert_eq!(tok(c), None, "column {c}");
        }
        // Quotes are not part of it, nor the full stop or the colon after.
        let notes = col("~/");
        assert_eq!(tok(notes + 2), Some(("~/notes.md".into(), notes, notes + 10)));
        let a = col("a.txt");
        assert_eq!(tok(a), Some(("a.txt".into(), a, a + 5)));
        assert_eq!(tok(a + 5), Some(("a.txt".into(), a, a + 5)), "on the colon: still the word before it");
        // Hangul is two cells a letter: either cell is the letter.
        let h = col("한글");
        for c in [h, h + 1, h + 4, h + 12] {
            assert_eq!(tok(c), Some(("한글/파일.txt".into(), h, h + 13)), "column {c}");
        }
        // A line in the scrollback ends at its last letter: to the right
        // of it is nothing (it was the last word).
        let mut t = Term::new(30, 2);
        run(&mut t, "see /a/b.txt\r\nx\r\ny");
        assert_eq!(t.history[0].text(), "see /a/b.txt");
        assert_eq!(t.token_at(0, 11), Some(("/a/b.txt".into(), 4, 12)));
        assert_eq!(t.token_at(0, 13), None);
        assert_eq!(t.token_at(0, 25), None);
        assert_eq!(t.token_at(7, 0), None, "no such line");
    }
}
