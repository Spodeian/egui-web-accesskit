use eframe::egui;
use egui_web_accesskit::WebAccessKitPlugin;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Density {
    Compact,
    Spacious,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoTheme {
    Dark,
    Light,
}

impl DemoTheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

fn scoped_row(ui: &mut egui::Ui, salt: &'static str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let row_id = ui.make_persistent_id(salt);
    ui.scope_builder(egui::UiBuilder::new().scope_id(row_id), add_contents);
}

struct DemoApp {
    clicks: u32,
    accepted: bool,
    tri_checked: bool,
    tri_indeterminate: bool,
    opacity: f64,
    volume: f64,
    note: String,
    force_warning: bool,
    bio: String,
    secret: String,
    muted: bool,
    live_status: &'static str,
    density: Density,
    retries: i32,
    progress: f32,
    theme: DemoTheme,
    show_modal: bool,
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
            force_warning: false,
            bio: String::from("Line 1"),
            secret: String::from("s3cr3t"),
            muted: false,
            live_status: "Audio active",
            density: Density::Compact,
            retries: 3,
            progress: 0.75,
            theme: DemoTheme::Dark,
            show_modal: false,
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

            scoped_row(ui, "live_status", |ui| {
                let live_resp = ui.label(self.live_status);
                ui.ctx().accesskit_node_builder(live_resp.id, |node| {
                    node.set_live(egui::accesskit::Live::Polite);
                });
            });

            ui.separator();

            ui.columns(2, |cols| {
                cols[0].label(format!("Button clicked {} times", self.clicks));

                cols[0].horizontal(|ui| {
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
                });

                cols[0].checkbox(&mut self.accepted, "Enable notifications");
                if cols[0]
                    .add(
                        egui::Checkbox::new(&mut self.tri_checked, "Tri-state option")
                            .indeterminate(self.tri_indeterminate),
                    )
                    .clicked()
                {
                    self.tri_indeterminate = false;
                }

                cols[0].add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).text("Opacity"));
                let vol_resp = cols[0].add(
                    egui::Slider::new(&mut self.volume, 0.0..=100.0)
                        .step_by(5.0)
                        .text("Volume"),
                );
                cols[0].ctx().accesskit_node_builder(vol_resp.id, |node| {
                    node.set_value(format!("{}%", self.volume as i32));
                });

                let link = cols[0].hyperlink_to("egui docs", "https://docs.rs/egui");
                cols[0].ctx().accesskit_node_builder(link.id, |node| {
                    node.set_url("https://docs.rs/egui");
                });

                if self.note.starts_with('!') || self.force_warning {
                    cols[0].label("Warning banner");
                }

                scoped_row(&mut cols[0], "note_row", |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Note:");
                        let note_id = ui.make_persistent_id("note_edit");
                        ui.add(egui::TextEdit::singleline(&mut self.note).id(note_id))
                            .labelled_by(label.id);
                        if ui.button("Clear note").clicked() {
                            self.note.clear();
                        }
                        if ui.button("Toggle warning").clicked() {
                            self.force_warning = !self.force_warning;
                        }
                    });
                });

                egui::CollapsingHeader::new("Advanced details").show(&mut cols[0], |ui| {
                    ui.label("Hidden detail text");
                });

                scoped_row(&mut cols[0], "bio_row", |ui| {
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

                scoped_row(&mut cols[0], "token_row", |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Token:");
                        let mut token: &str = "RO-123";
                        ui.text_edit_singleline(&mut token).labelled_by(label.id);
                    });
                });

                scoped_row(&mut cols[0], "pw_row", |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Password:");
                        ui.add(egui::TextEdit::singleline(&mut self.secret).password(true))
                            .labelled_by(label.id);
                        ui.label(format!("Secret length: {}", self.secret.chars().count()));
                    });
                });

                egui::ScrollArea::vertical()
                    .id_salt("demo_scroll")
                    .max_height(55.0)
                    .show(&mut cols[0], |ui| {
                        let _ = ui.button("Top item");
                        for i in 1..8 {
                            ui.label(format!("Item {i}"));
                        }
                        if ui.button("Bottom action").clicked() {
                            self.clicks += 1;
                        }
                    });

                let more = cols[1].heading("More controls");
                cols[1].ctx().accesskit_node_builder(more.id, |node| {
                    node.set_role(egui::accesskit::Role::Heading);
                    node.set_level(2);
                });

                if cols[1]
                    .add(egui::Button::new("Mute audio").selected(self.muted))
                    .clicked()
                {
                    self.muted = !self.muted;
                    self.live_status = if self.muted {
                        "Audio muted"
                    } else {
                        "Audio active"
                    };
                }

                cols[1].horizontal(|ui| {
                    ui.label("Density:");
                    ui.radio_value(&mut self.density, Density::Compact, "Compact");
                    ui.radio_value(&mut self.density, Density::Spacious, "Spacious");
                });

                let _ = cols[1].add_enabled(false, egui::Button::new("Locked action"));

                scoped_row(&mut cols[1], "retries_row", |ui| {
                    ui.horizontal(|ui| {
                        let lbl = ui.label("Retries:");
                        ui.add(egui::DragValue::new(&mut self.retries).range(0..=10).speed(1.0))
                            .labelled_by(lbl.id);
                    });
                });

                cols[1].add(egui::ProgressBar::new(self.progress).text("Sync progress"));

                egui::ComboBox::from_label("Theme")
                    .selected_text(self.theme.as_str())
                    .show_ui(&mut cols[1], |ui| {
                        ui.selectable_value(&mut self.theme, DemoTheme::Dark, "Dark");
                        ui.selectable_value(&mut self.theme, DemoTheme::Light, "Light");
                    });

                if cols[1].button("Open dialog").clicked() {
                    self.show_modal = true;
                }
            });

            if self.show_modal {
                let modal_id = ui.make_persistent_id("confirm_modal");
                egui::Modal::new(modal_id).show(ui.ctx(), |ui| {
                    let dialog_id = ui.response().id;
                    ui.ctx().accesskit_node_builder(dialog_id, |node| {
                        node.set_role(egui::accesskit::Role::AlertDialog);
                        node.set_label("Confirmation dialog");
                        node.set_modal();
                    });
                    ui.label("Confirm action?");
                    if ui.button("Close dialog").clicked() {
                        self.show_modal = false;
                    }
                });
            }
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
