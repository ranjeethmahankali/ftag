use crate::{
    interactive::{InteractiveSession, State},
    query::TagTable,
};
use crossterm::{
    ExecutableCommand,
    cursor::{MoveDown, MoveRight, MoveTo, MoveToColumn, MoveToNextLine},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    style::Print,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
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
    scroll: usize,
    max_scroll: usize,
    frameheight: usize,
    page_index: usize,
    last_page: usize,
    files_per_page: usize,
    file_index_width: u8,
    screen_buf: Vec<u8>,
}

impl TuiApp {
    fn init(table: TagTable) -> Self {
        let ntags = table.tags().len();
        let nfiles = table.files().len();
        TuiApp {
            session: InteractiveSession::init(table),
            scroll: 0,
            max_scroll: ntags,
            frameheight: 0,
            page_index: 0,
            last_page: 0,
            files_per_page: 0,
            file_index_width: count_digits(nfiles - 1),
            screen_buf: Default::default(),
        }
    }

    fn can_scroll(&self) -> bool {
        self.session.taglist().len() + 1 > self.frameheight
    }

    fn set_frame_height(&mut self, h: usize) {
        self.frameheight = h;
        let end = self.scroll + h;
        let ntags = self.session.taglist().len();
        if end > ntags {
            self.scroll = self.scroll.saturating_sub(end - ntags);
        }
        self.max_scroll = ntags.saturating_sub(h);
        // 1 for header, 1 for border at the top. 2 borders at the bottom, one
        // for the REPL line, two for the echo area. In total 7 lines.
        let old_fpp = std::mem::replace(&mut self.files_per_page, h.saturating_sub(7));
        (self.last_page, self.page_index) = if self.files_per_page == 0 {
            (0, 0)
        } else {
            (
                self.session.filelist().len() / self.files_per_page,
                (self.page_index * old_fpp) / self.files_per_page,
            )
        };
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
                        self.scroll = 0;
                        self.session.set_state(State::Default);
                    }
                }
                KeyCode::Esc => {
                    self.session.command_mut().clear();
                    self.session.stop_autocomplete();
                }
                KeyCode::Up if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_sub(1);
                }
                KeyCode::Down if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_add(1).min(self.max_scroll);
                }
                KeyCode::Tab => self.session.autocomplete(),
                _ => {}
            },
            KeyEventKind::Release => {} // Do nothing.
        }
    }

    fn render(&mut self, stdout: &mut std::io::Stdout) -> Result<(), TuiError> {
        let (ncols, nrows) = crossterm::terminal::size()?;
        self.set_frame_height(nrows as usize);
        self.screen_buf.clear();
        self.screen_buf
            .reserve((ncols as usize) * (nrows as usize) * 2);
        let lwidth = ((ncols - 1) / 5) as usize;
        let rwidth = (ncols as usize) - 1 - lwidth;
        // Clearn the screen and start rendering.
        self.screen_buf.execute(Clear(ClearType::All))?;
        self.screen_buf.execute(MoveTo(0, 0))?;
        // Render all tags.
        for tag in self
            .session
            .taglist()
            .iter()
            .map(|t| t.as_str())
            .chain(std::iter::repeat(""))
            .skip(self.scroll)
            .take(nrows as usize)
        {
            execute!(
                self.screen_buf,
                MoveRight(1),
                Print(format_args!("{tag:<w$.w$}", w = lwidth - 1)),
                MoveToNextLine(1)
            )?;
        }
        // Render the header.
        execute!(
            self.screen_buf,
            MoveTo(lwidth as u16, 0),
            Print(format_args!(
                "│{:^rwidth$.rwidth$}",
                format_args!(
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
            ))
        )?;
        // Render border below header.
        execute!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(format_args!("├{:─<rwidth$.rwidth$}", ""))
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
                    execute!(
                        self.screen_buf,
                        MoveToColumn(lwidth as u16),
                        MoveDown(1),
                        Print(format_args!("│{:<rwidth$}", ""))
                    )?;
                } else {
                    let (padding, trimmed) = remove_common_prefix(prevfile, file);
                    execute!(
                        self.screen_buf,
                        MoveToColumn(lwidth as u16),
                        MoveDown(1),
                        Print(format_args!(
                            "│{:<rwidth$.rwidth$}",
                            format_args!(
                                " [{i:>iw$}] {pp:.<padding$}{trimmed}",
                                iw = self.file_index_width as usize,
                                pp = ""
                            )
                        ))
                    )?;
                }
                Result::<&str, std::io::Error>::Ok(file)
            })?;
        // Render border.
        execute!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(format_args!("├{:─<rwidth$.rwidth$}", ""))
        )?;
        // Render two lines in the echo area one line at a time.
        for line in self
            .session
            .echo()
            .lines()
            .chain(std::iter::repeat(""))
            .take(2)
        {
            execute!(
                self.screen_buf,
                MoveToColumn(lwidth as u16),
                MoveDown(1),
                Print(format_args!("│{:<rwidth$.rwidth$}", line))
            )?;
        }
        // Render border.
        execute!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(format_args!("├{:─<rwidth$.rwidth$}", ""))
        )?;
        // Render the REPL line. We render the echo string last, because that
        // way we don't have to hide the cursor and render a cursor unicode
        // character artificially. The cursor will naturally land at the end of
        // the echo string.
        execute!(
            self.screen_buf,
            MoveToColumn(lwidth as u16),
            MoveDown(1),
            Print(format_args!(
                "│{:<.rwidth$}",
                format_args!(">>> {}", self.session.command())
            ))
        )?;
        // Write the screen buffer out to the terminal in a single sys call.
        stdout.write_all(&self.screen_buf)?;
        stdout.flush()?;
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
