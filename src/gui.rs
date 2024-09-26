use clap::{command, value_parser, Arg};
use egui::text::{CCursor, CCursorRange};
use ftag::{
    core::Error,
    interactive::{InteractiveSession, State},
    query::DenseTagTable,
};
use std::path::PathBuf;

fn main() -> Result<(), Error> {
    let matches = command!()
        .arg(
            Arg::new("path")
                .long("path")
                .short('p')
                .required(false)
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();
    let current_dir = if let Some(rootdir) = matches.get_one::<PathBuf>("path") {
        rootdir
            .canonicalize()
            .map_err(|_| Error::InvalidPath(rootdir.clone()))?
    } else {
        std::env::current_dir().map_err(|_| Error::InvalidWorkingDirectory)?
    };
    let table = DenseTagTable::from_dir(current_dir)?;
    let options = eframe::NativeOptions {
        follow_system_theme: true,
        viewport: egui::ViewportBuilder {
            maximized: Some(true),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "ftagui",
        options,
        Box::new(|cc| {
            let ctx = &cc.egui_ctx;
            ctx.set_pixels_per_point(1.2);
            egui_extras::install_image_loaders(ctx);
            Ok(Box::from(App {
                session: InteractiveSession::init(table),
                page_index: 0,
                num_pages: 1,
            }))
        }),
    )
    .map_err(Error::GUIFailure)
}

struct App {
    session: InteractiveSession,
    page_index: usize,
    num_pages: usize,
}

impl App {
    fn render_text_preview(ui: &mut egui::Ui, text: String) -> egui::Response {
        ui.add(
            egui::Label::new(
                egui::widget_text::RichText::new(text).text_style(egui::TextStyle::Monospace),
            )
            .sense(egui::Sense::click().union(egui::Sense::hover()))
            .selectable(false),
        )
    }

    fn render_grid_preview(&mut self, ui: &mut egui::Ui) {
        const DESIRED_ROW_HEIGHT: f32 = 200.;
        const DESIRED_COL_WIDTH: f32 = 200.;
        const ROW_SPACING: f32 = 5.;
        const COL_SPACING: f32 = 5.;
        let (ncols, ncells, row_height, col_width) = {
            let ncols = f32::ceil(ui.available_width() / (DESIRED_COL_WIDTH + COL_SPACING));
            let nrows = f32::ceil(ui.available_height() / (DESIRED_ROW_HEIGHT + ROW_SPACING));
            let row_height = (ui.available_height() / nrows) - ROW_SPACING;
            let col_width = (ui.available_width() / ncols) - COL_SPACING;
            (
                ncols as usize,
                ncols as usize * nrows as usize,
                row_height,
                col_width,
            )
        };
        // This takes the ceil of integer division.
        self.num_pages = usize::max((self.session.filelist().len() + ncells - 1) / ncells, 1);
        let mut echo = None;
        egui::Grid::new("image_grid")
            .min_row_height(row_height)
            .max_col_width(col_width)
            .striped(true)
            .spacing(egui::Vec2::new(COL_SPACING, ROW_SPACING))
            .show(ui, |ui| {
                for (counter, (relpath, path)) in self
                    .session
                    .filelist()
                    .iter()
                    .map(|file| {
                        let mut path = self.session.table().path().to_path_buf();
                        path.push(file);
                        (file, path)
                    })
                    .skip(self.page_index * ncells)
                    .take(ncells)
                    .enumerate()
                {
                    ui.vertical_centered(|ui| {
                        let response = match path.extension() {
                            Some(ext) => match ext.to_ascii_lowercase().to_str() {
                                Some(ext) => match ext {
                                    "png" | "jpg" | "jpeg" | "bmp" | "webp" => ui.add(
                                        egui::Image::from_uri(format!("file://{}", path.display()))
                                            .rounding(10.)
                                            .show_loading_spinner(true)
                                            .maintain_aspect_ratio(true)
                                            .sense(
                                                egui::Sense::click().union(egui::Sense::hover()),
                                            ),
                                    ),
                                    "pdf" => Self::render_text_preview(
                                        ui,
                                        format!("document:\n{}", relpath),
                                    ),
                                    "mov" | "flv" | "mp4" | "3gp" => Self::render_text_preview(
                                        ui,
                                        format!("video:\n{}", relpath),
                                    ),
                                    _ => {
                                        Self::render_text_preview(ui, format!("file:\n{}", relpath))
                                    }
                                },
                                None => {
                                    Self::render_text_preview(ui, format!("file:\n{}", relpath))
                                }
                            },
                            None => Self::render_text_preview(ui, format!("file:\n{}", relpath)),
                        };
                        if response.double_clicked() {
                            if let Err(_) = opener::open(&path) {
                                echo = Some("Unable to open the file.");
                            }
                        }
                        if response.hovered() {
                            response.show_tooltip_ui(|ui| {
                                ui.monospace(ftag::core::what_is(&path).unwrap_or(String::from(
                                    "Unable to fetch the description of this file.",
                                )));
                            });
                        }
                    });
                    if counter % ncols == ncols - 1 {
                        ui.end_row();
                    }
                }
                if let Some(message) = echo {
                    self.session.set_echo(message);
                }
            });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tags panel.
        egui::SidePanel::left("left_panel").show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for tag in self.session.taglist() {
                    ui.add(
                        egui::Label::new(
                            egui::widget_text::RichText::new(tag)
                                .text_style(egui::TextStyle::Monospace),
                        )
                        .selectable(false),
                    );
                }
            });
        });
        // Current filter string.
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::widget_text::RichText::new(format!(
                            "{}: [{} / {}]",
                            if self.session.filter_str().is_empty() {
                                "ALL_TAGS"
                            } else {
                                self.session.filter_str()
                            },
                            self.page_index + 1,
                            self.num_pages
                        ))
                        .text_style(egui::TextStyle::Monospace),
                    )
                    .selectable(false),
                );
            });
        });
        // Input field and echo string.
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::widget_text::RichText::new(self.session.echo())
                            .text_style(egui::TextStyle::Monospace),
                    )
                    .selectable(false),
                );
                ui.separator();
                let mut output = egui::TextEdit::singleline(self.session.command_mut())
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .min_size(egui::Vec2::new(100., 24.))
                    .font(egui::FontId::monospace(14.))
                    .horizontal_align(egui::Align::Center)
                    .vertical_align(egui::Align::Center)
                    .hint_text("query filter:")
                    .show(ui);
                let query_response = output.response;
                if query_response.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        // User hit return with a query.
                        self.session.process_input();
                        match self.session.state() {
                            State::Default | State::Autocomplete => {} // Do nothing.
                            State::ListsUpdated => {
                                self.page_index = 0;
                                self.session.set_state(State::Default);
                            }
                            State::Exit => {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        // Move the cursor to the end of the line, say, after autocomplete.
                        output.state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(self.session.command().len()),
                            CCursor::new(self.session.command().len()),
                        )));
                        output.state.store(ctx, query_response.id);
                    } else if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                        self.session.autocomplete();
                    }
                } else if query_response.changed() {
                    self.session.stop_autocomplete();
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
                    self.page_index = usize::clamp(self.page_index + 1, 0, self.num_pages - 1);
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::P)) {
                    self.page_index =
                        usize::clamp(self.page_index.saturating_sub(1), 0, self.num_pages - 1);
                }
                query_response.request_focus();
            });
        });
        // Files previews.
        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_grid_preview(ui);
        });
    }
}
