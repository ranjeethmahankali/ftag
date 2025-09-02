use crate::{
    interactive::{InteractiveSession, State},
    query::TagTable,
};
use crossterm::{
    ExecutableCommand,
    cursor::MoveTo,
    event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    style::Print,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use std::fmt::Write as FmtWrite;

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
    let stop = usize::min(prev.len(), curr.len());
    let mut start = 0usize;
    for (i, (l, r)) in prev.chars().zip(curr.chars()).enumerate() {
        if !(i < stop && l == r) {
            break;
        }
        if l == std::path::MAIN_SEPARATOR {
            start = i;
        }
    }
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
    num_pages: usize,
    files_per_page: usize,
    file_index_width: u8,
    screen_buf: String,
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
            num_pages: 0,
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
        // 2 rows for the header, 4 rows for the footer -> total 7 rows.
        let old_files_per_page = self.files_per_page;
        self.files_per_page = h.saturating_sub(6);
        (self.num_pages, self.page_index) = if self.files_per_page == 0 {
            (0, 0)
        } else {
            (
                self.session.filelist().len() / self.files_per_page,
                (self.page_index * old_files_per_page) / self.files_per_page,
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
                                    self.page_index.saturating_add(1).min(self.num_pages)
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
        self.screen_buf.reserve((ncols as usize) * (nrows as usize));
        let lwidth = ((ncols - 1) / 5) as usize;
        let rwidth = (ncols as usize) - 1 - lwidth;
        let mut tags = self
            .session
            .taglist()
            .iter()
            .skip(self.scroll)
            .take(nrows as usize);

        // Render first line with the top bar heading.
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}│")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}│", "")?,
        }
        write!(
            self.screen_buf,
            "{:^rwidth$.rwidth$}",
            format!(
                "{}: {} results, page {} of {}",
                if self.session.filter_str().is_empty() {
                    "ALL_TAGS"
                } else {
                    self.session.filter_str()
                },
                self.session.filelist().len(),
                self.page_index + 1,
                self.num_pages
            )
        )?;
        // Render second line with the border.
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}├")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}├", "")?,
        }
        write!(self.screen_buf, "{:─<rwidth$.rwidth$}", "")?;
        // Render the lines corresponding to file paths.
        let mut files = self
            .session
            .filelist()
            .iter()
            .enumerate()
            .skip(self.files_per_page * self.page_index)
            .take(self.files_per_page);
        let mut prevfile: &str = "";
        for _ in 0..self.files_per_page {
            match tags.next() {
                Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}│")?,
                None => write!(self.screen_buf, "{:<lwidth$.lwidth$}│", "")?,
            }
            match files.next() {
                Some((i, file)) => {
                    let (padding, trimmed) =
                        remove_common_prefix(std::mem::replace(&mut prevfile, file), file);
                    write!(
                        self.screen_buf,
                        "{:<rwidth$.rwidth$}",
                        format!(
                            " [{i:>iw$}] {pp:.<padding$}{trimmed}",
                            iw = self.file_index_width as usize,
                            pp = ""
                        )
                    )?;
                }
                None => self.screen_buf.extend(std::iter::repeat(' ').take(rwidth)),
            }
        }
        // Render the echo area.
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}├")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}├", "")?,
        }
        write!(self.screen_buf, "{:─<rwidth$.rwidth$}", "")?;
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}│")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}│", "")?,
        }
        write!(self.screen_buf, "{:<rwidth$.rwidth$}", self.session.echo())?;
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}├")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}├", "")?,
        }
        write!(self.screen_buf, "{:─<rwidth$.rwidth$}", "")?;
        match tags.next() {
            Some(tag) => write!(self.screen_buf, "{tag:<lwidth$.lwidth$}│")?,
            None => write!(self.screen_buf, "{:<lwidth$.lwidth$}│", "")?,
        }
        write!(
            self.screen_buf,
            "{:<.rwidth$}",
            format!(">>> {}", self.session.command())
        )?;
        // Write the screen buffer out to the terminal in a single sys call.
        execute!(
            stdout,
            Clear(ClearType::All),
            MoveTo(0, 0),
            Print(&self.screen_buf)
        )?;
        Ok(())
    }
}

/// Start the interactive TUI mode of ftag.
pub fn start(table: TagTable) -> Result<(), TuiError> {
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut app = TuiApp::init(table);
    run_app(&mut stdout, &mut app)?;
    // Clean up.
    stdout.execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn run_app(stdout: &mut std::io::Stdout, app: &mut TuiApp) -> Result<(), TuiError> {
    const DELAY: u64 = 20;
    // Main application loop. The terminal is only redrawn when an
    // event is registered, so it is necessary to draw it once at
    // first.
    app.render(stdout)?;
    loop {
        // Poll events to see if redraw needed.
        if event::poll(std::time::Duration::from_millis(DELAY))? {
            if let event::Event::Key(key) = crossterm::event::read()? {
                app.keyevent(key);
            }
            app.render(stdout)?;
        }
        if let State::Exit = app.session.state() {
            break;
        };
    }
    Ok(())
}
