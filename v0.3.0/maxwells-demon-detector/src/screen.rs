//! Frame buffer + flicker-free terminal presenter.
//!
//! Frames are grids of ASCII cells. `Screen::present` diffs the new frame against
//! the previous one and rewrites only the cells that changed, addressing them with
//! cursor moves (never newlines, so raw mode can't staircase and nothing can wrap),
//! wrapped in a synchronized-update block so terminals that support it swap the
//! whole frame at once. The bottom-right cell is never written (writing it makes
//! some terminals scroll).

use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{
    Attribute, Color, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Default,
    Dim,
    Grey,
    White,
    Cyan,
    Green,
    Blue,
    Magenta,
    Yellow,
    Red,
    Title,
}

impl Tone {
    fn fg(self) -> Option<Color> {
        match self {
            Tone::Default => None,
            Tone::Dim => Some(Color::DarkGrey),
            Tone::Grey => Some(Color::Grey),
            Tone::White | Tone::Title => Some(Color::White),
            Tone::Cyan => Some(Color::Cyan),
            Tone::Green => Some(Color::Green),
            Tone::Blue => Some(Color::Blue),
            Tone::Magenta => Some(Color::Magenta),
            Tone::Yellow => Some(Color::Yellow),
            Tone::Red => Some(Color::Red),
        }
    }

    fn bg(self) -> Option<Color> {
        match self {
            Tone::Title => Some(Color::DarkBlue),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Tone,
    /// Only `Tone::Title` (or `Default`) is meaningful as a background.
    pub bg: Tone,
    pub bold: bool,
}

impl Style {
    pub const PLAIN: Style = Style {
        fg: Tone::Default,
        bg: Tone::Default,
        bold: false,
    };

    pub const fn fg(fg: Tone) -> Style {
        Style {
            fg,
            bg: Tone::Default,
            bold: false,
        }
    }

    pub const fn bold(fg: Tone) -> Style {
        Style {
            fg,
            bg: Tone::Default,
            bold: true,
        }
    }

    pub const fn title(bold: bool) -> Style {
        Style {
            fg: Tone::Title,
            bg: Tone::Title,
            bold,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: u8,
    pub style: Style,
}

const BLANK: Cell = Cell {
    ch: b' ',
    style: Style::PLAIN,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub w: u16,
    pub h: u16,
    cells: Vec<Cell>,
}

impl Frame {
    pub fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![BLANK; usize::from(w) * usize::from(h)],
        }
    }

    fn idx(&self, x: u16, y: u16) -> usize {
        usize::from(y) * usize::from(self.w) + usize::from(x)
    }

    pub fn cell(&self, x: u16, y: u16) -> Cell {
        if x < self.w && y < self.h {
            self.cells[self.idx(x, y)]
        } else {
            BLANK
        }
    }

    /// Set one cell. Out-of-range writes and the bottom-right cell are ignored;
    /// anything outside printable ASCII becomes '?'.
    pub fn put(&mut self, x: u16, y: u16, ch: u8, style: Style) {
        if x >= self.w || y >= self.h || (x == self.w - 1 && y == self.h - 1) {
            return;
        }
        let ch = if (0x20..0x7F).contains(&ch) { ch } else { b'?' };
        let i = self.idx(x, y);
        self.cells[i] = Cell { ch, style };
    }

    /// Write text starting at (x, y), at most `max` columns. Returns columns used.
    pub fn text(&mut self, x: u16, y: u16, s: &str, style: Style, max: u16) -> u16 {
        let mut used = 0u16;
        for ch in s.chars() {
            if used >= max || x.saturating_add(used) >= self.w {
                break;
            }
            let b = if ch.is_ascii() { ch as u8 } else { b'?' };
            self.put(x + used, y, b, style);
            used += 1;
        }
        used
    }

    /// Fill a whole row with spaces in `style` (e.g. the title bar background).
    pub fn fill_row(&mut self, y: u16, style: Style) {
        for x in 0..self.w {
            self.put(x, y, b' ', style);
        }
    }

    /// Plain text of one row (no styling).
    pub fn row_text(&self, y: u16) -> String {
        (0..self.w).map(|x| self.cell(x, y).ch as char).collect()
    }

    /// Whole frame as plain text, trailing spaces trimmed per row.
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(usize::from(self.w + 1) * usize::from(self.h));
        for y in 0..self.h {
            out.push_str(self.row_text(y).trim_end());
            out.push('\n');
        }
        out
    }
}

/// Presents frames to a terminal, rewriting only what changed.
pub struct Screen {
    prev: Option<Frame>,
    pub color: bool,
    buf: Vec<u8>,
}

impl Screen {
    pub fn new(color: bool) -> Self {
        Self {
            prev: None,
            color,
            buf: Vec::with_capacity(64 * 1024),
        }
    }

