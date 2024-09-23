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
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::from(App {
                session: InteractiveSession::init(table),
            }))
        }),
    )
    .map_err(Error::GUIFailure)
}

struct App {
    session: InteractiveSession,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_pixels_per_point(1.2);
        // Tags panel.
        const SIDE_PANEL_WIDTH: f32 = 128.;
        egui::SidePanel::left("left_panel")
            .exact_width(SIDE_PANEL_WIDTH)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for tag in self.session.taglist() {
                        ui.monospace(tag);
                    }
                });
            });
        // Current filter string.
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.monospace(self.session.filter_str());
            });
        });
        // Files.
        egui::CentralPanel::default().show(ctx, |ui| {
            const CELL_HEIGHT: f32 = 256.;
            const CELL_WIDTH: f32 = 256.;
            let width = ui.available_width();
            egui::Grid::new("image_grid")
                .min_col_width(CELL_WIDTH)
                .min_row_height(CELL_HEIGHT)
                .striped(true)
                .show(ui, |ui| {
                    for (counter, file) in (0..self.session.filelist().len())
                        .filter_map(|i| {
                            let path = self.session.absolute_filepath(i);
                            match path.extension() {
                                Some(ext) if ext == "JPG" => Some(path),
                                Some(ext) if ext == "jpg" => Some(path),
                                Some(ext) if ext == "PNG" => Some(path),
                                Some(ext) if ext == "png" => Some(path),
                                _ => None,
                            }
                        })
                        .take(4)
                        .enumerate()
                    {
                        ui.add(
                            egui::Image::new(format!("file://{}", file.display()))
                                .show_loading_spinner(true)
                                .rounding(2.),
                        );
                        if counter + 1 >= (width / CELL_WIDTH).floor() as usize {
                            ui.end_row();
                        }
                    }
                });
        });
        // Input field and echo string.
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.monospace(self.session.echo());
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
                }
                query_response.request_focus();
            });
        });
    }
}
