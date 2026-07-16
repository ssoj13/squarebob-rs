pub mod dialogs;
pub mod frame;
pub mod io;
pub mod progress;
pub mod source;

pub use dialogs::encode::*;
pub use frame::*;
pub use source::*;

pub fn add_icon_font(fonts: &mut egui::FontDefinitions) {
    egui_phosphor::add_to_fonts(fonts, egui_phosphor::Variant::Regular);
}
