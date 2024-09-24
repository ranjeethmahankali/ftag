use crate::{
    core::what_is,
    filter::{Filter, FilterParseError},
    query::DenseTagTable,
};
use std::{fmt::Debug, path::PathBuf};

/// State of the app.
pub enum State {
    Default,
    Autocomplete,
    ListsUpdated,
    Exit,
}

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

pub struct InteractiveSession {
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
    // Autocomplete
    command_completions: Box<[String]>,
    suggestions: Vec<String>,
    suggestion_index: usize,
}

impl InteractiveSession {
    pub fn init(table: DenseTagTable) -> InteractiveSession {
        let taglist = table.tags().to_vec();
        let ntags = table.tags().len();
        let nfiles = table.files().len();
        let mut app = InteractiveSession {
            table,
            command: String::new(),
            echo: String::new(),
            state: State::Default,
            tag_active: vec![true; ntags],
            taglist,
            filelist: Vec::with_capacity(nfiles),
            filtered_indices: (0..nfiles).collect(),
            filter_str: String::new(),
            command_completions: ["exit", "quit", "reset", "whatis", "open"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            suggestions: Vec::new(),
            suggestion_index: 0,
        };
        InteractiveSession::update_file_list(
            &app.filtered_indices,
            app.table.files(),
            &mut app.filelist,
        );
        app
    }

    fn reset(&mut self) {
        self.filter_str.clear();
        self.filtered_indices.clear();
        self.filtered_indices.extend(0..self.num_files());
        self.update_lists();
        self.echo.clear();
        self.state = State::Default;
        self.tag_active.fill(true);
        self.state = State::ListsUpdated;
    }

    fn parse_index_to_filepath(&self, numstr: &str) -> Result<PathBuf, Error> {
        let index = match numstr.parse::<usize>() {
            Ok(num) if num < self.filtered_indices.len() => Ok(num),
            Ok(num) => Err(Error::InvalidCommand(format!(
                "{num} is not a valid choice. Please choose an index between 0 and  {}",
                self.filtered_indices.len()
            ))),
            Err(_) => Err(Error::InvalidCommand(format!(
                "Unable to parse '{numstr}' to an index."
            ))),
        }?;
        let mut path = self.table.path().to_path_buf();
        path.push(&self.table.files()[self.filtered_indices[index]]);
        Ok(path)
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
                Filter::<usize>::parse(&newfilter, &self.table).map_err(Error::InvalidFilter)?,
            ))
        }
    }

    fn num_files(&self) -> usize {
        self.table.files().len()
    }

    fn update_file_list(indices: &[usize], files: &[String], dst: &mut Vec<String>) {
        dst.clear();
        dst.reserve(indices.len());
        dst.extend(indices.iter().map(|i| files[*i].clone()));
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

    pub fn table(&self) -> &DenseTagTable {
        &self.table
    }

    pub fn taglist(&self) -> &[String] {
        &self.taglist
    }

    pub fn command_mut(&mut self) -> &mut String {
        &mut self.command
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub fn filelist(&self) -> &[String] {
        &self.filelist
    }

    pub fn echo(&self) -> &str {
        &self.echo
    }

    pub fn filter_str(&self) -> &str {
        &self.filter_str
    }

    pub fn process_input(&mut self) {
        match self.state {
            State::ListsUpdated | State::Default => {
                match self.parse_command() {
                    Ok(cmd) => match cmd {
                        Command::Exit | Command::Quit => self.state = State::Exit,
                        Command::WhatIs(path) => {
                            self.echo = what_is(&path)
                                .unwrap_or(String::from(
                                    "Unable to fetch the description of this file.",
                                ))
                                .to_string();
                        }
                        Command::Filter(filter) => {
                            self.filtered_indices.clear();
                            self.filtered_indices.extend(
                                (0..self.num_files()).filter(|i| filter.eval(self.table.flags(*i))),
                            );
                            self.update_lists();
                            self.filter_str = filter.text(self.table.tags());
                            self.state = State::ListsUpdated;
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
            State::Autocomplete => {
                self.command.truncate(self.last_word_start());
                self.command
                    .push_str(&self.suggestions[self.suggestion_index]);
                self.state = State::Default;
                self.echo.clear();
            }
            State::Exit => {} // Do nothing.
        }
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

    pub fn autocomplete(&mut self) {
        let next_state = match self.state {
            State::ListsUpdated | State::Default => {
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
                State::Autocomplete
            }
            State::Autocomplete if !self.suggestions.is_empty() => {
                self.suggestion_index = (self.suggestion_index + 1) % self.suggestions.len();
                self.show_suggestions();
                State::Autocomplete
            }
            State::Autocomplete => State::Autocomplete,
            State::Exit => State::Exit, // Do nothing.
        };
        self.state = next_state;
    }

    pub fn stop_autocomplete(&mut self) {
        match &self.state {
            State::ListsUpdated | State::Default => {}
            // Do nothing.
            State::Autocomplete => {
                self.suggestions.clear();
                self.suggestion_index = 0;
                self.echo.clear();
                self.state = State::Default;
            }
            State::Exit => {} // Do nothing.
        }
    }
}
