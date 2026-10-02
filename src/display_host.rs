//! Native window owner. egui paints into the float canvas; egui-display alone
//! encodes that canvas for the selected monitor signal.
use eframe::{Storage, egui};
use egui_display::screenshot::{Capture, Pixels};
use egui_display::{CANVAS_FORMAT, DisplayState, Output, PresentPass, Target};
use egui_wgpu::RenderState;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

type Error = Box<dyn std::error::Error>;
pub(crate) struct CreationContext<'a> {
    pub egui_ctx: egui::Context,
    pub storage: Option<&'a dyn Storage>,
    pub wgpu_render_state: Option<RenderState>,
}
pub(crate) trait NativeApp: eframe::App {
    fn ui_native(&mut self, ui: &mut egui::Ui);
}
type Creator<A> = Box<dyn FnOnce(&CreationContext<'_>) -> Result<A, Error>>;
fn prefs_id() -> egui::Id {
    egui::Id::new("squarebob.display_prefs")
}
pub(crate) fn settings_ui(ui: &mut egui::Ui) {
    let state = ui
        .ctx()
        .data(|d| d.get_temp::<DisplayState>(egui_display::state_id()));
    let mut prefs = ui
        .ctx()
        .data(|d| d.get_temp::<egui_display::DisplayPrefs>(prefs_id()))
        .unwrap_or_default();
    if egui_display::settings_ui(ui, &mut prefs, state.as_ref()) {
        ui.ctx().data_mut(|d| d.insert_temp(prefs_id(), prefs));
        ui.ctx().request_repaint();
    }
}

/// Keep eframe's storage keys and RON encoding, including its existing files
/// whose trailing commas make them look like invalid JSON.
pub(crate) struct SessionStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
}
impl SessionStorage {
    pub(crate) fn open(path: PathBuf) -> Result<Self, Error> {
        let values = match std::fs::read_to_string(&path) {
            Ok(text) => ron::from_str(&text)
                .map_err(|e| format!("Cannot read settings {}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, values })
    }
}
impl Storage for SessionStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.values.insert(key.into(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.values.remove(key);
    }
    fn flush(&mut self) {
        let result = (|| -> Result<(), Error> {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = ron::ser::to_string_pretty(&self.values, ron::ser::PrettyConfig::default())?;
            let temp = self.path.with_extension("json.tmp");
            std::fs::write(&temp, text)?;
            std::fs::rename(&temp, &self.path)?;
            Ok(())
        })();
        if let Err(e) = result {
            log::error!("Cannot persist settings {}: {e}", self.path.display());
        }
    }
}

enum Wake {
    AccessKit(egui_winit::accesskit_winit::Event),
    Repaint(Instant),
    Screenshot(Vec<egui::UserData>, Arc<egui::ColorImage>),
}
impl From<egui_winit::accesskit_winit::Event> for Wake {
    fn from(event: egui_winit::accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

pub(crate) fn run<A: NativeApp + 'static>(
    options: eframe::NativeOptions,
    creator: Creator<A>,
) -> Result<(), Error> {
    let event_loop = EventLoop::<Wake>::with_user_event().build()?;
    let storage = SessionStorage::open(
        options
            .persistence_path
            .clone()
            .ok_or("Missing settings path")?,
    )?;
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(true);
    if let Some(memory) = eframe::get_value::<egui::Memory>(&storage, "egui") {
        ctx.memory_mut(|m| *m = memory);
    }
    let proxy = event_loop.create_proxy();
    let repaint_proxy = proxy.clone();
    ctx.set_request_repaint_callback(move |request| {
        if let Some(deadline) = Instant::now().checked_add(request.delay) {
            let _ = repaint_proxy.send_event(Wake::Repaint(deadline));
        }
    });
    let prefs = storage
        .get_string("display_output")
        .and_then(|text| serde_json::from_str::<egui_display::DisplayPrefs>(&text).ok())
        .unwrap_or(egui_display::DisplayPrefs {
            output: Output::Hdr10,
            white: None,
            peak: None,
        });
    ctx.data_mut(|d| d.insert_temp(prefs_id(), prefs));
    let mut host = Host {
        options,
        creator: Some(creator),
        storage,
        ctx,
        proxy,
        live: None,
        error: None,
        next_repaint: None,
        last_save: Instant::now(),
        close_requested: false,
    };
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.error {
        return Err(error.into());
    }
    Ok(())
}
struct Host<A: NativeApp> {
    options: eframe::NativeOptions,
    creator: Option<Creator<A>>,
    storage: SessionStorage,
    ctx: egui::Context,
    proxy: EventLoopProxy<Wake>,
    live: Option<Live<A>>,
    error: Option<String>,
    next_repaint: Option<Instant>,
    last_save: Instant,
    close_requested: bool,
}
struct Live<A: NativeApp> {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    rs: RenderState,
    input: egui_winit::State,
    viewport: egui::ViewportInfo,
    app: A,
    present: PresentPass,
    state: DisplayState,
    ctx: egui::Context,
    refresh_display: bool,
    // Cache only the last negotiation attempt; app.settings owns the user's choice.
    requested_output: Output,
    configure: bool,
    pending_events: Vec<egui::Event>,
    screenshot_requests: Vec<egui::UserData>,
}
impl<A: NativeApp> Host<A> {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Error> {
        let mut builder = self.options.viewport.clone();
        let mut saved = eframe::get_value::<egui_winit::WindowSettings>(&self.storage, "window");
        let largest = event_loop
            .available_monitors()
            .map(|m| {
                let s = m
                    .size()
                    .to_logical::<f32>(m.scale_factor() * self.ctx.zoom_factor() as f64);
                egui::vec2(s.width, s.height)
            })
            .fold(egui::vec2(64.0, 64.0), |a, b| a.max(b));
        if let Some(s) = saved.as_mut() {
            s.clamp_size_to_sane_values(largest);
            s.clamp_position_to_monitors(self.ctx.zoom_factor(), event_loop);
            builder = s.initialize_viewport_builder(self.ctx.zoom_factor(), event_loop, builder);
        } else if let Some(size) = builder.inner_size {
            builder = builder.with_inner_size(size.min(largest));
        }
        // AccessKit must attach before a native window is shown for the first time.
        let visible = builder.visible.unwrap_or(true);
        builder = builder.with_visible(false);
        let window = Arc::new(egui_winit::create_window(&self.ctx, event_loop, &builder)?);
        if let Some(s) = saved {
            s.initialize_window(&window);
        }
        let instance = pollster::block_on(self.options.wgpu_options.wgpu_setup.new_instance());
        let surface = instance.create_surface(window.clone())?;
        let mut rs = pollster::block_on(RenderState::create(
            &self.options.wgpu_options,
            &instance,
            Some(&surface),
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        ))?;
        rs.target_format = CANVAS_FORMAT;
        rs.renderer = Arc::new(egui::mutex::RwLock::new(egui_wgpu::Renderer::new(
            &rs.device,
            CANVAS_FORMAT,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        )));
        let mut input = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(rs.device.limits().max_texture_dimension_2d as usize),
        );
        input.init_accesskit(event_loop, &window, self.proxy.clone());
        let app = self
            .creator
            .take()
            .ok_or("Application was already initialized")?(&CreationContext {
            egui_ctx: self.ctx.clone(),
            storage: Some(&self.storage),
            wgpu_render_state: Some(rs.clone()),
        })?;
        let size = window.inner_size();
        let config = surface
            .get_default_config(&rs.adapter, size.width.max(1), size.height.max(1))
            .ok_or("Shared GPU cannot present to this window")?;
        let present = PresentPass::new(&rs.device, config.format, Output::Sdr8);
        let state = DisplayState {
            output: Output::Sdr8,
            target: Target::default(),
            info: surface.display_hdr_info(&rs.adapter),
            error: None,
            available: Vec::new(),
        };
        let mut viewport = egui::ViewportInfo::default();
        egui_winit::update_viewport_info(&mut viewport, &self.ctx, &window, true);
        self.live = Some(Live {
            window,
            surface,
            config,
            rs,
            input,
            viewport,
            app,
            present,
            state,
            ctx: self.ctx.clone(),
            refresh_display: true,
            requested_output: Output::Sdr8,
            configure: true,
            pending_events: Vec::new(),
            screenshot_requests: Vec::new(),
        });
        let window = &self.live.as_ref().unwrap().window;
        window.set_visible(visible);
        window.request_redraw();
        Ok(())
    }
    fn save(&mut self) {
        if let Some(live) = self.live.as_mut() {
            live.app.save(&mut self.storage);
            if self.options.persist_window && !live.window.is_minimized().unwrap_or(false) {
                eframe::set_value(
                    &mut self.storage,
                    "window",
                    &egui_winit::WindowSettings::from_window(self.ctx.zoom_factor(), &live.window),
                );
            }
            if live.app.persist_egui_memory() {
                self.ctx
                    .memory(|m| eframe::set_value(&mut self.storage, "egui", m));
            }
            if let Some(prefs) = self
                .ctx
                .data(|d| d.get_temp::<egui_display::DisplayPrefs>(prefs_id()))
            {
                // DisplayPrefs accepts unknown enum names through a JSON Value decoder.
                // Keep its inner value JSON; the outer eframe storage file remains RON.
                if let Ok(json) = serde_json::to_string(&prefs) {
                    self.storage.set_string("display_output", json);
                }
            }
            self.storage.flush();
            self.last_save = Instant::now();
        }
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Error> {
        let Some(live) = self.live.as_mut() else {
            return Ok(());
        };
        live.refresh_output()?;
        self.ctx
            .data_mut(|d| d.insert_temp(egui_display::state_id(), live.state.clone()));
        egui_winit::update_viewport_info(&mut live.viewport, &self.ctx, &live.window, false);
        let mut input = live.input.take_egui_input(&live.window);
        input.events.append(&mut live.pending_events);
        if self.close_requested {
            live.viewport.events.push(egui::ViewportEvent::Close);
        }
        input
            .viewports
            .insert(egui::ViewportId::ROOT, live.viewport.clone());
        live.app.raw_input_hook(&self.ctx, &mut input);
        let output = self.ctx.run_ui(input, |ui| live.app.ui_native(ui));
        // The viewer backdrop the viewport published this frame (none when it showed no
        // picture); the present pass composites the canvas over it.
        live.present.backdrop = egui_display::take_backdrop(&self.ctx);
        live.viewport.events.clear();
        live.input.handle_platform_output_with_event_loop(
            &live.window,
            event_loop,
            output.platform_output,
        );
        let mut actions = Vec::new();
        if let Some(viewport_output) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            let mut close = self.close_requested;
            for command in &viewport_output.commands {
                match command {
                    egui::ViewportCommand::Close => close = true,
                    egui::ViewportCommand::CancelClose => close = false,
                    _ => {}
                }
            }
            self.close_requested = close;
            egui_winit::process_viewport_commands(
                &self.ctx,
                &mut live.viewport,
                viewport_output.commands.clone(),
                &live.window,
                &mut actions,
            );
            if let Some(deadline) = Instant::now().checked_add(viewport_output.repaint_delay) {
                self.next_repaint = Some(self.next_repaint.map_or(deadline, |v| v.min(deadline)));
            }
        }
        for action in actions {
            match action {
                egui_winit::ActionRequested::Screenshot(data) => {
                    live.screenshot_requests.push(data)
                }
                egui_winit::ActionRequested::Copy => live.pending_events.push(egui::Event::Copy),
                egui_winit::ActionRequested::Cut => live.pending_events.push(egui::Event::Cut),
                egui_winit::ActionRequested::Paste => {
                    if let Some(text) = live.input.clipboard_text() {
                        let text = text.replace("\r\n", "\n");
                        if !text.is_empty() {
                            live.pending_events.push(egui::Event::Paste(text));
                        }
                    }
                }
            }
        }
        let jobs = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        live.paint(
            &jobs,
            output.pixels_per_point,
            output.textures_delta,
            &self.proxy,
        )?;
        if !live.pending_events.is_empty() {
            live.window.request_redraw();
        }
        if self.close_requested {
            self.save();
            event_loop.exit();
        }
        Ok(())
    }
}
impl<A: NativeApp> Live<A> {
    fn refresh_output(&mut self) -> Result<(), Error> {
        let prefs = self
            .ctx
            .data(|d| d.get_temp::<egui_display::DisplayPrefs>(prefs_id()))
            .unwrap_or_default();
        if self.refresh_display || prefs.output != self.requested_output {
            let caps = self.surface.get_capabilities(&self.rs.adapter);
            self.state.available = Output::ALL
                .into_iter()
                .filter(|o| o.surface(&caps).is_some())
                .collect();
            let (output, pair, error) = match prefs.output.surface(&caps) {
                Some(pair) => (prefs.output, pair, None),
                None => (
                    Output::Sdr8,
                    Output::Sdr8
                        .surface(&caps)
                        .ok_or("Window has no SDR surface format")?,
                    Some(format!(
                        "{} is unavailable on this display; showing SDR 8-bit.",
                        prefs.output.label()
                    )),
                ),
            };
            if self.state.error != error
                && let Some(message) = &error
            {
                log::error!("{message}");
            }
            self.state.error = error;
            if self.config.format != pair.0
                || self.config.color_space != pair.1
                || self.state.output != output
            {
                self.config.format = pair.0;
                self.config.color_space = pair.1;
                let mut present = PresentPass::new(&self.rs.device, pair.0, output);
                present.inherit_canvas(&self.rs.device, &mut self.present);
                self.present = present;
                self.configure = true;
            }
            if self.state.output != output || self.refresh_display {
                log::info!(
                    "Display output: {} ({:?}, {:?})",
                    output.label(),
                    pair.0,
                    pair.1
                );
            }
            self.state.output = output;
            self.state.info = self.surface.display_hdr_info(&self.rs.adapter);
            self.refresh_display = false;
            self.requested_output = prefs.output;
        }
        self.state.target = prefs.target(self.state.output, &self.state.info);
        let size = self.window.inner_size();
        if size.width > 0
            && size.height > 0
            && (self.configure
                || self.config.width != size.width
                || self.config.height != size.height)
        {
            self.config.width = size.width;
            self.config.height = size.height;
            self.config.present_mode = self.rs.surface_config.present_mode;
            self.config.desired_maximum_frame_latency = self
                .rs
                .surface_config
                .desired_maximum_frame_latency
                .unwrap_or(2);
            self.surface.configure(&self.rs.device, &self.config);
            self.configure = false;
        }
        Ok(())
    }
    fn paint(
        &mut self,
        jobs: &[egui::ClippedPrimitive],
        pixels_per_point: f32,
        mut textures: egui::TexturesDelta,
        proxy: &EventLoopProxy<Wake>,
    ) -> Result<(), Error> {
        let rs = &self.rs;
        {
            let mut renderer = rs.renderer.write();
            for (id, deltas) in textures.set.drain() {
                for delta in deltas {
                    renderer.update_texture(&rs.device, &rs.queue, id, &delta);
                }
            }
        }
        let result = self.paint_frame(jobs, pixels_per_point, proxy);
        // Free even when minimized, occluded, or acquisition failed.
        for id in textures.free.drain() {
            self.rs.renderer.write().free_texture(&id);
        }
        result
    }
    fn paint_frame(
        &mut self,
        jobs: &[egui::ClippedPrimitive],
        pixels_per_point: f32,
        proxy: &EventLoopProxy<Wake>,
    ) -> Result<(), Error> {
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 || self.window.is_minimized().unwrap_or(false) {
            return Ok(());
        }
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => (f, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => (f, true),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure = true;
                self.refresh_display = true;
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.surface = self.rs.instance.create_surface(self.window.clone())?;
                self.configure = true;
                self.refresh_display = true;
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("Window surface validation failed".into());
            }
        };
        let rs = &self.rs;
        self.present
            .prepare_canvas(&rs.device, &rs.queue, size.width, size.height);
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("squarebob.window"),
            });
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
            pixels_per_point,
        };
        let mut renderer = rs.renderer.write();
        let commands = renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, jobs, &screen);
        {
            let canvas = self.present.canvas(&rs.device, size.width, size.height);
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("squarebob.egui.float"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: canvas,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear({
                                let [r, g, b, a] =
                                    self.app.clear_color(&self.ctx.global_style().visuals);
                                wgpu::Color {
                                    r: r as f64,
                                    g: g as f64,
                                    b: b as f64,
                                    a: a as f64,
                                }
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            renderer.render(&mut pass, jobs, &screen);
        }
        drop(renderer);
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.present
            .draw(&rs.queue, &mut encoder, &view, self.state.target);
        // Submit canvas updates before presenting on the same shared queue.
        rs.queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        rs.queue.present(frame);
        if !self.screenshot_requests.is_empty() {
            self.capture_window(proxy)?;
        }
        if suboptimal {
            self.configure = true;
            self.refresh_display = true;
            self.window.request_redraw();
        }
        Ok(())
    }
    /// The frame just presented, encoded for `output` at `target`'s levels and read back on
    /// a worker thread: egui-display's one capture path (`PresentPass::capture`, the same shader
    /// and parameters as the swapchain draw, backdrop included). `deliver` runs on the worker.
    fn capture(
        &self,
        output: Output,
        target: Target,
        what: &'static str,
        deliver: impl FnOnce(Capture) + Send + 'static,
    ) -> Result<(), Error> {
        let rs = &self.rs;
        let pending = self
            .present
            .capture(&rs.device, &rs.queue, output, target)?;
        let device = rs.device.clone();
        std::thread::Builder::new()
            .name("squarebob-window-capture".into())
            .spawn(move || match pending.wait(&device) {
                Ok(capture) => deliver(capture),
                Err(error) => log::error!("{what}: readback failed: {error}"),
            })?;
        Ok(())
    }

    /// The internal SDR window screenshot (egui `ViewportCommand::Screenshot`: UI tests, the
    /// REST API): always 8-bit sRGB, whatever the window shows.
    fn capture_window(&mut self, proxy: &EventLoopProxy<Wake>) -> Result<(), Error> {
        let proxy = proxy.clone();
        let requests = std::mem::take(&mut self.screenshot_requests);
        self.capture(
            Output::Sdr8,
            Target::default(),
            "Screenshot",
            move |capture| {
                let Pixels::Rgba8(rgba) = capture.pixels else {
                    log::error!("Screenshot: an SDR capture came back without 8-bit pixels");
                    return;
                };
                let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied(
                    [capture.width as usize, capture.height as usize],
                    &rgba,
                ));
                let _ = proxy.send_event(Wake::Screenshot(requests, image));
            },
        )
    }
}

