pub mod mapper;

#[cfg(target_arch = "wasm32")]
mod dom;

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};

pub use mapper::{DomNodeSpec, DomOp, DomTag, TreeBridge};

static NEXT_BRIDGE_ID: AtomicU64 = AtomicU64::new(1);

/// Configuration for [`WebAccessKitPlugin`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebAccessKitConfig {
    /// HTML `id` of the `<canvas>` hosting the `egui` surface.
    /// When empty (or not found), the first `<canvas>` in the document is used.
    pub canvas_id: String,
}

impl WebAccessKitConfig {
    pub fn new(canvas_id: impl Into<String>) -> Self {
        Self {
            canvas_id: canvas_id.into(),
        }
    }
}

/// Drop-in [`egui::Plugin`] that mirrors `egui`'s AccessKit tree to the browser DOM on `wasm32`.
///
/// On non-`wasm32` targets this plugin is a no-op so native desktop builds continue using
/// `eframe` / `egui-winit`'s built-in platform adapters unchanged.
pub struct WebAccessKitPlugin {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    bridge_id: u64,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    config: WebAccessKitConfig,
    #[cfg(target_arch = "wasm32")]
    queue: dom::ActionQueue,
}

impl WebAccessKitPlugin {
    /// Creates a plugin targeting the `<canvas>` with `canvas_id`.
    pub fn new(canvas_id: impl Into<String>) -> Self {
        Self::with_config(WebAccessKitConfig::new(canvas_id))
    }

    /// Creates a plugin from an explicit [`WebAccessKitConfig`].
    pub fn with_config(config: WebAccessKitConfig) -> Self {
        Self {
            bridge_id: NEXT_BRIDGE_ID.fetch_add(1, Ordering::Relaxed),
            config,
            #[cfg(target_arch = "wasm32")]
            queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the plugin configuration.
    pub fn config(&self) -> &WebAccessKitConfig {
        &self.config
    }
}

impl Default for WebAccessKitPlugin {
    fn default() -> Self {
        Self::with_config(WebAccessKitConfig::default())
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for WebAccessKitPlugin {
    fn drop(&mut self) {
        dom::remove_bridge(self.bridge_id);
    }
}

impl egui::Plugin for WebAccessKitPlugin {
    fn debug_name(&self) -> &'static str {
        "egui_web_accesskit"
    }

    #[cfg(target_arch = "wasm32")]
    fn setup(&mut self, ctx: &egui::Context) {
        ctx.enable_accesskit();

        let wakeup_ctx = ctx.clone();
        let wakeup = Box::new(move || wakeup_ctx.request_repaint());
        if let Some(bridge) =
            dom::WebDomBridge::mount(&self.config.canvas_id, Arc::clone(&self.queue), wakeup)
        {
            dom::insert_bridge(self.bridge_id, bridge);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Ok(mut pending) = self.queue.lock()
            && !pending.is_empty()
        {
            input.events.extend(pending.drain(..));
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let Some(update) = output.platform_output.accesskit_update.take() else {
            return;
        };
        let pixels_per_point = ctx.pixels_per_point();
        dom::with_bridge_mut(self.bridge_id, |bridge| {
            bridge.apply_update(update, pixels_per_point);
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn on_exit(&mut self, _ctx: &egui::Context) {
        dom::remove_bridge(self.bridge_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_registers_and_runs_pass() {
        let ctx = egui::Context::default();
        ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));

        let mut clicked = false;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            clicked = ui.button("OK").clicked();
        });

        assert!(!clicked);
        assert!(!output.shapes.is_empty());
        output.drop_without_applying_deltas();
    }
}
