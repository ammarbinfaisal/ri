mod raw;

use std::{fs::File, io::Write, process::exit};

use raw::*;
use rustix::{
    fd::BorrowedFd,
    io::{self, Errno},
    stdio,
    termios::tcgetwinsize,
};
use std::cmp::{max, min};

#[derive(PartialEq, Debug)]
enum EditorMode {
    Normal,
    Insert,
    Command,
}

#[derive(Debug)]
struct EditorConfig<'a> {
    cx: usize,
    cy: usize,
    max_x: usize,
    screenrows: u16,
    screencols: u16,
    stdout: BorrowedFd<'a>,
    stdin: BorrowedFd<'a>,
    rowoff: u16,
    coloff: u16,
    /// true when END has been pressed
    /// and left/HOME key hasn't been pressed
    rightted: bool,
    rows: Vec<EditorRow>,
    cmd: String,
    cmdix: usize,
    mode: EditorMode,
    cx_base: usize,
    log: File,
    filename: &'a str,
}

#[derive(Debug)]
struct EditorRow {
    chars: Vec<char>,
    len: usize,
}

#[derive(PartialEq, Debug)]
enum EditorKey {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    DelKey,
    HomeKey,
    EndKey,
    PageUp,
    PageDown,
    Backspace,
    Insert,
    K(u8),
}

impl EditorRow {
    fn new(s: &str) -> Self {
        Self {
            chars: s.chars().collect(),
            len: s.len(),
        }
    }

    fn remove(&mut self, ix: usize) {
        if ix < self.len {
            self.chars.remove(ix);
            self.len -= 1;
        }
    }

    fn insert(&mut self, ix: usize, c: char) {
        if ix < self.len {
            self.chars.insert(ix, c);
            self.len += 1;
        }
    }

    fn pop(&mut self) {
        if self.len > 0 {
            self.chars.pop();
            if self.len > 0 {
                self.len -= 1;
            }
        }
    }

    fn push(&mut self, c: char) {
        self.chars.push(c);
        self.len += 1;
    }
}

impl std::fmt::Display for EditorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = match self {
            EditorMode::Normal => "normal",
            EditorMode::Insert => "insert",
            EditorMode::Command => "",
        };
        // with background color pink and foreground color white
        write!(f, "{}", mode)
    }
}

const NEUTRAL_COLOR: &str = "\x1b[0m";