impl<A: NativeApp> ApplicationHandler<Wake> for Host<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_none()
            && let Err(error) = self.initialize(event_loop)
        {
            self.error = Some(error.to_string());
            event_loop.exit();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if id != live.window.id() {
            return;
        }
        let response = live.input.on_window_event(&live.window, &event);
        match event {
            WindowEvent::RedrawRequested => {
                self.next_repaint = None;
                if let Err(error) = self.redraw(event_loop) {
                    self.error = Some(error.to_string());
                    event_loop.exit();
                }
            }
            WindowEvent::CloseRequested => {
                self.close_requested = true;
                live.window.request_redraw();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                live.configure = true;
                live.refresh_display = true;
                live.window.request_redraw();
                // Windows runs a modal sizing loop: queued redraws can wait until
                // the drag ends. Paint here as eframe's native integration did.
                if cfg!(target_os = "windows")
                    && let Err(error) = self.redraw(event_loop)
                {
                    self.error = Some(error.to_string());
                    event_loop.exit();
                }
            }
            WindowEvent::Moved(_) | WindowEvent::Focused(_) => {
                live.refresh_display = true;
                live.window.request_redraw();
            }
            WindowEvent::Occluded(false) => {
                live.window.request_redraw();
            }
            _ => {
                if response.repaint {
                    live.window.request_redraw();
                }
            }
        }
    }
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: Wake) {
        match event {
            Wake::AccessKit(event) => {
                if let Some(live) = self.live.as_mut()
                    && event.window_id == live.window.id()
                {
                    match event.window_event {
                        egui_winit::accesskit_winit::WindowEvent::InitialTreeRequested => {
                            self.ctx.enable_accesskit();
                            live.window.request_redraw();
                        }
                        egui_winit::accesskit_winit::WindowEvent::ActionRequested(request) => {
                            live.input.on_accesskit_action_request(request);
                            live.window.request_redraw();
                        }
                        egui_winit::accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                            self.ctx.disable_accesskit()
                        }
                    }
                }
            }
            Wake::Repaint(deadline) => {
                self.next_repaint = Some(self.next_repaint.map_or(deadline, |v| v.min(deadline)));
                if deadline <= Instant::now()
                    && let Some(live) = &self.live
                {
                    live.window.request_redraw();
                }
            }
            Wake::Screenshot(requests, image) => {
                if let Some(live) = self.live.as_mut() {
                    for user_data in requests {
                        live.pending_events.push(egui::Event::Screenshot {
                            viewport_id: egui::ViewportId::ROOT,
                            user_data,
                            image: image.clone(),
                        });
                    }
                    live.window.request_redraw();
                }
            }
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let save_at = self.last_save
            + self.live.as_ref().map_or(Duration::from_secs(30), |live| {
                live.app.auto_save_interval()
            });
        let now = Instant::now();
        if now >= save_at {
            self.save();
        }
        if self.next_repaint.is_some_and(|deadline| deadline <= now) {
            if let Some(live) = &self.live {
                live.window.request_redraw();
            }
            self.next_repaint = None;
        }
        let deadline = self.next_repaint.map_or(
            self.last_save
                + self.live.as_ref().map_or(Duration::from_secs(30), |live| {
                    live.app.auto_save_interval()
                }),
            |v| {
                v.min(
                    self.last_save
                        + self.live.as_ref().map_or(Duration::from_secs(30), |live| {
                            live.app.auto_save_interval()
                        }),
                )
            },
        );
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.save();
        if let Some(live) = self.live.as_mut() {
            live.app.on_exit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_and_present_pass_share_the_float_canvas_format() {
        assert_eq!(
            render_core::DISPLAY_TEXTURE_FORMAT,
            egui_display::CANVAS_FORMAT
        );
    }

    #[test]
    fn session_preserves_existing_eframe_keys_and_display_preferences() {
        let dir = std::env::temp_dir().join(format!("squarebob-display-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.ron");
        std::fs::write(
            &path,
            "{\"squarebob_state\": \"existing app JSON\",\"egui\": \"memory\",}",
        )
        .unwrap();
        let mut storage = SessionStorage::open(path.clone()).unwrap();
        assert_eq!(
            storage.get_string("squarebob_state").as_deref(),
            Some("existing app JSON")
        );
        let prefs = egui_display::DisplayPrefs {
            output: Output::Hdr10,
            white: Some(240.0),
            peak: None,
        };
        storage.set_string("display_output", serde_json::to_string(&prefs).unwrap());
        storage.flush();
        // Replacement of an existing RON file must work on Windows too.
        storage.set_string("window", "window geometry".into());
        storage.flush();
        let restored = SessionStorage::open(path.clone()).unwrap();
        assert_eq!(
            restored.get_string("squarebob_state").as_deref(),
            Some("existing app JSON")
        );
        assert_eq!(restored.get_string("egui").as_deref(), Some("memory"));
        assert_eq!(
            restored
                .get_string("display_output")
                .and_then(|text| serde_json::from_str::<egui_display::DisplayPrefs>(&text).ok()),
            Some(prefs)
        );
        assert_eq!(
            restored.get_string("window").as_deref(),
            Some("window geometry")
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn corrupt_session_is_reported_without_overwriting_it() {
        let path =
            std::env::temp_dir().join(format!("squarebob-display-{}.ron", uuid::Uuid::new_v4()));
        std::fs::write(&path, "not RON").unwrap();
        assert!(SessionStorage::open(path.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not RON");
        std::fs::remove_file(path).unwrap();
    }
}
