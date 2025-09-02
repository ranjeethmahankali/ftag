use crate::{
    interactive::{InteractiveSession, State},
    query::TagTable,
};
use crossterm::{
    ExecutableCommand,
    cursor::{MoveDown, MoveRight, MoveTo, MoveToColumn, MoveToNextLine},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{
        Clear, ClearType, DisableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
        disable_raw_mode, enable_raw_mode,
    },
};
use std::io::Write as IOWrite;

/// Count digits in the integer as written in base 10.
fn count_digits(mut num: usize) -> u8 {
    if num == 0 {
        return 1;
    }
    let mut digits = 0u8;
    while num > 0 {
        digits += 1;
        num /= 10;
    }
    digits
}

/// Given `prev` and `curr`, this function removes the common prefix
/// from `curr` and returns the resulting string as part of a
/// tuple. The first element of the tuple is the length of the prefix
/// that was trimmed.
fn remove_common_prefix<'a>(prev: &str, curr: &'a str) -> (usize, &'a str) {
    let start =
        match prev
            .chars()
            .zip(curr.chars())
            .enumerate()
            .try_fold(0usize, |start, (i, (l, r))| match (l, r) {
                (std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR) => Ok(i),
                (l, r) if l == r => Ok(start),
                _ => Err(start),
            }) {
            Ok(start) => start,
            Err(start) => start,
        };
    (start, &curr[start..])
}

#[inline(always)]
fn truncate_string(input: &str, len: u16) -> &str {
    &input[..((len as usize).min(input.len()))]
}

#[derive(Debug)]
pub enum TuiError {
    IO(std::io::Error),
    Fmt(std::fmt::Error),
}

impl From<std::io::Error> for TuiError {
    fn from(value: std::io::Error) -> Self {
        TuiError::IO(value)
    }
}

impl From<std::fmt::Error> for TuiError {
    fn from(value: std::fmt::Error) -> Self {
        TuiError::Fmt(value)
    }
}

struct TuiApp {
    session: InteractiveSession,
    tag_scroll: usize,
    tag_max_scroll: usize,
    frame_height: u16,
    frame_width: u16,
    page_index: usize,
    last_page: usize,
    files_per_page: usize,
    file_index_width: u8,
    screen_buf: Vec<u8>,
    hline: String,
    left_width: u16,
    right_width: u16,
    max_tag_len: u16,
}

impl TuiApp {
    fn init(table: TagTable) -> Self {
        let ntags = table.tags().len();
        let nfiles = table.files().len();
        let max_tag_len = table.tags().iter().map(|t| t.len()).max().unwrap_or(0) as u16;
        TuiApp {
            session: InteractiveSession::init(table),
            file_index_width: count_digits(nfiles - 1),
            tag_max_scroll: ntags,
            tag_scroll: 0,
            frame_height: 0,
            frame_width: 0,
            page_index: 0,
            last_page: 0,
            files_per_page: 0,
            screen_buf: Default::default(),
            hline: String::default(),
            left_width: 0,
            right_width: 0,
            max_tag_len,
        }
    }

    fn can_scroll(&self) -> bool {
        self.session.taglist().len() + 1 > (self.frame_height as usize)
    }

    fn set_frame_size(&mut self, ncols: u16, nrows: u16) {
        if ncols != self.frame_width {
            // Deal with columns and panel widths.
            self.frame_width = ncols;
            self.left_width = (ncols.saturating_sub(1) / 5).min(self.max_tag_len + 3);
            self.right_width = ncols.saturating_sub(1).saturating_sub(self.left_width);
            self.hline = format!(
                "{line:─<w$.w$}",
                line = "├",
                w = (self.right_width as usize) + 1
            );
        }
        if nrows != self.frame_height {
            // Deal with rows and scrolling etc.
            self.frame_height = nrows;
            let end = self.tag_scroll + (nrows as usize);
            let ntags = self.session.taglist().len();
            if end > ntags {
                self.tag_scroll = self.tag_scroll.saturating_sub(end - ntags);
            }
            self.tag_max_scroll = ntags.saturating_sub(nrows as usize);
            // 1 for header, 1 for border at the top. 2 borders at the bottom, one
            // for the REPL line, two for the echo area. In total 7 lines.
            let old_fpp =
                std::mem::replace(&mut self.files_per_page, nrows.saturating_sub(7) as usize);
            (self.last_page, self.page_index) = if self.files_per_page == 0 {
                (0, 0)
            } else {
                (
                    self.session.filelist().len() / self.files_per_page,
                    (self.page_index * old_fpp) / self.files_per_page,
                )
            };
        }
    }

