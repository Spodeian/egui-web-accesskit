//! Drop-in [`egui::Plugin`] that mirrors `egui`'s AccessKit tree to an HTML/ARIA DOM overlay on `wasm32`.
//!
//! `eframe` on the web does not wire AccessKit updates to browser DOM elements on its own.
//! Registering [`WebAccessKitPlugin`] once during setup bridges `egui`'s AccessKit output to
//! screen readers and assistive technologies without modifying `egui` or `eframe`, while becoming
//! a clean no-op on native desktop targets.
//!
//! # Version Compatibility
//!
//! Each branch/release matches `egui`'s minor version:
//!
//! | `egui` / `eframe` | `egui-web-accesskit` | Branch |
//! | --- | --- | --- |
//! | `0.37` | `0.37` | `main` |
//! | `0.36` | `0.36` | `egui-0.36` |
//! | `0.35` | `0.35` | `egui-0.35` |
//!
//! # Setup with `eframe`
//!
//! ```ignore
//! eframe::WebRunner::new().start(
//!     canvas,
//!     web_options,
//!     Box::new(|cc| {
//!         cc.egui_ctx
//!             .add_plugin(egui_web_accesskit::WebAccessKitPlugin::new("the_canvas_id"));
//!         Ok(Box::new(MyApp::default()))
//!     }),
//! );
//! ```
//!
//! # Shadow DOM & Manual Teardown
//!
//! - When `<canvas>` lives inside a `ShadowRoot`, construct the plugin with
//!   `WebAccessKitPlugin::from_canvas(canvas)` on `wasm32`.
//! - When tearing down an `egui::Context` outside `eframe::WebRunner::destroy` (or on earlier
//!   `egui` versions without `Plugin::on_exit`), call [`WebAccessKitPlugin::teardown`] to detach
//!   all DOM listeners and break the repaint-wakeup reference cycle.

#![warn(missing_docs)]

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
    /// Defer AccessKit tree construction and DOM mirroring until a screen reader
    /// user interacts with the activation control or [`WebAccessKitPlugin::activate`]
    /// is called. Defaults to `false` (eager mirroring).
    pub lazy: bool,
}

impl WebAccessKitConfig {
    /// Creates a [`WebAccessKitConfig`] targeting the DOM element with `canvas_id`.
    pub fn new(canvas_id: impl Into<String>) -> Self {
        Self {
            canvas_id: canvas_id.into(),
            lazy: false,
        }
    }

    /// Sets whether AccessKit tree construction is deferred until first AT interaction
    /// or an explicit [`WebAccessKitPlugin::activate`] call.
    pub fn with_lazy(mut self, lazy: bool) -> Self {
        self.lazy = lazy;
        self
    }
}

/// Drop-in [`egui::Plugin`] that mirrors `egui`'s AccessKit tree to the browser DOM on `wasm32`.
///
/// On non-`wasm32` targets this plugin is a no-op so native desktop builds continue using
/// `eframe` / `egui-winit`'s built-in platform adapters unchanged.
///
/// # Example
///
/// ```
/// use egui_web_accesskit::WebAccessKitPlugin;
///
/// let ctx = egui::Context::default();
/// ctx.add_plugin(WebAccessKitPlugin::new("the_canvas_id"));
///
/// let _output = ctx.run_ui(egui::RawInput::default(), |ui| {
///     let _ = ui.button("OK");
/// });
/// WebAccessKitPlugin::teardown(&ctx);
/// ```
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

    /// Creates a lazily activated plugin targeting the `<canvas>` with `canvas_id`.
    ///
    /// No AccessKit updates or semantic DOM nodes are built until an assistive-tech
    /// user activates the fallback control or [`Self::activate`] is called.
    pub fn new_lazy(canvas_id: impl Into<String>) -> Self {
        Self::with_config(WebAccessKitConfig::new(canvas_id).with_lazy(true))
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

    /// Creates a plugin bound directly to a `<canvas>` element (works inside a Shadow DOM).
    #[cfg(target_arch = "wasm32")]
    pub fn from_canvas(canvas: web_sys::HtmlCanvasElement) -> Self {
        let plugin = Self::new(canvas.id());
        dom::stash_pending_canvas(plugin.bridge_id, canvas);
        plugin
    }

    /// Programmatically enables AccessKit mirroring when configured with `lazy: true`.
    ///
    /// Safe to call repeatedly or when already active.
    pub fn activate(ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if let Some(plugin) = ctx.plugin_opt::<Self>() {
            let bridge_id = plugin.lock().bridge_id;
            dom::activate_bridge(bridge_id);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;
    }

    /// Tears down the DOM bridge and breaks the `Context` repaint-wakeup reference cycle.
    ///
    /// Useful when unmounting an `egui::Context` outside `eframe`'s `destroy()` lifecycle.
    pub fn teardown(ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if let Some(plugin) = ctx.plugin_opt::<Self>() {
            let bridge_id = plugin.lock().bridge_id;
            dom::remove_bridge(bridge_id);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;
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
        if !self.config.lazy {
            ctx.enable_accesskit();
        }

        let activate_ctx = ctx.clone();
        let activate = Box::new(move || {
            activate_ctx.enable_accesskit();
            activate_ctx.request_repaint();
        });
        let wakeup_ctx = ctx.clone();
        let wakeup = Box::new(move || wakeup_ctx.request_repaint());
        if let Some(bridge) = dom::WebDomBridge::mount(
            self.bridge_id,
            &self.config.canvas_id,
            self.config.lazy,
            Arc::clone(&self.queue),
            wakeup,
            activate,
        ) {
            dom::insert_bridge(self.bridge_id, bridge);
            dom::install_viewport_listeners(self.bridge_id);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if dom::has_dom_focus(self.bridge_id) {
            input.focused = true;
            input
                .events
                .retain(|event| !matches!(event, egui::Event::WindowFocused(false)));
        }
        dom::drain_pending_events(self.bridge_id, ctx, &self.queue, &mut input.events);
    }

    #[cfg(target_arch = "wasm32")]
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if output.platform_output.requested_discard()
            && output.platform_output.num_completed_passes < ctx.options(|o| o.max_passes.get())
        {
            return;
        }
        let Some(update) = output.platform_output.accesskit_update.take() else {
            return;
        };
        dom::with_bridge_mut(self.bridge_id, |bridge| {
            bridge.apply_update(update);
        });
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
        WebAccessKitPlugin::teardown(&ctx);
    }
}
