use eframe::egui;
use egui_web_accesskit::WebAccessKitPlugin;

struct DemoApp {
    clicks: u32,
    accepted: bool,
    tri_checked: bool,
    tri_indeterminate: bool,
    opacity: f64,
    volume: f64,
    note: String,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            clicks: 0,
            accepted: false,
            tri_checked: false,
            tri_indeterminate: true,
            opacity: 0.35,
            volume: 50.0,
            note: String::from("Type here"),
        }
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("egui-web-accesskit demo");
            ui.label(format!("Button clicked {} times", self.clicks));

            if ui.button("Increment counter").clicked() {
                self.clicks += 1;
            }

            if ui.button("Zoom 1.5x").clicked() {
                ui.ctx().set_zoom_factor(1.5);
            }

            ui.checkbox(&mut self.accepted, "Enable notifications");
            if ui
                .add(
                    egui::Checkbox::new(&mut self.tri_checked, "Tri-state option")
                        .indeterminate(self.tri_indeterminate),
                )
                .clicked()
            {
                self.tri_indeterminate = false;
            }

            ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).text("Opacity"));
            ui.add(
                egui::Slider::new(&mut self.volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );

            if self.note.starts_with('!') {
                ui.label("Warning banner");
            }

            let note_row_id = ui.make_persistent_id("note_row");
            ui.scope_builder(egui::UiBuilder::new().scope_id(note_row_id), |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Note:");
                        ui.text_edit_singleline(&mut self.note)
                            .labelled_by(label.id);
                        if ui.button("Clear note").clicked() {
                            self.note.clear();
                        }
                    });
                });

            egui::CollapsingHeader::new("Advanced details").show(ui, |ui| {
                ui.label("Hidden detail text");
            });
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "egui-web-accesskit demo",
        options,
        Box::new(|cc| {
            cc.egui_ctx
                .add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
            Ok(Box::new(DemoApp::default()))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing #the_canvas_id canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#the_canvas_id is not an HtmlCanvasElement");

        let web_options = eframe::WebOptions::default();
        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| {
                    cc.egui_ctx
                        .add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
                    Ok(Box::new(DemoApp::default()))
                }),
            )
            .await
            .expect("failed to start eframe");
    });
}
