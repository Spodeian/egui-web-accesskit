use eframe::egui;
use egui_web_accesskit::WebAccessKitPlugin;

struct DemoApp {
    clicks: u32,
    accepted: bool,
    volume: f64,
    note: String,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            clicks: 0,
            accepted: false,
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

            ui.checkbox(&mut self.accepted, "Enable notifications");
            ui.add(egui::Slider::new(&mut self.volume, 0.0..=100.0).text("Volume"));

            ui.horizontal(|ui| {
                let label = ui.label("Note:");
                ui.text_edit_singleline(&mut self.note)
                    .labelled_by(label.id);
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
