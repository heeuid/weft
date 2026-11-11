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

    /// Text of columns `[from, to)`, wide-char tails skipped, trailing blanks trimmed.
    pub fn text_range(&self, from: usize, to: usize) -> String {
        let to = to.min(self.cells.len());
        let mut s: String = self.cells[from.min(to)..to]
            .iter()
            .filter(|c| c.wide != WIDE_TAIL)
            .map(|c| c.ch)
            .collect();
        s.truncate(s.trim_end().len());
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
            .filter(|(_, c)| c.wide != WIDE_TAIL)
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
    /// OSC 7777 weft protocol message, anchored at the cursor's absolute line.
    Weft { anchor: u64, value: serde_json::Value },
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
    autowrap: bool,
    insert_mode: bool,
    line_drawing: bool,
    last_char: char,
    pub title: String,
    pub cwd: Option<String>,
    pub cwd_host: Option<String>,
    pub replies: Vec<u8>,
    pub events: Vec<TermEvent>,
    pub blocks: Vec<Block>,
    next_block: u64,
    /// Bumped on every mutation; lets the UI know something changed.
    pub generation: u64,
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
            autowrap: true,
            insert_mode: false,
            line_drawing: false,
            last_char: ' ',
            title: String::new(),
            cwd: None,
            cwd_host: None,
            replies: Vec::new(),
            events: Vec::new(),
            blocks: Vec::new(),
            next_block: 1,
            generation: 0,
        }
    }

    pub fn feed(&mut self, parser: &mut vte::Parser, bytes: &[u8]) {
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
        !self.alt_active && self.blocks.last().map(|b| b.state() == BlockState::Prompt).unwrap_or(false)
    }

    /// Whitespace/bracket-delimited token under a cell (for clickable paths/URLs).
    pub fn token_at(&self, abs: u64, col: usize) -> Option<(String, usize, usize)> {
        let row = self.line(abs)?;
        let chars = row.chars_with_cols();
        let idx = chars.iter().rposition(|&(_, c)| c <= col)?;
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
        for r in self.screen.iter_mut() {
            r.resize(cols, attr);
        }
        if let Some(p) = self.primary_saved.as_mut() {
            for r in p.iter_mut() {
                r.resize(cols, attr);
            }
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

    fn push_history(&mut self, row: Row) {
        self.history.push_back(row);
        while self.history.len() > self.max_history {
            self.history.pop_front();
            self.dropped += 1;
        }
        let dropped = self.dropped;
        self.blocks.retain(|b| b.prompt_line >= dropped);
    }

    pub fn clear_history(&mut self) {
        self.dropped += self.history.len() as u64;
        self.history.clear();
        let dropped = self.dropped;
        self.blocks.retain(|b| b.prompt_line >= dropped);
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
            let blank = Cell::blank(self.attr);
            self.screen[self.cy].cells[self.cx] = blank;
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
        if self.blocks.last().map(|b| b.state() != BlockState::Prompt).unwrap_or(true) {
            self.mark_prompt();
        }
        let here = (self.cursor_abs(), self.cx);
        let cmd = match self.blocks.last().and_then(|b| b.input) {
            Some(inp) => self.text_between(inp, here),
            None => String::new(),
        };
        let b = self.blocks.last_mut().unwrap();
        b.command = cmd.trim().to_string();
        b.out_line = Some(here.0 + if here.1 > 0 { 1 } else { 0 });
        b.started = Some(Instant::now());
    }

    fn mark_done(&mut self, exit: i32) {
        let end = self.cursor_abs() + if self.cx > 0 { 1 } else { 0 };
        if let Some(b) = self.blocks.last_mut() {
            if b.state() == BlockState::Running {
                b.end_line = Some(end.max(b.out_line.unwrap_or(end)));
                b.exit = Some(exit);
                b.duration = b.started.map(|s| s.elapsed());
            }
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
                25 => self.cursor_visible = on,
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
        self.cwd_host = Some(host.to_string());
        if self.cwd.as_deref() != Some(path.as_str()) {
            self.cwd = Some(path);
            self.events.push(TermEvent::CwdChanged);
        }
    }
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
        0x300..=0x36F | 0x483..=0x489 | 0x1160..=0x11FF | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200B..=0x200F
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
            ('c', []) => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            ('c', [b'>']) => self.replies.extend_from_slice(b"\x1b[>0;10;1c"),
            ('t', []) => {
                if ps.first() == Some(&18) {
                    let r = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.replies.extend_from_slice(r.as_bytes());
                }
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
            b"133" => match arg(1).as_str() {
                "A" => self.mark_prompt(),
                "B" => self.mark_input(),
                "C" => self.mark_output(),
                "D" => self.mark_done(arg(2).trim().parse().unwrap_or(0)),
                _ => {}
            },
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
            b"9" => {
                if arg(1) != "4" {
                    self.events.push(TermEvent::Notify(rest()));
                }
            }
            b"777" => {
                if arg(1) == "notify" {
                    let msg = params[2..].iter().map(|p| String::from_utf8_lossy(p)).collect::<Vec<_>>().join(": ");
                    self.events.push(TermEvent::Notify(msg));
                }
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
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
    fn resize_shrink_pushes_to_history() {
        let mut t = Term::new(10, 4);
        run(&mut t, "1\r\n2\r\n3\r\n4");
        t.resize(10, 2);
        assert_eq!(t.history.len(), 2);
        assert_eq!(t.screen[1].text(), "4");
        assert_eq!(t.cy, 1);
    }
}
