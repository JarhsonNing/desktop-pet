use desktop_pet::{engine::Mood, platform, skin::Skin};
use eframe::egui::{self, Pos2};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone)]
pub struct Probe {
    pub skin: Arc<Skin>,
    pub position: Pos2,
    pub scale: f32,
    pub native_scale: f32,
    pub reference_height: f32,
    pub enabled: bool,
    pub recovery: bool,
    pub mood: Mood,
    pub elapsed: f64,
    pub gaze: [f32; 2],
    pub extras: Vec<egui::Rect>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            skin: Skin::builtin(),
            position: Pos2::ZERO,
            scale: 1.0,
            native_scale: 1.0,
            reference_height: 0.0,
            enabled: false,
            recovery: false,
            mood: Mood::Idle,
            elapsed: 0.0,
            gaze: [0.0; 2],
            extras: vec![],
        }
    }
}

impl Probe {
    fn hit(&self, position: [f32; 2]) -> bool {
        if !self.enabled {
            return false;
        }
        let local = Pos2::new(position[0], position[1]) - self.position.to_vec2();
        self.extras.iter().any(|rect| rect.contains(local))
            || crate::native_content::mouse_pressed() && {
                let bounds = self.skin.ui.interaction_bounds;
                egui::Rect::from_min_size(
                    Pos2::new(bounds[0], bounds[1]) * self.scale,
                    egui::Vec2::new(bounds[2], bounds[3]) * self.scale,
                )
                .contains(local)
            }
            || self.skin.hit_test(
                [local.x, local.y],
                self.scale,
                self.mood,
                self.elapsed,
                self.gaze,
            )
            || self.recovery && self.skin.recovery_hit([local.x, local.y], self.scale)
    }
}

pub struct HoverSampler {
    probe: Arc<Mutex<Probe>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub receiver: Receiver<()>,
}

impl HoverSampler {
    pub fn new(context: egui::Context) -> Self {
        let probe = Arc::new(Mutex::new(Probe::default()));
        let running = Arc::new(AtomicBool::new(true));
        let (sender, receiver) = mpsc::channel();
        let shared = probe.clone();
        let alive = running.clone();
        let handle = thread::spawn(move || {
            let mut previous = false;
            while alive.load(Ordering::Acquire) {
                let state = match shared.lock() {
                    Ok(state) => state.clone(),
                    Err(_) => break,
                };
                if state.enabled
                    && let Some(position) =
                        platform::cursor_position(state.native_scale, state.reference_height)
                {
                    let hovered = state.hit(position);
                    if hovered != previous {
                        previous = hovered;
                        if sender.send(()).is_err() {
                            break;
                        }
                        context.request_repaint_of(egui::ViewportId::ROOT);
                    }
                }
                thread::sleep(Duration::from_millis(75));
            }
        });
        Self {
            probe,
            running,
            handle: Some(handle),
            receiver,
        }
    }
    pub fn publish(&self, state: Probe) {
        if let Ok(mut current) = self.probe.lock() {
            *current = state;
        }
    }
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for HoverSampler {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_probes_respect_global_position_scale_and_recovery() {
        let mut probe = Probe {
            position: Pos2::new(-500.0, 100.0),
            scale: 1.5,
            enabled: true,
            ..Probe::default()
        };
        assert!(probe.hit([-305.0, 340.0]));
        assert!(!probe.hit([-500.0, 100.0]));
        assert!(!probe.hit([-470.0, 145.0]));
        probe.recovery = true;
        assert!(probe.hit([-470.0, 145.0]));
        probe.enabled = false;
        assert!(!probe.hit([-305.0, 340.0]));
    }
}
