use clap::{command, value_parser, Arg};
use egui::{
    popup_above_or_below_widget, popup_below_widget, text::LayoutJob, CentralPanel,
    FontDefinitions, Id, Rect,
};
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
    let options = eframe::NativeOptions::default();
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
            .exact_height(38.)
            .show(ctx, |ui| {
                let field = egui::TextEdit::singleline(&mut self.filter_str)
                    .desired_width(f32::INFINITY)
                    .min_size(egui::Vec2::new(100., 24.))
                    .font(egui::FontId::monospace(14.))
                    .horizontal_align(egui::Align::Center)
                    .hint_text("query filter:");
                let response = field.show(ui).response;
                response.request_focus();
                if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.response.clear();
                    let popup_id = Id::new("response_popup");
                    match Filter::<usize>::parse(&self.filter_str, &self.table) {
                        Ok(filter) => {
                            self.response = format!("{:?}", filter.text(self.table.tags()));
                            self.filter = filter;
                        }
                        Err(e) => self.response = format!("{:?}", e),
                    }
                    if !self.response.is_empty() {
                        println!("{}", self.response); // DEBUG
                        ui.memory_mut(|mem| mem.open_popup(popup_id));
                    }
                    popup_above_or_below_widget(
                        &ui,
                        popup_id,
                        &response,
                        egui::AboveOrBelow::Above,
                        egui::PopupCloseBehavior::IgnoreClicks,
                        |ui| {
                            ui.monospace(&self.response);
                        },
                    );
                }
            });

        // CentralPanel::default().show(ctx, |ui| {
        //     ui.label("PopupCloseBehavior::CloseOnClickAway popup");
        //     let response = ui.button("Open");
        //     let popup_id = Id::new("popup_id");

        //     if response.clicked() {
        //         ui.memory_mut(|mem| mem.toggle_popup(popup_id));
        //     }

        //     popup_below_widget(
        //         ui,
        //         popup_id,
        //         &response,
        //         egui::PopupCloseBehavior::CloseOnClickOutside,
        //         |ui| {
        //             ui.set_min_width(300.0);
        //             ui.label("This popup will be open even if you click the checkbox");
        //         },
        //     );

        //     ui.label("PopupCloseBehavior::CloseOnClick popup");
        // });
    }
}
