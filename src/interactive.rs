use crate::query::DenseTagTable;
use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{Backend, Constraint, CrosstermBackend, Direction, Layout, Terminal},
    text::Text,
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph},
    Frame,
};
use std::io::stdout;

enum State {
    Stale,
    Fresh,
    Exit,
}
use State::*;

struct App {
    table: DenseTagTable,
    tagwidth: u16,
    filewidth: u16,
    command: String,
    echo: String,
    state: State,
}

impl App {
    fn from(table: DenseTagTable) -> App {
        let mut state = App {
            table,
            tagwidth: 0,
            filewidth: 0,
            command: String::new(),
            echo: String::new(),
            state: Stale,
        };
        state.tagwidth = usize::max(
            state
                .table
                .tags()
                .iter()
                .map(|t| t.len())
                .max()
                .unwrap_or(0),
            32,
        ) as u16;
        state.filewidth = state
            .table
            .files()
            .iter()
            .map(|f| f.len())
            .max()
            .unwrap_or(0) as u16;
        return state;
    }

    fn fresh(&mut self) {
        self.state = match self.state {
            Stale => Fresh,
            Fresh => Fresh,
            Exit => Exit,
        };
    }

    fn process_input(&mut self) {
        if self.command == "exit" {
            self.state = Exit;
        }
        self.echo = format!(
            "Received input: {}",
            std::mem::replace(&mut self.command, String::new())
        );
    }

    fn keystroke(&mut self, code: KeyCode) {
        self.state = Stale;
        match code {
            KeyCode::Char(c) => {
                self.command.push(c);
            }
            KeyCode::Backspace => {
                self.command.pop();
            }
            KeyCode::Enter => {
                self.process_input();
            }
            _ => {}
        }
    }
}

pub(crate) fn start(table: DenseTagTable) -> std::io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let mut app = App::from(table);
    run_app(&mut terminal, &mut app)?;
    // Clean up.
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    return Ok(());
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> std::io::Result<()> {
    const DELAY: u64 = 20;
    // Main application loop
    loop {
        // Poll events to see if redraw needed.
        if event::poll(std::time::Duration::from_millis(DELAY))? {
            // If a key event occurs, handle it
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.keystroke(key.code);
                }
            }
        }
        match app.state {
            Stale => {
                terminal.draw(|f| render(f, app))?;
            }
            Fresh => {} // Do nothing.
            Exit => break,
        };
    }
    return Ok(());
}

fn render(f: &mut Frame, app: &mut App) {
    let hlayout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Length(app.tagwidth),
            Constraint::Min(app.filewidth),
        ])
        .split(f.size());
    let vlayout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Percentage(90),
            Constraint::Percentage(5),
            Constraint::Length(1),
        ])
        .split(hlayout[1]);
    let tagblock = hlayout[0];
    let fileblock = vlayout[0];
    let echoblock = vlayout[1];
    let cmdblock = vlayout[2];
    f.render_widget(
        List::new(
            app.table
                .tags()
                .iter()
                .map(|t| ListItem::new(t.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(2)),
        ),
        tagblock,
    );
    f.render_widget(
        List::new(
            app.table
                .files()
                .iter()
                .map(|f| ListItem::new(f.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(2)),
        ),
        fileblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(app.echo.clone()))
            .block(Block::new().padding(Padding::horizontal(2))),
        echoblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(format!(">>> {}█", app.command)))
            .block(Block::new().borders(Borders::ALL)),
        cmdblock,
    );
    app.fresh();
}
