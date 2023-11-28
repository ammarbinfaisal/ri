use std::ops::IndexMut;
use rustix::{io::Errno, stdio, termios::*, fd::BorrowedFd};

pub fn enable_raw_mode<'a>() -> Result<(Termios, BorrowedFd<'a>), Errno> {
    let fd = stdio::stdin();
    let orig_termios = tcgetattr(fd)?;
    let mut raw = orig_termios.clone();
    raw.input_modes &= !(InputModes::BRKINT
        | InputModes::ICRNL
        | InputModes::INPCK
        | InputModes::ISTRIP
        | InputModes::IXON);
    raw.output_modes &= !(OutputModes::OPOST);
    raw.control_modes |= ControlModes::CS8;
    raw.local_modes &=
        !(LocalModes::ECHO | LocalModes::ICANON | LocalModes::IEXTEN | LocalModes::ISIG);
    *raw.special_codes.index_mut(SpecialCodeIndex::VMIN) = 0;
    *raw.special_codes.index_mut(SpecialCodeIndex::VTIME) = 1;
    tcsetattr(fd, OptionalActions::Flush, &raw)?;
    Ok((orig_termios, fd))
}

pub fn disable_raw_mode<'a>(old_termios: &Termios, fd: BorrowedFd<'a>) {
    if let Ok(_) = tcsetattr(fd, OptionalActions::Flush, &old_termios) {
        println!("bye!");
    }
}

pub fn clear_screen() {
    print!("\x1b[2J");
}
