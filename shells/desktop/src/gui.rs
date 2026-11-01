//! The interactive emulator window.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant as StdInstant};

use paperpaw_platform::{
    ButtonState, InputEvent, Point, PointerEvent, PointerPhase, PointerTool, buttons,
};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::budget;
use crate::device::Device;
use crate::headless;
use crate::record::Recorder;
use crate::resize::Settle;

const BACKGROUND: u32 = 0x0030_3030;
/// Frame interval while an animation runs.
const ANIMATION_FRAME: Duration = Duration::from_millis(30);

/// Keyboard keys emulating device buttons.
const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::PageDown, buttons::PAGE_FORWARD),
    (KeyCode::Space, buttons::PAGE_FORWARD),
    (KeyCode::PageUp, buttons::PAGE_BACK),
    (KeyCode::ArrowUp, buttons::UP),
    (KeyCode::ArrowDown, buttons::DOWN),
    (KeyCode::ArrowLeft, buttons::LEFT),
    (KeyCode::ArrowRight, buttons::RIGHT),
    (KeyCode::Enter, buttons::CONFIRM),
    (KeyCode::Escape, buttons::BACK),
    (KeyCode::Backspace, buttons::BACK),
    (KeyCode::KeyM, buttons::MENU),
    (KeyCode::KeyH, buttons::HOME),
    (KeyCode::KeyP, buttons::POWER),
];

