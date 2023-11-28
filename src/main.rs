mod raw;

use std::process::exit;

use raw::*;
use rustix::{
    io::{self, Errno},
    stdio,
    termios::tcgetwinsize,
};

struct EditorConfig {
    cx: usize,
    cy: usize,
    screenrows: u16,
    screencols: u16,
}

#[derive(PartialEq)]
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

impl EditorConfig {
    fn new() -> Self {
        Self {
            cx: 0,
            cy: 0,
            screenrows: 0,
            screencols: 0,
        }
    }

    fn set_size(&mut self) {
        let winsize = tcgetwinsize(stdio::stdout());
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
        buf.push_str("\x1b[?25h");
        io::write(stdio::stdout(), buf.as_bytes()).unwrap();
    }

    fn get_cursor_position(&mut self) -> Result<(), Errno> {
        let mut buf = [0u8; 32];
        let mut i = 0;
        let mut rows = 0;
        let mut cols = 0;
        io::write(stdio::stdout(), b"\x1b[6n")?;
        let n = io::read(stdio::stdin(), &mut buf)?;
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

    fn read_key(&mut self) -> Result<u8, Errno> {
        let mut buf = [0u8; 1];
        io::read(stdio::stdin(), &mut buf)?;
        Ok(buf[0])
    }

    fn read_editor_key(&mut self) -> Result<EditorKey, Errno> {
        let c = self.read_key()?;
        match c {
            b'\x1b' => {
                let c = self.read_key()?;
                match c {
                    b'[' => {
                        let c = self.read_key()?;
                        match c {
                            b'D' => Ok(EditorKey::ArrowLeft),
                            b'C' => Ok(EditorKey::ArrowRight),
                            b'A' => Ok(EditorKey::ArrowUp),
                            b'B' => Ok(EditorKey::ArrowDown),
                            b'H' => Ok(EditorKey::HomeKey),
                            b'F' => Ok(EditorKey::EndKey),
                            b'1'..=b'8' => {
                                let c = self.read_key()?;
                                match c {
                                    b'~' => match c {
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
                                }
                            }
                            _ => Ok(EditorKey::K(c)),
                        }
                    }
                    b'O' => {
                        let c = self.read_key()?;
                        match c {
                            b'H' => Ok(EditorKey::HomeKey),
                            b'F' => Ok(EditorKey::EndKey),
                            _ => Ok(EditorKey::K(c)),
                        }
                    }
                    _ => Ok(EditorKey::K(c)),
                }
            }
            _ => Ok(EditorKey::K(c)),
        }
    }

    fn run(&mut self) -> Result<(), Errno> {
        loop {
            clear_screen();
            self.get_cursor_position()?;
            self.refresh_screen();
            match self.read_editor_key() {
                Ok(EditorKey::ArrowLeft) => {
                    if self.cx != 1 {
                        self.cx -= 1;
                    }
                }
                Ok(EditorKey::ArrowRight) => {
                    if self.cx != self.screencols as usize {
                        self.cx += 1;
                    }
                }
                Ok(EditorKey::ArrowUp) => {
                    if self.cy != 1 {
                        self.cy -= 1;
                    }
                }
                Ok(EditorKey::ArrowDown) => {
                    if self.cy != self.screenrows as usize {
                        self.cy += 1;
                    }
                }
                Ok(EditorKey::DelKey) => {}
                Ok(EditorKey::HomeKey) => {
                    self.cx = 1;
                }
                Ok(EditorKey::EndKey) => {
                    self.cx = self.screencols as usize;
                }
                Ok(EditorKey::PageUp) => {}
                Ok(EditorKey::PageDown) => {}
                Ok(EditorKey::K(_)) => {}
                Err(e) => {
                    return Err(e);
                }
            }
        }
    }
}

fn main() {
    let old_termios = match enable_raw_mode() {
        Ok(t) => t,
        Err(e) => {
            println!("error: {:?}", e);
            exit(1);
        }
    };
    let mut editor = EditorConfig::new();
    match editor.run() {
        Ok(_) => {}
        Err(e) => {
            println!("error: {:?}", e);
        }
    }
    disable_raw_mode(&old_termios);
}
