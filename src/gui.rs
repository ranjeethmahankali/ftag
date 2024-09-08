use clap::{command, value_parser, Arg};
use egui::KeyboardShortcut;
use ftag::{core::Error, filter::Filter, query::DenseTagTable};
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
                filter_str: String::new(),
                filter: Default::default(),
                table,
                response: String::new(),
            }))
        }),
    )
    .map_err(Error::GUIFailure)
}

struct App {
    filter_str: String,
    filter: Filter<usize>,
    table: DenseTagTable,
    response: String,
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
                    for tag in self.table.tags() {
                        ui.monospace(tag);
                    }
                });
            });
        egui::TopBottomPanel::bottom("query_panel")
            .exact_height(80.)
            .show(ctx, |ui| {
                // Query field.
                let query_field = egui::TextEdit::singleline(&mut self.filter_str)
                    .desired_width(f32::INFINITY)
                    .min_size(egui::Vec2::new(100., 24.))
                    .font(egui::FontId::monospace(14.))
                    .horizontal_align(egui::Align::Center)
                    .vertical_align(egui::Align::Center)
                    .hint_text("query filter:");
                let query_response = query_field.show(ui).response;
                if query_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    // User hit return with a query.
                    self.response.clear();
                    match Filter::<usize>::parse(&self.filter_str, &self.table) {
                        Ok(filter) => {
                            self.response = format!("{:?}", filter.text(self.table.tags()));
                            self.filter = filter;
                        }
                        Err(e) => self.response = format!("{:?}", e),
                    }
                }
                query_response.request_focus();
                ui.add_space(12.);
                ui.vertical_centered(|ui| {
                    ui.monospace(&mut self.response); // Render the response.
                });
            });
    }
}
