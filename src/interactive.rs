use crate::{
    core::what_is,
    filter::{Filter, FilterParseError},
    query::DenseTagTable,
};
use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{Backend, Constraint, CrosstermBackend, Direction, Layout, Terminal},
    text::{Line, Text},
    widgets::{
        Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::{fmt::Debug, io::stdout, path::PathBuf};

/// State of the app.
enum State {
    Default,
    Autocomplete,
    Exit,
}
use State::*;

enum Command {
    Exit,
    Quit,
    Reset,
    Filter(Filter<usize>),
    WhatIs(PathBuf),
    Open(PathBuf),
}

enum Error {
    InvalidCommand(String),
    InvalidFilter(FilterParseError),
}

impl Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCommand(message) => write!(f, "Invalid command: {}", message),
            Self::InvalidFilter(err) => write!(f, "Invalid filter: {err:?}"),
        }
    }
}

struct App {
    table: DenseTagTable,
    // State management.
    command: String,
    echo: String,
    state: State,
    tag_active: Vec<bool>,
    filtered_indices: Vec<usize>,
    filter_str: String,
    taglist: Vec<String>,
    filelist: Vec<String>,
    // UI management.
    scroll: usize,
    scrollstate: ScrollbarState,
    frameheight: usize,
    file_index_width: u8,
    // Autocomplete
    command_completions: Box<[String]>,
    suggestions: Vec<String>,
    suggestion_index: usize,
}

/// Given `prev` and `curr`, this function removes the common prefix
/// from `curr` and returns the resulting string as part of a
/// tuple. The first element of the tuple is the length of the prefix
/// that was trimmed.
fn remove_common_prefix<'a>(prev: &str, curr: &'a str) -> (usize, &'a str) {
    let stop = usize::min(prev.len(), curr.len());
    let mut iter = prev.chars().zip(curr.chars()).enumerate();
    let mut start = 0usize;
    while let Some((i, (l, r))) = iter.next() {
        if !(i < stop && l == r) {
            break;
        }
        if l == std::path::MAIN_SEPARATOR {
            start = i;
        }
    }
    return (start, &curr[start..]);
}

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
    return digits;
}