fn bg_color(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[48;2;{};{};{}m", r, g, b)
}

fn fg_color(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{};{};{}m", r, g, b)
}

impl<'editor> EditorConfig<'editor> {
    fn new(contents: &str, filename: &'editor str) -> Self {
        let file = File::create("log").unwrap();
        let mut rows = contents
            .lines()
            .map(|s| EditorRow::new(s))
            .collect::<Vec<_>>();
        if contents.chars().last() == Some('\n') {
            rows.push(EditorRow::new(""));
        }
        let cx_base = rows.len().to_string().len() + 4;
        Self {
            cx: cx_base,
            cy: 1,
            max_x: cx_base,
            screenrows: 0,
            screencols: 0,
            stdout: stdio::stdout(),
            stdin: stdio::stdin(),
            mode: EditorMode::Normal,
            cmd: String::new(),
            rows,
            rowoff: 0,
            coloff: 0,
            cmdix: 0,
            log: file,
            rightted: false,
            cx_base,
            filename,
        }
    }

    fn set_size(&mut self) {
        let prev = (self.screenrows, self.screencols);
        let winsize = tcgetwinsize(self.stdout);
        if let Ok(winsize) = winsize {
            if winsize.ws_row != 0 && winsize.ws_col != 0 {
                self.screenrows = winsize.ws_row;
                self.screencols = winsize.ws_col;
            }
            if prev != (self.screenrows, self.screencols) {
                self.get_cursor_position().unwrap();
            }
        }
    }

    fn refresh_screen(&mut self) {
        clear_screen();
        self.set_size();
        let mut buf = String::new();
        buf.push_str("\x1b[?25l");
        buf.push_str("\x1b[H");
        let textbg = bg_color(250, 238, 209);
        let blackfg = fg_color(0, 0, 0);
        let linenobg = bg_color(96, 115, 116);
        let cmdbg = bg_color(178, 165, 155);
        let row_count = self.rows.len();
        let rows_to_write = min(self.screenrows as usize - 1, row_count);
        for i in (self.rowoff as usize)..(self.rowoff as usize + rows_to_write) {
            let mut rowstr = format!(" {} ", i + 1);
            let l = rowstr.len();
            for _ in l..(self.cx_base - 2) {
                rowstr = format!(" {}", rowstr.clone());
            }
            buf.push_str(format!("\x1b[K{}{}{}", linenobg, rowstr, NEUTRAL_COLOR).as_str());
            buf.push_str(&textbg);
            buf.push_str(&blackfg);
            buf.push_str(" ");
            let row = &self.rows[i as usize];
            let len = min(
                self.screencols as usize - self.cx_base + self.coloff as usize,
                row.len,
            ) as usize;
            for j in self.coloff as usize..len {
                buf.push(row.chars[j]);
            }
            // blank space to the end of the line
            let subbed = if len > 0 && len > self.coloff as usize {
                len - self.coloff as usize
            } else {
                0
            };
            let space_count = self.screencols as usize - self.cx_base - subbed + 1;
            buf.push_str(" ".repeat(space_count).as_str());
            buf.push_str(NEUTRAL_COLOR);
            buf.push_str("\r\n");
        }
        // if space is left, fill it with tildes
        if rows_to_write < self.screenrows as usize - 1 {
            for _ in rows_to_write..self.screenrows as usize - 1 {
                buf.push_str("\x1b[K~\r\n");
            }
        }
        // move the cursor to the bottom of the screen
        buf.push_str("\x1b[H");
        buf.push_str("\x1b[?25h");
        if self.mode == EditorMode::Normal || self.mode == EditorMode::Insert {
            buf.push_str(&format!("\x1b[{};{}H", self.screenrows, 1,));
            buf.push_str("\x1b[K");
            // "-" * self.cx_base
            let dashes = "-".repeat(self.cx_base - 2);
            // B2A59B
            buf.push_str(&format!("{}", linenobg));
            buf.push_str(&dashes);
            buf.push_str(NEUTRAL_COLOR);
            buf.push_str(&format!("{}", cmdbg,));
            let mode = self.mode.to_string();
            buf.push_str(&mode);
            for _ in 0..self.screencols as usize - self.cx_base - mode.len() + 2 {
                buf.push(' ');
            }
            buf.push_str(NEUTRAL_COLOR);
        } else if self.mode == EditorMode::Command {
            buf.push_str(&format!("\x1b[{};{}H", self.screenrows, 1,));
            buf.push_str(&format!("{}", cmdbg,));
            buf.push_str("\x1b[K: ");
            buf.push_str(&self.cmd);
            buf.push_str(&format!("\x1b[{};{}H", self.screenrows, self.cmdix + 3,));
            buf.push_str(NEUTRAL_COLOR);
        }
        buf.push_str(&format!("\x1b[{};{}H", self.cy, self.cx));
        io::write(self.stdout, buf.as_bytes()).unwrap();
    }

    fn get_cursor_position(&mut self) -> Result<(), Errno> {
        io::write(self.stdout, "\x1b[999C\x1b[999B".as_bytes()).unwrap();
        let mut buf = [0u8; 32];
        io::read(self.stdin, &mut buf).unwrap();
        let mut cx = self.cx_base;
        let mut cy = 1;
        let mut i = 0;
        while i < buf.len() {
            if buf[i] == b'R' {
                break;
            }
            i += 1;
        }
        i += 1;
        while i < buf.len() {
            if buf[i] == b';' {
                break;
            }
            cx = cx * 10 + (buf[i] - b'0') as usize;
            i += 1;
        }
        i += 1;
        while i < buf.len() {
            if buf[i] == b'R' {
                break;
            }
            cy = cy * 10 + (buf[i] - b'0') as usize;
            i += 1;
        }
        self.cx = cx;
        self.cy = cy;
        Ok(())
    }

    fn read_key<'a>(&mut self) -> Result<u8, Errno> {
        let mut buf = [0u8; 1];
        io::read(self.stdin, &mut buf)?;
        Ok(buf[0])
    }

    fn read_editor_key<'a>(&mut self) -> Result<EditorKey, Errno> {
        let c = self.read_key()?;
        match c {
            b'\x1b' => {
                let mut buf = [0u8; 3];
                io::read(self.stdin, &mut buf)?;
                match buf[0] {
                    b'[' => match buf[1] {
                        b'D' => Ok(EditorKey::ArrowLeft),
                        b'C' => Ok(EditorKey::ArrowRight),
                        b'A' => Ok(EditorKey::ArrowUp),
                        b'B' => Ok(EditorKey::ArrowDown),
                        b'H' => Ok(EditorKey::HomeKey),
                        b'F' => Ok(EditorKey::EndKey),
                        b'1'..=b'8' => match buf[2] {
                            b'~' => match buf[1] {
                                b'1' => Ok(EditorKey::HomeKey),
                                b'2' => Ok(EditorKey::Insert),
                                b'3' => Ok(EditorKey::DelKey),
                                b'4' => Ok(EditorKey::EndKey),
                                b'5' => Ok(EditorKey::PageUp),
                                b'6' => Ok(EditorKey::PageDown),
                                b'7' => Ok(EditorKey::HomeKey),
                                b'8' => Ok(EditorKey::EndKey),
                                _ => Ok(EditorKey::K(c)),
                            },
                            _ => Ok(EditorKey::K(c)),
                        },
                        _ => Ok(EditorKey::K(c)),
                    },
                    b'O' => match buf[1] {
                        b'H' => Ok(EditorKey::HomeKey),
                        b'F' => Ok(EditorKey::EndKey),
                        _ => Ok(EditorKey::K(c)),
                    },
                    _ => Ok(EditorKey::K(c)),
                }
            }
            b'\x7f' => Ok(EditorKey::Backspace),
            _ => Ok(EditorKey::K(c)),
        }
    }

    fn curr_right_limit(&self) -> usize {
        let len = self.rows[self.rowoff as usize + self.cy - 1].len;
        min(
            self.screencols as usize,
            len + self.cx_base - self.coloff as usize,
        )
    }

    fn set_x_after_up_down(&mut self) {
        let len = self.rows[self.rowoff as usize + self.cy - 1].len;
        let rightlim = self.curr_right_limit();
        if len < self.coloff as usize {
            self.log
                .write_all(format!("len: {}\n", len).as_bytes())
                .unwrap();
            self.log
                .write_all(format!("coloff: {}\n", self.coloff).as_bytes())
                .unwrap();
            self.log.flush().unwrap();
            self.coloff = len as u16;
            self.cx = self.cx_base;
        } else if !self.rightted && self.max_x < rightlim {
            self.cx = self.max_x;
        } else {
            self.log
                .write_all(format!("rightlim: {}\n", rightlim).as_bytes())
                .unwrap();
            self.log.flush().unwrap();
            self.cx = rightlim;
        }
    }

    fn run<'a>(&mut self) -> Result<(), Errno> {
        // open a log file
        loop {
            self.refresh_screen();
            match self.read_editor_key() {
                Ok(key) => {
                    self.log
                        .write_all(&format!("{:?}\n", key).as_bytes())
                        .unwrap();
                    self.log.flush().unwrap();
                    match key {
                        EditorKey::Insert => match self.mode {
                            EditorMode::Normal => {
                                self.mode = EditorMode::Insert;
                            }
                            EditorMode::Insert => {
                                self.mode = EditorMode::Normal;
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::ArrowLeft => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cx == self.cx_base {
                                    if self.coloff > 0 {
                                        self.coloff -= 1;
                                    }
                                } else {
                                    self.cx -= 1;
                                    self.max_x = self.cx;
                                }
                                self.max_x = self.cx;
                                self.rightted = false;
                            }
                            EditorMode::Command => {
                                if self.cmdix != 0 {
                                    self.cmdix -= 1;
                                }
                            }
                        },
                        EditorKey::ArrowRight => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cx == self.screencols as usize
                                    && (self.cx - self.cx_base + self.coloff as usize)
                                        < self.rows[self.rowoff as usize + self.cy - 1].len
                                {
                                    self.log.write_all("at right\n".as_bytes()).unwrap();
                                    self.coloff += 1;
                                } else {
                                    let rightlim = self.curr_right_limit();
                                    if self.cx < rightlim {
                                        self.cx += 1;
                                    } else {
                                        self.cx = rightlim;
                                    }
                                }
                                self.max_x = self.cx;
                            }
                            EditorMode::Command => {
                                if self.cmdix != self.cmd.len() {
                                    self.cmdix += 1;
                                }
                            }
                        },
                        EditorKey::ArrowUp => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cy == 1 {
                                    if self.rowoff > 0 {
                                        self.rowoff -= 1;
                                    }
                                } else {
                                    self.cy -= 1;
                                }
                                self.set_x_after_up_down();
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::ArrowDown => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cy + 1 == self.screenrows as usize {
                                    if (self.cy + self.rowoff as usize) < self.rows.len() {
                                        self.rowoff += 1;
                                    }
                                } else if self.cy < self.screenrows as usize - 1 {
                                    self.cy += 1;
                                }
                                self.set_x_after_up_down();
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::DelKey => match self.mode {
                            EditorMode::Insert | EditorMode::Normal => {
                                if (self.cx + self.coloff as usize - self.cx_base)
                                    == self.curr_right_limit()
                                {
                                    self.rows[self.cy - 1].pop();
                                    self.cx -= 1;
                                } else if self.cx >= self.cx_base {
                                    self.rows[self.cy - 1].remove(self.cx - self.cx_base);
                                }
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::HomeKey => {
                            self.coloff = 0;
                            self.cx = self.cx_base;
                            self.max_x = self.cx;
                            self.rightted = false;
                        }
                        EditorKey::EndKey => {
                            if self.screencols as usize - self.cx_base
                                < self.rows[self.rowoff as usize + self.cy - 1].len
                            {
                                // such that the cursor is at the end of the screen
                                self.coloff = (self.rows[self.rowoff as usize + self.cy - 1].len
                                    - (self.screencols as usize - self.cx_base))
                                    as u16;
                                self.cx = self.screencols as usize;
                            } else {
                                self.cx = min(
                                    self.cx_base
                                        + self.rows[self.rowoff as usize + self.cy - 1].len,
                                    self.screencols as usize,
                                );
                            }
                            self.max_x = self.cx;
                            self.rightted = true;
                        }
                        EditorKey::PageUp => {
                            let row_offset = self.screenrows as usize - self.cy - 1;
                            if self.rowoff > row_offset as u16 {
                                self.rowoff -= row_offset as u16 + 1;
                            } else {
                                self.rowoff = 0;
                            }
                            self.cy = 1;
                            self.set_x_after_up_down();
                        }
                        EditorKey::PageDown => {
                            let row_count = self.rows.len();
                            let bottom = self.screenrows as usize - 1;
                            if ((self.rowoff) as usize + self.cy - 1 + bottom) < self.rows.len() {
                                self.rowoff += self.cy as u16;
                            } else {
                                self.rowoff = (row_count - self.cy) as u16;
                            }
                            self.cy = bottom;
                            self.set_x_after_up_down();
                        }
                        EditorKey::Backspace => match self.mode {
                            EditorMode::Insert | EditorMode::Normal => {
                                if self.cx > self.cx_base {
                                    self.rows[self.cy - 1].remove(self.cx - self.cx_base - 1);
                                    self.cx -= 1;
                                }
                            }
                            EditorMode::Command => {
                                if self.cmdix != 0 {
                                    self.cmd.remove(self.cmdix - 1);
                                    self.cmdix -= 1;
                                }
                            }
                        },
                        EditorKey::K(c) => match self.mode {
                            EditorMode::Normal => match c {
                                b'i' => {
                                    self.mode = EditorMode::Insert;
                                }
                                b':' => {
                                    self.mode = EditorMode::Command;
                                }
                                _ => {}
                            },
                            EditorMode::Insert => match c {
                                b'\x1b' => {
                                    self.mode = EditorMode::Normal;
                                }
                                _ => {
                                    if c > 31 && c < 127 {
                                        // insert the character at the cursor position
                                        if self.coloff as usize + self.cx - self.cx_base
                                            >= self.rows[self.rowoff as usize + self.cy - 1].len
                                        {
                                            self.rows[self.rowoff as usize + self.cy - 1]
                                                .push(c as char);
                                            self.max_x = max(self.max_x, self.cx);
                                        } else {
                                            self.rows[self.rowoff as usize + self.cy - 1].insert(
                                                self.rowoff as usize + self.cx - self.cx_base,
                                                c as char,
                                            );
                                        }
                                        self.cx += 1;
                                        self.max_x = max(self.max_x, self.cx);
                                    }
                                }
                            },
                            EditorMode::Command => match c {
                                b'\x1b' => {
                                    self.mode = EditorMode::Normal;
                                }
                                b'\r' => {
                                    self.mode = EditorMode::Normal;
                                    match self.cmd.as_str() {
                                        "q" => {
                                            return Ok(());
                                        }
                                        "w" => {
                                            let mut file =
                                                File::create(format!("{}.t", self.filename))
                                                    .unwrap();
                                            for ix in 0..self.rows.len() - 1 {
                                                let row = &self.rows[ix];
                                                file.write_all(
                                                    &row.chars
                                                        .iter()
                                                        .collect::<String>()
                                                        .as_bytes(),
                                                )
                                                .unwrap();
                                                file.write_all("\n".as_bytes()).unwrap();
                                            }
                                        }
                                        _ => {}
                                    }
                                    self.cmd.clear();
                                    self.cmdix = 0;
                                }
                                b'\x7f' => {
                                    if self.cmdix != 0 {
                                        self.cmd.remove(self.cmdix - 1);
                                        self.cmdix -= 1;
                                    }
                                }
                                _ => {
                                    if c > 31 && c < 127 {
                                        if self.cmdix == self.cmd.len() {
                                            self.cmd.push(c as char);
                                        } else {
                                            self.cmd.insert(self.cmdix, c as char);
                                        }
                                        self.cmdix += 1;
                                    }
                                }
                            },
                        },
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1);
    let file = if let Some(arg) = arg {
        arg
    } else {
        return;
    };
    let old_termios = match enable_raw_mode() {
        Ok(t) => t,
        Err(e) => {
            println!("error: {:?}", e);
            exit(1);
        }
    };
    let contents = std::fs::read_to_string(file.clone()).unwrap();
    let mut editor = EditorConfig::new(&contents, &file);
    loop {
        let res = editor.run();
        disable_raw_mode(&old_termios);
        match res {
            Err(e) => {
                println!("error: {:?}", e);
            }
            _ => {}
        }
        break;
    }
}