pub fn run(
    device: Device,
    screenshot_dir: PathBuf,
    recorder: Option<Recorder<std::fs::File>>,
    cable: bool,
    free_size: bool,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let mut app = App {
        device,
        screenshot_dir,
        recorder,
        cable: cable.then(crate::cable::lines),
        window: None,
        surface: None,
        cursor: PhysicalPosition::new(0.0, 0.0),
        pointer_down: false,
        animating: false,
        free_size: free_size.then(Settle::default),
        epoch: StdInstant::now(),
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct App {
    device: Device,
    screenshot_dir: PathBuf,
    /// Writes what is done as a flow's steps (`--record`).
    recorder: Option<Recorder<std::fs::File>>,
    /// The cable transfer's lines from standard input (`--cable`).
    cable: Option<std::sync::mpsc::Receiver<String>>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    cursor: PhysicalPosition<f64>,
    pointer_down: bool,
    /// Whether the last frame was part of an animation, so the frame after
    /// its end is drawn too.
    animating: bool,
    /// Where the window sets the size of the screen (`--free-size`): the
    /// size it has had for a moment is made (see `resize`).
    free_size: Option<Settle>,
    /// What the window's sizes are timed from.
    epoch: StdInstant,
    error: Option<String>,
}

/// Where the panel is drawn inside the window.
#[derive(Debug, Clone, Copy)]
struct Viewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            let screen = self
                .device
                .kernel
                .active_screen()
                .map(ToString::to_string)
                .unwrap_or_default();
            window.set_title(&format!(
                "PaperPaw Emulator | {} | {} | {}",
                self.device.profile.device.name,
                screen,
                budget::stats()
            ));
        }
    }

    fn viewport(&self, size: PhysicalSize<u32>) -> Viewport {
        let (pw, ph) = (
            u32::from(self.device.panel.width()),
            u32::from(self.device.panel.height()),
        );
        let scale = (size.width as f64 / pw as f64).min(size.height as f64 / ph as f64);
        let width = ((pw as f64 * scale) as u32).max(1);
        let height = ((ph as f64 * scale) as u32).max(1);
        Viewport {
            x: size.width.saturating_sub(width) / 2,
            y: size.height.saturating_sub(height) / 2,
            width,
            height,
        }
    }

    /// Converts a window position to logical panel coordinates.
    fn to_panel(&self, position: PhysicalPosition<f64>) -> Option<Point> {
        let window = self.window.as_ref()?;
        let v = self.viewport(window.inner_size());
        let x = position.x - f64::from(v.x);
        let y = position.y - f64::from(v.y);
        if x < 0.0 || y < 0.0 || x >= f64::from(v.width) || y >= f64::from(v.height) {
            return None;
        }
        let px = x * f64::from(self.device.panel.width()) / f64::from(v.width);
        let py = y * f64::from(self.device.panel.height()) / f64::from(v.height);
        Some(Point::new(px as i32, py as i32))
    }

    fn pointer(&mut self, phase: PointerPhase, point: Point) {
        if let Some(recorder) = &mut self.recorder {
            recorder.pointer(phase, point, &mut self.device);
        }
        self.device.input(InputEvent::Pointer(PointerEvent::new(
            0,
            phase,
            point,
            PointerTool::Finger,
            1000,
        )));
        self.after_input();
    }

    /// The window's size in the screen's pixels: logical, so that the
    /// screen is as large to the eye on a dense display as on another.
    fn window_size(&self) -> Option<(u32, u32)> {
        let window = self.window.as_ref()?;
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        Some((size.width.round() as u32, size.height.round() as u32))
    }

    fn millis(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// With `--free-size`: the window has another size; the screen follows
    /// once it has kept it for a moment.
    fn window_resized(&mut self) {
        let (now, size) = (self.millis(), self.window_size());
        if let (Some(settle), Some(size)) = (&mut self.free_size, size) {
            settle.asked(now, size);
        }
    }

    /// With `--free-size`: makes the screen the size the window has kept.
    fn follow_window(&mut self) {
        let now = self.millis();
        let Some(settle) = &mut self.free_size else {
            return;
        };
        if let Some((width, height)) = settle.due(now) {
            self.device.resize_to(width, height);
            self.after_input();
        }
    }

    fn after_input(&mut self) {
        self.update_title();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn redraw(&mut self) -> Result<(), String> {
        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        let v = self.viewport(size);
        let pixels = self.device.panel.compose();
        let (pw, ph) = (
            usize::from(self.device.panel.width()),
            usize::from(self.device.panel.height()),
        );
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        surface.resize(w, h).map_err(|e| e.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|e| e.to_string())?;
        buffer.fill(BACKGROUND);

        let columns: Vec<usize> = (0..v.width as usize)
            .map(|x| (x * pw / v.width as usize).min(pw - 1))
            .collect();
        let stride = size.width as usize;
        for y in 0..v.height as usize {
            let source_row = (y * ph / v.height as usize).min(ph - 1) * pw;
            let target_row = (y + v.y as usize) * stride + v.x as usize;
            for (x, &column) in columns.iter().enumerate() {
                buffer[target_row + x] = pixels[source_row + column];
            }
        }
        buffer.present().map_err(|e| e.to_string())
    }

    fn screenshot(&self) {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let path = self
            .screenshot_dir
            .join(format!("{}-{millis}.png", self.device.profile.device.id));
        match headless::screenshot(&self.device, &path) {
            Ok(()) => println!("screenshot saved to {}", path.display()),
            Err(error) => eprintln!("screenshot failed: {error}"),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (pw, ph) = (
            f64::from(self.device.panel.width()),
            f64::from(self.device.panel.height()),
        );
        // Start at a comfortable size; the window is freely resizable.
        let scale = (900.0 / ph).min(1.0);
        let attributes = Window::default_attributes()
            .with_title("PaperPaw Emulator")
            .with_inner_size(winit::dpi::LogicalSize::new(pw * scale, ph * scale));
        let result = event_loop
            .create_window(attributes)
            .map_err(|e| e.to_string())
            .and_then(|window| {
                let window = Rc::new(window);
                let context =
                    softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
                let surface = softbuffer::Surface::new(&context, window.clone())
                    .map_err(|e| e.to_string())?;
                Ok((window, surface))
            });
        match result {
            Ok((window, surface)) => {
                self.window = Some(window);
                self.surface = Some(surface);
                let size = (self.device.panel.width(), self.device.panel.height());
                if let Some(settle) = &mut self.free_size {
                    settle.is_now(size);
                }
                self.device.start();
                self.after_input();
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.device.shut_down();
                event_loop.exit();
            }
            WindowEvent::Resized(_) => {
                self.window_resized();
                self.after_input();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                let pressed = event.state == ElementState::Pressed;
                match code {
                    KeyCode::F2 if pressed => {
                        self.device.panel.show_regions = !self.device.panel.show_regions;
                    }
                    KeyCode::F3 if pressed => {
                        self.device.panel.animate = !self.device.panel.animate;
                    }
                    KeyCode::F12 if pressed => self.screenshot(),
                    _ => {
                        if event.repeat {
                            return;
                        }
                        let Some(&(_, name)) = KEYS
                            .iter()
                            .find(|(key, name)| *key == code && self.device.has_button(name))
                        else {
                            return;
                        };
                        let state = if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        };
                        if let Some(recorder) = &mut self.recorder {
                            recorder.button(name, pressed, &self.device);
                        }
                        self.device.input(InputEvent::Button { name, state });
                        self.after_input();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if self.pointer_down
                    && let Some(point) = self.to_panel(position)
                {
                    self.pointer(PointerPhase::Move, point);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if !self.device.profile.device.input.touch {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        if let Some(point) = self.to_panel(self.cursor) {
                            self.pointer_down = true;
                            self.pointer(PointerPhase::Down, point);
                        }
                    }
                    ElementState::Released if self.pointer_down => {
                        self.pointer_down = false;
                        match self.to_panel(self.cursor) {
                            Some(point) => self.pointer(PointerPhase::Up, point),
                            None => self.pointer(PointerPhase::Cancel, Point::new(0, 0)),
                        }
                    }
                    ElementState::Released => {}
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let before = self.device.next_wake();
        let in_flight = self.device.http.in_flight;
        if let Some(lines) = &self.cable {
            let lines: Vec<String> = lines.try_iter().collect();
            for line in &lines {
                self.device.cable_line(line);
            }
            if !lines.is_empty() {
                self.after_input();
            }
        }
        self.follow_window();
        self.device.tick_if_due();
        if self.cable.is_some() {
            crate::cable::send(&self.device.take_cable_lines());
        }
        if self.device.next_wake() != before || self.device.http.in_flight != in_flight {
            self.after_input();
        }

        let mut deadline: Option<StdInstant> = None;
        let animating = self.device.panel.is_animating();
        // Keep drawing while the panel animates, and once more when it has
        // finished, or the last flash phase (white) would stay visible.
        if (animating || self.animating)
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
        self.animating = animating;
        if animating {
            deadline = Some(StdInstant::now() + ANIMATION_FRAME);
        }
        // Answers to HTTP requests, servers' requests, what sockets bring
        // and lines on standard input are looked for regularly.
        if self.device.http.in_flight > 0
            || self.device.servers.any()
            || self.device.sockets.any()
            || self.cable.is_some()
        {
            let at = StdInstant::now() + Duration::from_millis(100);
            deadline = Some(deadline.map_or(at, |d| d.min(at)));
        }
        // A size the window has not kept long enough yet.
        let now = self.millis();
        if let Some(wait) = self.free_size.as_ref().and_then(|settle| settle.wait(now)) {
            let at = StdInstant::now() + Duration::from_millis(wait);
            deadline = Some(deadline.map_or(at, |d| d.min(at)));
        }
        if let Some(wake) = self.device.next_wake() {
            let wait = Duration::from_millis(wake.since(self.device.now()));
            let at = StdInstant::now() + wait;
            deadline = Some(deadline.map_or(at, |d| d.min(at)));
        }
        event_loop.set_control_flow(match deadline {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
