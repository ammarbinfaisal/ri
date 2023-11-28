mod raw;

use std::{fs::File, io::Write, process::exit};

use raw::*;
use rustix::{
    fd::BorrowedFd,
    io::{self, Errno},
    stdio,
    termios::tcgetwinsize,
};

#[derive(Debug)]
struct EditorConfig<'a> {
    cx: usize,
    cy: usize,
    screenrows: u16,
    screencols: u16,
    stdout: BorrowedFd<'a>
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

impl<'editor> EditorConfig<'editor> {
    fn new() -> Self {
        Self {
            cx: 0,
            cy: 0,
            screenrows: 0,
            screencols: 0,
            stdout: stdio::stdout(),
        }
    }

    fn set_size(&mut self) {
        let winsize = tcgetwinsize(self.stdout);
        if let Ok(winsize) = winsize {
            self.screenrows = winsize.ws_row;
            self.screencols = winsize.ws_col;
        }
    }

    fn refresh_screen(&mut self) {
        self.set_size();
        let mut buf = String::new();
        buf.push_str("\x1b[?25l");
        buf.push_str("\x1b[H");
        for i in 0..self.screenrows {
            buf.push_str("~");
            buf.push_str("\x1b[K");
            if i < self.screenrows - 1 {
                buf.push_str("\r\n");
            }
        }
        buf.push_str("\x1b[H");
        buf.push_str("\x1b[?25h");
        // put cursor
        buf.push_str(&format!("\x1b[{};{}H", self.cy, self.cx));
        io::write(self.stdout, buf.as_bytes()).unwrap();
    }

    fn get_cursor_position<'a>(&mut self, fd: BorrowedFd<'a>) -> Result<(), Errno> {
        let mut buf = [0u8; 32];
        let mut i = 0;
        let mut rows = 0;
        let mut cols = 0;
        io::write(self.stdout, b"\x1b[6n")?;
        let n = io::read(fd, &mut buf)?;
        while i < n {
            if buf[i] == b'R' {
                break;
            }
            i += 1;
        }
        if i == n {
            return Err(Errno::IO);
        }
        i += 1;
        while i < n {
            if buf[i].is_ascii_digit() {
                rows = rows * 10 + (buf[i] - b'0') as usize;
            } else {
                break;
            }
            i += 1;
        }
        while i < n {
            if buf[i] == b';' {
                break;
            }
            i += 1;
        }
        i += 1;
        while i < n {
            if buf[i].is_ascii_digit() {
                cols = cols * 10 + (buf[i] - b'0') as usize;
            } else {
                break;
            }
            i += 1;
        }
        self.cx = cols;
        self.cy = rows;
        Ok(())
    }

    fn read_key<'a>(&mut self, fd: BorrowedFd<'a>) -> Result<u8, Errno> {
        let mut buf = [0u8; 1];
        io::read(fd, &mut buf)?;
        Ok(buf[0])
    }

    fn read_editor_key<'a>(&mut self, fd: BorrowedFd<'a>) -> Result<EditorKey, Errno> {
        let c = self.read_key(fd)?;
        match c {
            b'\x1b' => {
                let mut buf = [0u8; 3];
                io::read(fd, &mut buf)?;
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

    fn run<'a>(&mut self, fd: BorrowedFd<'a>) -> Result<(), Errno> {
        // open a log file
        let mut file = File::create("log").unwrap();
        loop {
            self.get_cursor_position(fd)?;
            self.refresh_screen();
            match self.read_editor_key(fd) {
                Ok(key) => {
                    file.write_all(&format!("{:?}\n", key).as_bytes()).unwrap();
                    file.flush().unwrap();
                    match key {
                        EditorKey::ArrowLeft => {
                            if self.cx != 0 {
                                self.cx -= 1;
                            }
                        }
                        EditorKey::ArrowRight => {
                            if self.cx != self.screencols as usize - 1 {
                                self.cx += 1;
                            }
                        }
                        EditorKey::ArrowUp => {
                            if self.cy != 0 {
                                self.cy -= 1;
                            }
                        }
                        EditorKey::ArrowDown => {
                            if self.cy != self.screenrows as usize - 1 {
                                self.cy += 1;
                            }
                        }
                        EditorKey::DelKey => {}
                        EditorKey::HomeKey => {
                            self.cx = 0;
                        }
                        EditorKey::EndKey => {
                            self.cx = self.screencols as usize - 1;
                        }
                        EditorKey::PageUp => {}
                        EditorKey::PageDown => {}
                        EditorKey::K(c) => {
                            if c == b'q' {
                                clear_screen();
                                return Ok(());
                            }
                        }
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
    let (old_termios, fd) = match enable_raw_mode() {
        Ok(t) => t,
        Err(e) => {
            println!("error: {:?}", e);
            exit(1);
        }
    };
    let mut editor = EditorConfig::new();
    match editor.run(fd) {
        Ok(_) => {}
        Err(e) => {
            println!("error: {:?}", e);
        }
    }
    disable_raw_mode(&old_termios, fd);
}
