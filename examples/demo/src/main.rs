#[cfg(target_arch = "wasm32")]
use eframe::egui;
#[cfg(target_arch = "wasm32")]
use egui_web_accesskit::WebAccessKitPlugin;

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Density {
    Compact,
    Spacious,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoTheme {
    Dark,
    Light,
}

#[cfg(target_arch = "wasm32")]
impl DemoTheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn scoped_row(ui: &mut egui::Ui, salt: &'static str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let row_id = ui.make_persistent_id(salt);
    ui.scope_builder(egui::UiBuilder::new().scope_id(row_id), add_contents);
}

#[cfg(target_arch = "wasm32")]
struct DemoApp {
    clicks: u32,
    accepted: bool,
    tri_checked: bool,
    tri_indeterminate: bool,
    opacity: f64,
    volume: f64,
    note: String,
    note_submitted: u32,
    force_warning: bool,
    bio: String,
    search: String,
    secret: String,
    muted: bool,
    live_status: &'static str,
    density: Density,
    retries: i32,
    progress: f32,
    theme: DemoTheme,
    show_modal: bool,
    dialog_dismissals: u32,
    dialog_confirmed: bool,
    /// Frames left before `WebAccessKitPlugin::teardown` runs (set by `?teardown=1`).
    teardown_frames: Option<u32>,
    torn_down: bool,
    /// Whether the bridge was registered at startup (`false` when the page was opened with `?bridge=off`).
    mirror_on: bool,
}

#[cfg(target_arch = "wasm32")]
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
            note_submitted: 0,
            force_warning: false,
            bio: String::from("Line 1"),
            search: String::new(),
            secret: String::from("s3cr3t"),
            muted: false,
            live_status: "Audio active",
            density: Density::Compact,
            retries: 3,
            progress: 0.75,
            theme: DemoTheme::Dark,
            show_modal: false,
            dialog_dismissals: 0,
            dialog_confirmed: false,
            teardown_frames: None,
            torn_down: false,
            mirror_on: true,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(frames) = self.teardown_frames {
            if frames == 0 {
                WebAccessKitPlugin::teardown(ui.ctx());
                self.teardown_frames = None;
                self.torn_down = true;
            } else {
                self.teardown_frames = Some(frames - 1);
                ui.ctx().request_repaint();
            }
        }
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
                        let note_resp = ui
                            .add(egui::TextEdit::singleline(&mut self.note).id(note_id))
                            .labelled_by(label.id);
                        if note_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.note_submitted += 1;
                        }
                        if ui.button("Clear note").clicked() {
                            self.note.clear();
                        }
                        if ui.button("Toggle warning").clicked() {
                            self.force_warning = !self.force_warning;
                        }
                    });
                });
                cols[0].label(format!("Note submits: {}", self.note_submitted));

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

                // Explicit AccessKit autocomplete: the only field that should expose aria-autocomplete.
                scoped_row(&mut cols[0], "search_row", |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Search:");
                        let search = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .hint_text("Type to search"),
                            )
                            .labelled_by(label.id);
                        ui.ctx().accesskit_node_builder(search.id, |node| {
                            node.set_auto_complete(egui::accesskit::AutoComplete::List);
                        });
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
                        ui.add(
                            egui::DragValue::new(&mut self.retries)
                                .range(0..=10)
                                .speed(1.0),
                        )
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
                let dialog_result = if self.dialog_confirmed {
                    "confirmed"
                } else if self.dialog_dismissals > 0 {
                    "dismissed"
                } else {
                    "not yet answered"
                };
                cols[1].label(format!("Last dialog result: {dialog_result}"));
                if self.torn_down {
                    cols[1].label("Accessibility mirror: torn down");
                }
                let (mirror_button, next_mirror_on) = if self.mirror_on {
                    ("Turn screen reader mirror off", false)
                } else {
                    ("Turn screen reader mirror on", true)
                };
                if cols[1].button(mirror_button).clicked() {
                    toggle_mirror_location(next_mirror_on);
                }
                if !self.mirror_on {
                    cols[1].label(
                        "Screen reader mirror is off: the page has no DOM overlay. \
                         Press the button again from the page with the mirror on.",
                    );
                }
            });

            if self.show_modal {
                let modal_id = ui.make_persistent_id("confirm_modal");
                egui::Modal::new(modal_id).show(ui.ctx(), |ui| {
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        self.show_modal = false;
                    }
                    let dialog_id = ui.response().id;
                    ui.ctx().accesskit_node_builder(dialog_id, |node| {
                        node.set_role(egui::accesskit::Role::AlertDialog);
                        node.set_label("Confirmation dialog");
                        node.set_modal();
                    });
                    ui.label("Confirm action?");
                    ui.horizontal(|ui| {
                        if ui.button("Close dialog").clicked() {
                            self.show_modal = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_modal = false;
                            self.dialog_dismissals += 1;
                        }
                        if ui.button("Confirm action").clicked() {
                            self.show_modal = false;
                            self.dialog_confirmed = true;
                        }
                    });
                });
            }
        });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "egui-web-accesskit-demo is browser-only (wasm32-unknown-unknown). \
         Run it with `trunk serve` or build it with `trunk build` in examples/demo."
    );
    std::process::exit(1);
}

/// Reloads the page with the bridge enabled or disabled. egui keeps one plugin per type, so a
/// mounted bridge cannot be re-added in place; a reload is the reliable way to switch it.
#[cfg(target_arch = "wasm32")]
fn toggle_mirror_location(mirror_on: bool) {
    if let Some(window) = web_sys::window() {
        let search = if mirror_on { "" } else { "?bridge=off" };
        let _ = window.location().set_search(search);
    }
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

        let search = window.location().search().unwrap_or_default();
        let shadow_mode = search.contains("shadow=1");
        let lazy_mode = search.contains("lazy=1");
        let teardown_mode = search.contains("teardown=1");
        let mirror_on = !search.contains("bridge=off");

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
                .attach_shadow(&web_sys::ShadowRootInit::new(web_sys::ShadowRootMode::Open))
                .expect("attach shadow root");
            let _ = canvas.set_attribute("style", "width: 100%; height: 100%; display: block;");
            shadow
                .append_child(&canvas)
                .expect("append canvas to shadow root");
            WebAccessKitPlugin::from_canvas(canvas.clone())
        } else if lazy_mode {
            WebAccessKitPlugin::new_lazy("the_canvas_id")
        } else {
            WebAccessKitPlugin::new("the_canvas_id")
        };

        let web_options = eframe::WebOptions::default();
        eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(move |cc| {
                    if mirror_on {
                        cc.egui_ctx.add_plugin(plugin);
                    }
                    let mut app = DemoApp {
                        mirror_on,
                        ..DemoApp::default()
                    };
                    if teardown_mode {
                        app.teardown_frames = Some(3);
                    }
                    Ok(Box::new(app))
                }),
            )
            .await
            .expect("failed to start eframe");
    });
}