impl App {
    fn init(table: DenseTagTable) -> App {
        let taglist = table.tags().to_vec();
        let ntags = table.tags().len();
        let nfiles = table.files().len();
        let mut app = App {
            table,
            command: String::new(),
            echo: String::new(),
            state: Default,
            tag_active: vec![true; ntags],
            taglist,
            filelist: Vec::with_capacity(nfiles),
            scroll: 0,
            scrollstate: ScrollbarState::new(ntags),
            frameheight: 0,
            filtered_indices: (0..nfiles).collect(),
            filter_str: String::new(),
            file_index_width: count_digits(nfiles - 1),
            command_completions: ["exit", "quit", "reset", "whatis", "open"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            suggestions: Vec::new(),
            suggestion_index: 0,
        };
        App::update_file_list(
            &app.filtered_indices,
            &app.table.files(),
            app.file_index_width,
            &mut app.filelist,
        );
        return app;
    }

    fn reset(&mut self) {
        self.filter_str.clear();
        self.filtered_indices.clear();
        self.filtered_indices.extend(0..self.num_files());
        self.update_lists();
        self.echo.clear();
        self.state = Default;
        self.tag_active.fill(true);
        self.scroll = 0;
        self.scrollstate = ScrollbarState::new(self.table.tags().len());
    }

    fn parse_index_to_filepath(&self, numstr: &str) -> Result<PathBuf, Error> {
        let index = match numstr.parse::<usize>() {
            Ok(num) if num < self.filtered_indices.len() => Ok(num),
            Ok(num) => Err(Error::InvalidCommand(format!(
                "{num} is not a valid choice. Please choose an index between 0 and  {}",
                self.filtered_indices.len()
            ))),
            Err(_) => Err(Error::InvalidCommand(String::from(format!(
                "Unable to parse '{numstr}' to an index."
            )))),
        }?;
        let mut path = self.table.path().to_path_buf();
        path.push(&self.table.files()[self.filtered_indices[index]]);
        return Ok(path);
    }

    fn parse_command(&mut self) -> Result<Command, Error> {
        let cmd = self.command.trim();
        if let Some(cmd) = cmd.strip_prefix('/') {
            if cmd == "exit" {
                Ok(Command::Exit)
            } else if cmd == "quit" {
                Ok(Command::Quit)
            } else if cmd == "reset" {
                Ok(Command::Reset)
            } else if let Some(numstr) = cmd.strip_prefix("whatis ") {
                Ok(Command::WhatIs(self.parse_index_to_filepath(numstr)?))
            } else if let Some(numstr) = cmd.strip_prefix("open ") {
                Ok(Command::Open(self.parse_index_to_filepath(numstr)?))
            } else {
                Err(Error::InvalidCommand(cmd.to_string()))
            }
        } else {
            let newfilter = format!("{} {cmd}", self.filter_str);
            Ok(Command::Filter(
                Filter::<usize>::parse(&newfilter, &self.table)
                    .map_err(|e| Error::InvalidFilter(e))?,
            ))
        }
    }

    fn num_files(&self) -> usize {
        self.table.files().len()
    }

    fn update_file_list(
        indices: &[usize],
        files: &[String],
        index_width: u8,
        dst: &mut Vec<String>,
    ) {
        dst.clear();
        dst.reserve(indices.len());
        let mut filecounter = 0;
        let mut prevfile: &str = "";
        for file in indices.iter().map(|i| &files[*i]) {
            dst.push(format!(
                "[{}] {}",
                {
                    let nspaces = index_width - count_digits(filecounter);
                    format!("{}{filecounter}", " ".repeat(nspaces as usize))
                },
                {
                    let (space, trimmed) = remove_common_prefix(prevfile, file);
                    format!("{}{}", ".".repeat(space), trimmed)
                }
            ));
            prevfile = file;
            filecounter += 1;
        }
    }

    fn update_tag_list(
        indices: &[usize],
        tags: &[String],
        table: &DenseTagTable,
        active: &mut [bool],
        dst: &mut Vec<String>,
    ) {
        active.fill(false);
        for flags in indices.iter().map(|i| table.flags(*i)) {
            active
                .iter_mut()
                .zip(flags.iter())
                .for_each(|(dst, src)| *dst = *dst || *src);
        }
        dst.clear();
        dst.extend(tags.iter().zip(0..table.tags().len()).filter_map(|(t, i)| {
            if active[i] {
                Some(t.clone())
            } else {
                None
            }
        }));
    }

    fn update_lists(&mut self) {
        Self::update_file_list(
            &self.filtered_indices,
            self.table.files(),
            self.file_index_width,
            &mut self.filelist,
        );
        Self::update_tag_list(
            &self.filtered_indices,
            self.table.tags(),
            &self.table,
            &mut self.tag_active,
            &mut self.taglist,
        );
    }

    fn last_word_start(&self) -> usize {
        const DELIMS: &str = " ()&|!/";
        DELIMS
            .chars()
            .map(|ch| match self.command.rfind(ch) {
                Some(val) => val + 1,
                None => 0,
            })
            .max()
            .unwrap_or(0)
    }

    fn process_input(&mut self) {
        match self.state {
            Default => {
                match self.parse_command() {
                    Ok(cmd) => match cmd {
                        Command::Exit | Command::Quit => self.state = Exit,
                        Command::WhatIs(path) => {
                            self.echo = format!(
                                "{}",
                                what_is(&path).unwrap_or(String::from(
                                    "Unable to fetch the description of this file."
                                ))
                            );
                        }
                        Command::Filter(filter) => {
                            self.filtered_indices.clear();
                            self.filtered_indices.extend(
                                (0..self.num_files()).filter(|i| filter.eval(self.table.flags(*i))),
                            );
                            self.update_lists();
                            self.filter_str = filter.to_string(self.table.tags());
                            self.scroll = 0;
                            self.scrollstate = self.scrollstate.content_length(self.taglist.len());
                        }
                        Command::Reset => self.reset(),
                        Command::Open(path) => match opener::open(path) {
                            Ok(_) => {} // Do nothing.
                            Err(_) => self.echo = String::from("Unable to open the file."),
                        },
                    },
                    Err(e) => self.echo = format!("{:?}", e),
                }
                self.command.clear();
            }
            Autocomplete => {
                self.command.truncate(self.last_word_start());
                self.command
                    .push_str(&self.suggestions[self.suggestion_index]);
                self.state = Default;
                self.echo.clear();
            }
            Exit => {} // Do nothing.
        }
    }

    fn can_scroll(&self) -> bool {
        self.taglist.len() + 1 > self.frameheight
    }

    fn show_suggestions(&mut self) {
        self.echo.clear();
        for (i, suggestion) in self.suggestions.iter().enumerate() {
            if i == self.suggestion_index {
                self.echo.push_str(&format!("[{}]", suggestion));
            } else {
                self.echo.push_str(&format!(" {} ", suggestion));
            }
        }
    }

    fn autocomplete(&mut self) {
        let next_state = match self.state {
            Default => {
                self.suggestions.clear();
                let start = self.last_word_start();
                let word = &self.command[start..];
                if self.command.starts_with('/') {
                    // Complete commands.
                    self.suggestions
                        .extend(self.command_completions.iter().filter_map(|c| {
                            if c.starts_with(word) {
                                Some(c.to_string())
                            } else {
                                None
                            }
                        }));
                } else {
                    self.suggestions
                        .extend(self.table.tags().iter().filter_map(|t| {
                            if t.starts_with(word) {
                                Some(t.to_string())
                            } else {
                                None
                            }
                        }));
                }
                self.suggestion_index = 0;
                self.show_suggestions();
                Autocomplete
            }
            Autocomplete if !self.suggestions.is_empty() => {
                self.suggestion_index = (self.suggestion_index + 1) % self.suggestions.len();
                self.show_suggestions();
                Autocomplete
            }
            Autocomplete => Autocomplete,
            Exit => Exit, // Do nothing.
        };
        self.state = next_state;
    }

    fn stop_autocomplete(&mut self) {
        match &self.state {
            Default => {}
            // Do nothing.
            Autocomplete => {
                self.suggestions.clear();
                self.suggestion_index = 0;
                self.echo.clear();
                self.state = Default;
            }
            Exit => {} // Do nothing.
        }
    }

    fn keyevent(&mut self, evt: KeyEvent) {
        match evt.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => match evt.code {
                KeyCode::Char(c) => {
                    self.command.push(c);
                    self.stop_autocomplete();
                }
                KeyCode::Backspace => {
                    self.command.pop();
                    self.stop_autocomplete();
                }
                KeyCode::Enter => self.process_input(),
                KeyCode::Esc => {
                    self.command.clear();
                    self.stop_autocomplete();
                }
                KeyCode::Up if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_sub(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                KeyCode::Down if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_add(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                KeyCode::Tab => self.autocomplete(),
                _ => {}
            },
            KeyEventKind::Release => {} // Do nothing.
        }
    }
}

/// Start the interactive TUI mode of ftag.
pub(crate) fn start(table: DenseTagTable) -> std::io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let mut app = App::init(table);
    run_app(&mut terminal, &mut app)?;
    // Clean up.
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    return Ok(());
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> std::io::Result<()> {
    const DELAY: u64 = 20;
    // Main application loop. The terminal is only redrawn when an
    // event is registered, so it is necessary to draw it once at
    // first.
    terminal.draw(|f| render(f, app))?;
    loop {
        // Poll events to see if redraw needed.
        if event::poll(std::time::Duration::from_millis(DELAY))? {
            // If a key event occurs, handle it
            match crossterm::event::read()? {
                event::Event::Key(key) => {
                    app.keyevent(key);
                }
                _ => {} //  Do nothing.
            }
            terminal.draw(|f| render(f, app))?;
        }
        match app.state {
            Exit => break,
            _ => {} // Do nothing.
        };
    }
    return Ok(());
}

fn render(f: &mut Frame, app: &mut App) {
    const TAGWIDTH_PERCENT: u16 = 20;
    app.frameheight = f.size().height as usize;
    let hlayout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Percentage(TAGWIDTH_PERCENT),
            Constraint::Percentage(100 - TAGWIDTH_PERCENT),
        ])
        .split(f.size());
    let rblocks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Max(1001),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(hlayout[1]);
    let lblocks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Max(1000), Constraint::Length(6)])
        .split(hlayout[0]);
    let tagblock = lblocks[0];
    let filterblock = lblocks[1];
    let fileblock = rblocks[0];
    let echoblock = rblocks[1];
    let cmdblock = rblocks[2];
    // Tags.
    f.render_widget(
        Paragraph::new(
            app.taglist
                .iter()
                .map(|t| Line::from(t.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::new()
                .borders(Borders::TOP | Borders::RIGHT)
                .padding(Padding::horizontal(4)),
        )
        .scroll((app.scroll as u16, 0)),
        tagblock,
    );
    // Scroll bar.
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalLeft)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓")),
        tagblock,
        &mut app.scrollstate,
    );
    f.render_widget(
        Paragraph::new(
            app.filelist
                .iter()
                .map(|f| Line::from(f.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::new()
                .borders(Borders::TOP)
                .padding(Padding::horizontal(2)),
        ),
        fileblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(app.echo.clone())).block(
            Block::new()
                .padding(Padding::horizontal(2))
                .borders(Borders::TOP),
        ),
        echoblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(app.filter_str.clone())).block(
            Block::new()
                .padding(Padding::horizontal(2))
                .borders(Borders::TOP | Borders::RIGHT),
        ),
        filterblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(format!(">>> {}█", app.command)))
            .block(Block::new().borders(Borders::TOP)),
        cmdblock,
    );
}
