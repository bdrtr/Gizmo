#[cfg(feature = "egui")]
use crate::dev_console;
#[cfg(feature = "egui")]
use crate::egui_ctx::EguiContext;
use gizmo_core::system::Schedule;
use gizmo_core::world::World;
use gizmo_renderer::renderer::Renderer;
use gizmo_renderer::RenderContext;
use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, Event, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{WindowAttributes, WindowId},
};

// setup_panic_hook ve Plugin lib.rs ve plugin.rs'ye taşındı.

// Frame timing clock: `std::time::Instant` panics on wasm32-unknown-unknown,
// so the browser build swaps in `web_time::Instant` (same API, backed by
// `performance.now()`). Native stays on `std` — zero behavior change.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant as FrameInstant;
#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::Instant as FrameInstant;

/// WASM: `resumed` içinde başlatılan async GPU init'in el-değiştirme durumu.
/// Tek-thread'li web ortamında `Rc<RefCell<…>>` yeterli; `spawn_local` future'ı
/// renderer'ı slota bırakıp `request_redraw` ile event loop'u uyandırır.
#[cfg(target_arch = "wasm32")]
struct PendingWebInit {
    window: Arc<winit::window::Window>,
    renderer_slot: std::rc::Rc<std::cell::RefCell<Option<Renderer>>>,
}

/// Plugin that registers the default renderer asset collections
/// (`Assets<Mesh>` and `Assets<Material>`) into the [`App`]'s world.
///
/// Added automatically by [`App::new`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AssetPlugin;

impl<State: 'static> super::Plugin<State> for AssetPlugin {
    fn build(&self, app: &mut App<State>) {
        app.world
            .insert_resource(gizmo_core::asset::Assets::<gizmo_renderer::components::Mesh>::new());
        app.world.insert_resource(gizmo_core::asset::Assets::<
            gizmo_renderer::components::Material,
        >::new());
    }
}

/// The windowed application builder and runtime.
///
/// `App` owns the ECS [`World`] and [`Schedule`], collects user-provided
/// lifecycle hooks (setup, update, render, input, UI) through its builder
/// methods, and finally drives the main event/render loop in [`App::run`].
///
/// Typical usage chains the builder methods and ends with [`App::run`]:
/// the conventional order is `new` -> `set_setup` -> `set_update` ->
/// (`set_render` / `set_simple_render` / `set_ui`) -> `run`. The render hooks
/// require the `render` feature; the `set_ui` overlay hook requires the `egui`
/// feature (also pulled in by `editor`).
///
/// This windowed variant is exported when the `window` feature is enabled.
/// With the feature disabled, a different, headless `App` type is exported
/// from the `headless` module instead.
pub struct App<State: 'static = ()> {
    /// The ECS world holding all entities, components and resources.
    pub world: World,
    /// The system schedule executed every (fixed) simulation step.
    pub schedule: Schedule,
    window_title: String,
    window_size: (u32, u32),

    setup_fn: Option<Box<dyn FnOnce(&mut World, &Renderer) -> State + 'static>>,
    update_fn: Option<Box<dyn FnMut(&mut World, &mut State, f32, &gizmo_core::input::Input)>>, // dt, input
    render_fn: Option<
        Box<
            dyn FnMut(
                &mut World,
                &State,
                &mut wgpu::CommandEncoder,
                &wgpu::TextureView,
                &mut Renderer,
                f32,
            ),
        >,
    >, // light_time
    simple_render_fn: Option<Box<dyn for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>)>>,
    input_fn: Option<Box<dyn FnMut(&mut World, &mut State, &winit::event::Event<()>) -> bool>>, // Input handler
    #[cfg(feature = "egui")]
    ui_fn: Option<Box<dyn FnMut(&mut World, &mut State, &egui::Context)>>, // Overlay UI handler
    /// Current keyboard/mouse input state, updated from window events.
    pub input: gizmo_core::input::Input,
    #[allow(clippy::type_complexity)]
    event_updaters: Vec<Box<dyn FnMut(&mut World)>>,
    initial_scene: Option<String>,
    window_icon: Option<&'static [u8]>,
    /// When `true`, per-frame input is recorded and saved on exit.
    pub record_mode: bool,
    /// Optional path to an input recording to replay instead of live input.
    pub playback_file: Option<String>,
    record_data: Option<gizmo_core::input::PlaybackData>,
    playback_data: Option<gizmo_core::input::PlaybackData>,
    playback_frame_index: usize,
    runner: Option<Box<dyn FnOnce(App<State>)>>,
    embedded_assets: std::collections::HashMap<String, std::borrow::Cow<'static, [u8]>>,

    // ── winit 0.30 `ApplicationHandler` runtime state ──
    // The window + GPU/editor/user-state are created lazily in `resumed` (the
    // window is only available from `&ActiveEventLoop` there), then driven by the
    // `window_event`/`about_to_wait`/`device_event` handlers. All `None`/default
    // until the first `resumed`.
    window_attributes: Option<WindowAttributes>,
    window: Option<Arc<winit::window::Window>>,
    #[cfg(feature = "egui")]
    editor: Option<EguiContext>,
    app_state: Option<State>,
    /// WASM: `resumed` bloklayamaz — `Renderer::new` (async WebGPU init)
    /// `spawn_local`'da koşar, sonucu bu slot üzerinden ilk uyanışta
    /// `finish_initialize`'a teslim edilir.
    #[cfg(target_arch = "wasm32")]
    pending_web_init: Option<PendingWebInit>,
    last_frame_time: Option<FrameInstant>,
    light_time: f32,
    /// Set by `resumed` when lazy initialization (renderer build / setup hook)
    /// fails, so `run` can propagate the `AppError` instead of returning `Ok(())`
    /// after the event loop exits (`resumed` itself returns `()`).
    init_error: Option<crate::AppError>,
}

impl<State: 'static> std::fmt::Debug for App<State> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ds = f.debug_struct("App");
        ds.field("window_title", &self.window_title)
            .field("window_size", &self.window_size)
            .field("setup_fn", &self.setup_fn.as_ref().map(|_| "<closure>"))
            .field("update_fn", &self.update_fn.as_ref().map(|_| "<closure>"))
            .field("render_fn", &self.render_fn.as_ref().map(|_| "<closure>"))
            .field(
                "simple_render_fn",
                &self.simple_render_fn.as_ref().map(|_| "<closure>"),
            )
            .field("input_fn", &self.input_fn.as_ref().map(|_| "<closure>"));
        #[cfg(feature = "egui")]
        ds.field("ui_fn", &self.ui_fn.as_ref().map(|_| "<closure>"));
        ds.field("event_updaters", &self.event_updaters.len())
            .field("initial_scene", &self.initial_scene)
            .field("window_icon", &self.window_icon.map(|b| b.len()))
            .field("record_mode", &self.record_mode)
            .field("playback_file", &self.playback_file)
            .field("playback_frame_index", &self.playback_frame_index)
            .field("runner", &self.runner.as_ref().map(|_| "<closure>"))
            .field("embedded_assets", &self.embedded_assets.keys())
            .finish_non_exhaustive()
    }
}

mod builder;
mod lifecycle;
mod event;
