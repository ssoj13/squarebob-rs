use eframe::egui;

pub struct ProgressBar {
    width: f32,
    height: f32,
    current: usize,
    total: usize,
}

impl ProgressBar {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            current: 0,
            total: 0,
        }
    }

    pub fn set_progress(&mut self, current: usize, total: usize) {
        self.current = current;
        self.total = total;
    }

    pub fn render(&self, ui: &mut egui::Ui) -> egui::Response {
        let fraction = if self.total == 0 {
            0.0
        } else {
            self.current as f32 / self.total as f32
        };
        let text = if self.total == 0 {
            String::new()
        } else {
            format!("{}/{}", self.current, self.total)
        };
        ui.add_sized(
            [self.width, self.height],
            egui::ProgressBar::new(fraction.clamp(0.0, 1.0)).text(text),
        )
    }
}
