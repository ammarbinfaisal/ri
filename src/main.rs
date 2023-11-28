mod raw;

use std::{fs::File, io::Write, process::exit};

use raw::*;
use rustix::{
    fd::BorrowedFd,
    io::{self, Errno},
    stdio,
    termios::tcgetwinsize,
};
use std::cmp::{min, max};

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
    rows: Vec<String>,
    cmd: String,
    cmdix: usize,
    mode: EditorMode,
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
    K(u8),
}

const CX_INIT: usize = 4;

impl<'editor> EditorConfig<'editor> {
    fn new(contents: &str) -> Self {
        Self {
            cx: CX_INIT,
            cy: 1,
            max_x: CX_INIT,
            screenrows: 0,
            screencols: 0,
            stdout: stdio::stdout(),
            stdin: stdio::stdin(),
            mode: EditorMode::Normal,
            cmd: String::new(),
            rows: contents.split("\n").map(|s| s.to_string()).collect(),
            rowoff: 0,
            coloff: 0,
            cmdix: 0,
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
        self.set_size();
        let mut buf = String::new();
        buf.push_str("\x1b[?25l");
        buf.push_str("\x1b[H");
        let row_count = self.rows.len() as u16;
        for i in self.rowoff..self.screenrows {
            buf.push_str(format!("{:2} ", i + 1).as_str());
            buf.push_str("\x1b[K");
            if i < self.screenrows - 1 {
                if i < row_count {
                    let row = self.rows[i as usize].clone();
                    if (self.coloff as usize) < row.len() {
                        buf.push_str(&row[self.coloff as usize..]);
                    }
                }
                buf.push_str("\r\n");
            }
        }
        buf.push_str("\x1b[H");
        buf.push_str("\x1b[?25h");
        if self.mode == EditorMode::Normal || self.mode == EditorMode::Insert {
            buf.push_str(&format!("\x1b[{};{}H", self.cy, self.cx));
        } else if self.mode == EditorMode::Command {
            buf.push_str(&format!("\x1b[{};{}H", self.screenrows, 1,));
            buf.push_str("\x1b[K: ");
            buf.push_str(&self.cmd);
            buf.push_str(&format!("\x1b[{};{}H", self.screenrows, self.cmdix + 3,));
        }
        io::write(self.stdout, buf.as_bytes()).unwrap();
    }

    fn get_cursor_position(&mut self) -> Result<(), Errno> {
        io::write(self.stdout, "\x1b[999C\x1b[999B".as_bytes()).unwrap();
        let mut buf = [0u8; 32];
        io::read(self.stdin, &mut buf).unwrap();
        let mut cx = CX_INIT;
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
            _ => Ok(EditorKey::K(c)),
        }
    }

    fn curr_right_limit(&self) -> usize {
        let lim = if self.mode == EditorMode::Insert {
            0 as i64
        } else {
            -1
        };
        let res = (min(self.screencols as usize, self.rows[self.cy - 1].len())) as i64 + lim;
        if res < 0 {
            0
        } else {
            res as usize
        }
    }

    fn set_cx(&mut self) {
        if self.cx < self.curr_right_limit() + CX_INIT {
            self.cx = self.max_x;
        } else {
            self.cx = self.curr_right_limit() + CX_INIT;
        }
    }

    fn run<'a>(&mut self) -> Result<(), Errno> {
        // open a log file
        let mut file = File::create("log").unwrap();
        loop {
            self.refresh_screen();
            match self.read_editor_key() {
                Ok(key) => {
                    file.write_all(&format!("{:?}\n", key).as_bytes()).unwrap();
                    file.flush().unwrap();
                    match key {
                        EditorKey::ArrowLeft => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cx != CX_INIT {
                                    self.cx -= 1;
                                    self.max_x = self.cx;
                                }
                            }
                            EditorMode::Command => {
                                if self.cmdix != 0 {
                                    self.cmdix -= 1;
                                }
                            }
                        },
                        EditorKey::ArrowRight => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cx - CX_INIT < self.curr_right_limit() {
                                    self.cx += 1;
                                } else {
                                    self.cx = self.curr_right_limit() + CX_INIT;
                                }
                                self.max_x = max(self.max_x, self.cx);
                            }
                            EditorMode::Command => {
                                if self.cmdix != self.cmd.len() {
                                    self.cmdix += 1;
                                }
                            }
                        },
                        EditorKey::ArrowUp => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cy != 1 {
                                    self.cy -= 1;
                                }
                                self.set_cx();
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::ArrowDown => match self.mode {
                            EditorMode::Normal | EditorMode::Insert => {
                                if self.cy != self.screenrows as usize - 1 {
                                    self.cy += 1;
                                }
                                self.set_cx();
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::DelKey => match self.mode {
                            EditorMode::Insert | EditorMode::Normal => {
                                // handles the case where the cursor is at the end of the line
                                // if mode is insert, the cursor is can be past the last character
                                // if mode is normal, the cursor is always on a character
                                if (self.cx - CX_INIT) == self.curr_right_limit() {
                                    self.rows[self.cy - 1].pop();
                                    self.cx -= 1;
                                } else {
                                    self.rows[self.cy - 1].remove(self.cx - CX_INIT);
                                }
                            }
                            EditorMode::Command => {}
                        },
                        EditorKey::HomeKey => {
                            self.cx = CX_INIT;
                        }
                        EditorKey::EndKey => {
                            self.cx = self.curr_right_limit() + CX_INIT;
                        }
                        EditorKey::PageUp => {
                            if self.cy > self.screenrows as usize {
                                self.cy -= self.screenrows as usize;
                            } else {
                                self.cy = 1;
                            }
                        }
                        EditorKey::PageDown => {
                            // TODO: support scrolling
                            if (self.cy + self.screenrows as usize) < self.rows.len() {
                                self.cy += self.screenrows as usize;
                            } else {
                                self.cy = self.rows.len();
                            }
                        }
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
                                        self.rows[self.cy - 1].insert(self.cx - CX_INIT, c as char);
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
    let contents = std::fs::read_to_string(file).unwrap();
    let mut editor = EditorConfig::new(&contents);
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
