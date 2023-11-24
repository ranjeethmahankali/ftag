use crate::query::DenseTagTable;
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
use std::io::stdout;

enum State {
    Stale,
    Fresh,
    Exit,
}
use State::*;

struct App {
    table: DenseTagTable,
    command: String,
    echo: String,
    state: State,
    // Tags and files that are actually visible with filter.s
    taglist: Vec<String>,
    filelist: Vec<String>,
    // UI management.
    scroll: usize,
    scrollstate: ScrollbarState,
}

impl App {
    fn from(table: DenseTagTable) -> App {
        let taglist = table.tags().to_vec();
        let filelist = table.files().to_vec();
        let ntags = taglist.len();
        App {
            table,
            command: String::new(),
            echo: String::new(),
            state: Stale,
            taglist,
            filelist,
            scroll: 0,
            scrollstate: ScrollbarState::new(ntags),
        }
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

    fn keyevent(&mut self, evt: KeyEvent) {
        self.state = Stale;
        match evt.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => match evt.code {
                KeyCode::Char(c) => {
                    self.command.push(c);
                }
                KeyCode::Backspace => {
                    self.command.pop();
                }
                KeyCode::Enter => {
                    self.process_input();
                }
                KeyCode::Esc => {
                    self.command.clear();
                }
                KeyCode::Up => {
                    self.scroll = self.scroll.saturating_sub(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                KeyCode::Down => {
                    self.scroll = self.scroll.saturating_add(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                _ => {}
            },
            KeyEventKind::Release => {} // Do nothing.
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
            match crossterm::event::read()? {
                event::Event::Key(key) => {
                    app.keyevent(key);
                }
                event::Event::Resize(_, _) => {
                    terminal.draw(|f| render(f, app))?;
                }
                _ => {} //  Do nothing.
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
    const TAGWIDTH_PERCENT: u16 = 20;
    let hlayout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Percentage(TAGWIDTH_PERCENT),
            Constraint::Percentage(100 - TAGWIDTH_PERCENT),
        ])
        .split(f.size());
    let vlayout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Max(1001),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(hlayout[1]);
    let tagblock = hlayout[0];
    let fileblock = vlayout[0];
    let echoblock = vlayout[1];
    let cmdblock = vlayout[2];
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
                .padding(Padding::horizontal(3)),
        )
        .scroll((app.scroll as u16, 0)),
        tagblock,
    );
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
        Paragraph::new(Text::from(format!(">>> {}█", app.command)))
            .block(Block::new().borders(Borders::TOP)),
        cmdblock,
    );
    app.fresh();
}