    /// Force a full redraw on the next `present` (after resize, colour toggle, ...).
    pub fn invalidate(&mut self) {
        self.prev = None;
    }

    pub fn present<W: Write>(&mut self, out: &mut W, frame: &Frame) -> io::Result<()> {
        let full = self
            .prev
            .as_ref()
            .is_none_or(|p| p.w != frame.w || p.h != frame.h);
        let mut buf = std::mem::take(&mut self.buf);
        buf.clear();
        queue!(buf, BeginSynchronizedUpdate)?;
        if full {
            queue!(
                buf,
                SetAttribute(Attribute::Reset),
                ResetColor,
                Clear(ClearType::All)
            )?;
        }
        let prev = if full { None } else { self.prev.as_ref() };
        let changed = |x: u16, y: u16| -> bool {
            match prev {
                Some(p) => p.cell(x, y) != frame.cell(x, y),
                None => frame.cell(x, y) != BLANK,
            }
        };
        let mut current: Option<Style> = None;
        for y in 0..frame.h {
            let row_end = if y == frame.h - 1 {
                frame.w.saturating_sub(1)
            } else {
                frame.w
            };
            let mut x = 0u16;
            while x < row_end {
                if !changed(x, y) {
                    x += 1;
                    continue;
                }
                // Extend the run across small unchanged gaps (cheaper than a cursor move).
                let mut end = x + 1;
                let mut look = x + 1;
                while look < row_end {
                    if changed(look, y) {
                        end = look + 1;
                    } else if look - end >= 4 {
                        break;
                    }
                    look += 1;
                }
                queue!(buf, MoveTo(x, y))?;
                for cx in x..end {
                    let cell = frame.cell(cx, y);
                    if current != Some(cell.style) {
                        apply_style(&mut buf, cell.style, self.color)?;
                        current = Some(cell.style);
                    }
                    buf.push(cell.ch);
                }
                x = end;
            }
        }
        queue!(
            buf,
            SetAttribute(Attribute::Reset),
            ResetColor,
            EndSynchronizedUpdate
        )?;
        out.write_all(&buf)?;
        out.flush()?;
        self.buf = buf;
        self.prev = Some(frame.clone());
        Ok(())
    }
}

fn apply_style(buf: &mut Vec<u8>, s: Style, color: bool) -> io::Result<()> {
    queue!(buf, SetAttribute(Attribute::Reset))?;
    if s.bold {
        queue!(buf, SetAttribute(Attribute::Bold))?;
    }
    if color {
        if let Some(fg) = s.fg.fg() {
            queue!(buf, SetForegroundColor(fg))?;
        }
        if let Some(bg) = s.bg.bg() {
            queue!(buf, SetBackgroundColor(bg))?;
        }
    } else if s.bg == Tone::Title {
        queue!(buf, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_guards_edges_and_non_ascii() {
        let mut f = Frame::new(4, 2);
        f.put(3, 1, b'X', Style::PLAIN); // bottom-right: ignored
        f.put(9, 9, b'X', Style::PLAIN); // out of range: ignored
        f.put(0, 0, 0xE9, Style::PLAIN); // non-ASCII -> '?'
        assert_eq!(f.cell(3, 1).ch, b' ');
        assert_eq!(f.cell(0, 0).ch, b'?');
        assert_eq!(f.text(1, 0, "héllo", Style::PLAIN, 10), 3);
        assert_eq!(f.row_text(0), "?h?l");
    }

    #[test]
    fn diff_writes_only_changes_and_no_newlines() {
        let mut screen = Screen::new(true);
        let mut a = Frame::new(40, 5);
        a.text(0, 0, "hello world", Style::bold(Tone::Green), 40);
        let mut out = Vec::new();
        screen.present(&mut out, &a).unwrap();
        assert!(!out.contains(&b'\n'), "cursor addressing only");
        let first = out.len();

        let mut b = a.clone();
        b.put(20, 3, b'@', Style::bold(Tone::Red));
        let mut out2 = Vec::new();
        screen.present(&mut out2, &b).unwrap();
        assert!(
            out2.len() < first,
            "diff should be smaller than a full frame"
        );
        assert!(out2.windows(1).any(|w| w == b"@"));
        assert!(!String::from_utf8_lossy(&out2).contains("hello"));

        // Unchanged frame: just the sync/reset wrapper.
        let mut out3 = Vec::new();
        screen.present(&mut out3, &b).unwrap();
        assert!(out3.len() < 40, "no-op present wrote {} bytes", out3.len());
    }
}
