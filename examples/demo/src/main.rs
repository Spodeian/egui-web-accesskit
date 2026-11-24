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
    bio: String,
    secret: String,
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
            bio: String::from("Line 1"),
            secret: String::from("s3cr3t"),
        }
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            let heading = ui.heading("egui-web-accesskit demo");
            ui.ctx().accesskit_node_builder(heading.id, |node| {
                node.set_role(egui::accesskit::Role::Heading);
                node.set_level(1);
            });
            ui.label(format!("Button clicked {} times", self.clicks));

            let inc = ui.button("Increment counter");
            ui.ctx().accesskit_node_builder(inc.id, |node| {
                node.set_description("Adds one to the click counter");
            });
            if inc.clicked() {
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
            let vol_resp = ui.add(
                egui::Slider::new(&mut self.volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );
            ui.ctx().accesskit_node_builder(vol_resp.id, |node| {
                node.set_value(format!("{}%", self.volume as i32));
            });

            let link = ui.hyperlink_to("egui docs", "https://docs.rs/egui");
            ui.ctx().accesskit_node_builder(link.id, |node| {
                node.set_url("https://docs.rs/egui");
            });

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

            let bio_row_id = ui.make_persistent_id("bio_row");
            ui.scope_builder(egui::UiBuilder::new().scope_id(bio_row_id), |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Bio:");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.bio)
                            .hint_text("Write bio")
                            .desired_rows(2),
                    )
                    .labelled_by(label.id);
                });
            });

            let token_row_id = ui.make_persistent_id("token_row");
            ui.scope_builder(egui::UiBuilder::new().scope_id(token_row_id), |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Token:");
                    let mut token: &str = "RO-123";
                    ui.text_edit_singleline(&mut token).labelled_by(label.id);
                });
            });

            let pw_row_id = ui.make_persistent_id("pw_row");
            ui.scope_builder(egui::UiBuilder::new().scope_id(pw_row_id), |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Password:");
                    ui.add(egui::TextEdit::singleline(&mut self.secret).password(true))
                        .labelled_by(label.id);
                    ui.label(format!("Secret length: {}", self.secret.chars().count()));
                });
            });

            egui::ScrollArea::vertical().max_height(50.0).show(ui, |ui| {
                for i in 0..8 {
                    ui.label(format!("Item {i}"));
                }
                if ui.button("Bottom action").clicked() {
                    self.clicks += 1;
                }
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
        let window = web_sys::window().expect("no window");
        let document = window.document().expect("no document");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("missing #the_canvas_id canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("#the_canvas_id is not an HtmlCanvasElement");

        let shadow_mode = window
            .location()
            .search()
            .ok()
            .is_some_and(|s| s.contains("shadow=1"));

        let plugin = if shadow_mode {
            canvas.remove();
            let host = document.create_element("div").expect("shadow host");
            let _ = host.set_attribute("id", "shadow-host");
            let _ = host.set_attribute("style", "width: 100%; height: 100%;");
            document
                .body()
                .expect("no document body")
                .append_child(&host)
                .expect("append shadow host");
            let shadow = host
                .attach_shadow(&web_sys::ShadowRootInit::new(
                    web_sys::ShadowRootMode::Open,
                ))
                .expect("attach shadow root");
            let _ = canvas.set_attribute("style", "width: 100%; height: 100%; display: block;");
            shadow
                .append_child(&canvas)
                .expect("append canvas to shadow root");
            WebAccessKitPlugin::from_canvas(canvas.clone())
        } else {
            WebAccessKitPlugin::new("the_canvas_id")
        };

        let web_options = eframe::WebOptions::default();
        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(move |cc| {
                    cc.egui_ctx.add_plugin(plugin);
                    Ok(Box::new(DemoApp::default()))
                }),
            )
            .await
            .expect("failed to start eframe");
    });
}