    fn keyevent(&mut self, evt: KeyEvent) {
        match evt.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => match evt.code {
                KeyCode::Char(c) => {
                    if evt.modifiers.contains(KeyModifiers::CONTROL) {
                        match c {
                            'q' | 'Q' => self.session.set_state(State::Exit),
                            'n' | 'N' => {
                                self.page_index =
                                    self.page_index.saturating_add(1).min(self.last_page)
                            }
                            'p' | 'P' => self.page_index = self.page_index.saturating_sub(1),
                            _ => {}
                        }
                    } else {
                        self.session.command_mut().push(c);
                        self.session.stop_autocomplete();
                    }
                }
                KeyCode::Backspace => {
                    let cmd = self.session.command_mut();
                    if evt.modifiers.contains(KeyModifiers::ALT)
                        || evt.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        while let Some(c) = cmd.pop() {
                            if c.is_whitespace() {
                                break;
                            }
                        }
                    } else {
                        cmd.pop();
                    }
                    self.session.stop_autocomplete();
                }
                KeyCode::Enter => {
                    self.session.process_input();
                    if let State::ListsUpdated = self.session.state() {
                        self.tag_scroll = 0;
                        self.session.set_state(State::Default);
                    }
                }
                KeyCode::Esc => {
                    self.session.command_mut().clear();
                    self.session.stop_autocomplete();
                }
                KeyCode::Up if self.can_scroll() => {
                    self.tag_scroll = self.tag_scroll.saturating_sub(1);
                }
                KeyCode::Down if self.can_scroll() => {
                    self.tag_scroll = self.tag_scroll.saturating_add(1).min(self.tag_max_scroll);
                }
                KeyCode::Tab => self.session.autocomplete(),
                _ => {}
            },
            KeyEventKind::Release => {} // Do nothing.
        }
    }

    fn render(&mut self, stdout: &mut std::io::Stdout) -> Result<(), TuiError> {
        let time_before = std::time::Instant::now();

        let (ncols, nrows) = crossterm::terminal::size()?;
        self.set_frame_size(ncols, nrows);
        let (lwidth, rwidth) = (self.left_width, self.right_width);
        self.screen_buf.clear();
        self.screen_buf
            .reserve((ncols as usize) * (nrows as usize) * 2);
        // Clearn the screen and start rendering.
        queue!(
            self.screen_buf,
            Clear(ClearType::All),
            MoveTo(0, 0),
            DisableLineWrap
        )?;
        // Render all tags.
        for tag in self
            .session
            .taglist()
            .iter()
            .map(|t| t.as_str())
            .chain(std::iter::repeat(""))
            .skip(self.tag_scroll)
            .take(nrows as usize)
        {
            queue!(
                self.screen_buf,
                MoveRight(1),
                Print(format_args!("{}", truncate_string(tag, lwidth - 1))),
                MoveToNextLine(1)
            )?;
        }
        // Render the header.
        queue!(
            self.screen_buf,
            MoveTo(lwidth as u16, 0),
            SetAttribute(Attribute::Bold),
            Print(format_args!(
                "│{header:^w$.w$}",
                w = rwidth as usize,
                // Must use format instead of format_args, for the center alignment to work.
                header = format!(
                    "{}: {} results, page {} of {}",
                    if self.session.filter_str().is_empty() {
                        "ALL_TAGS"
                    } else {
                        self.session.filter_str()
                    },
                    self.session.filelist().len(),
                    self.page_index + 1, // Don't show the user zero based index.
                    self.last_page + 1,
                )
            )),
            SetAttribute(Attribute::Reset)
        )?;
        // Render border below header.
        queue!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(&self.hline)
        )?;
        // Render filepaths.
        self.session
            .filelist()
            .iter()
            .map(|f| f.as_str())
            .chain(std::iter::repeat(""))
            .enumerate()
            .skip(self.files_per_page * self.page_index)
            .take(self.files_per_page)
            .try_fold("", |prevfile, (i, file)| {
                if file.is_empty() {
                    queue!(
                        self.screen_buf,
                        MoveToColumn(lwidth as u16),
                        MoveDown(1),
                        Print("│")
                    )?;
                } else {
                    let (padding, trimmed) = remove_common_prefix(prevfile, file);
                    queue!(
                        self.screen_buf,
                        MoveToColumn(lwidth as u16),
                        MoveDown(1),
                        Print(format_args!(
                            "│ [{i:>iw$}] {pp:.<padding$}{trimmed}",
                            iw = self.file_index_width as usize,
                            pp = "",
                            trimmed = truncate_string(
                                trimmed,
                                rwidth
                                    .saturating_sub(4)
                                    .saturating_sub(self.file_index_width as u16)
                            )
                        ))
                    )?;
                }
                Result::<&str, std::io::Error>::Ok(file)
            })?;
        // Render border.
        queue!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(&self.hline)
        )?;
        // Render two lines in the echo area one line at a time.
        for line in self
            .session
            .echo()
            .lines()
            .chain(std::iter::repeat(""))
            .take(2)
        {
            queue!(
                self.screen_buf,
                MoveToColumn(lwidth as u16),
                MoveDown(1),
                Print(format_args!("│{}", truncate_string(line, rwidth)))
            )?;
        }
        // Render border.
        queue!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(&self.hline)
        )?;
        // Render the REPL line. We render the echo string last, because that
        // way we don't have to hide the cursor and render a cursor unicode
        // character artificially. The cursor will naturally land at the end of
        // the echo string.
        queue!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(format_args!(
                "│>>> {}",
                truncate_string(self.session.command(), rwidth.saturating_sub(4))
            ))
        )?;
        // Write the screen buffer out to the terminal in a single sys call.
        stdout.write_all(&self.screen_buf)?;
        stdout.flush()?;

        self.session.set_echo(&format!(
            "Last frame took: {:?}",
            std::time::Instant::now() - time_before
        ));

        Ok(())
    }
}

/// Start the interactive TUI mode of ftag.
pub fn run(table: TagTable) -> Result<(), TuiError> {
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut app = TuiApp::init(table);
    // Main application loop. The terminal is only redrawn when an
    // event is registered, so it is necessary to draw it once at
    // first.
    app.render(&mut stdout)?;
    loop {
        match event::read()? {
            Event::Key(key) => {
                app.keyevent(key);
                app.render(&mut stdout)?;
            }
            Event::Resize(..) => app.render(&mut stdout)?,
            _ => {} // Do nothing.
        };
        if let State::Exit = app.session.state() {
            break;
        };
    }
    // Clean up.
    stdout.execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
