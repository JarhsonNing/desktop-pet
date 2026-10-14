use eframe::egui::{Color32, Vec2};

pub const INK: Color32 = Color32::from_rgb(44, 48, 70);
pub const BLUE: Color32 = Color32::from_rgb(80, 100, 230);

pub fn limit_length(vector: Vec2, maximum: f32) -> Vec2 {
    if vector.length() > maximum {
        vector.normalized() * maximum
    } else {
        vector
    }
}
pub fn tray_rgba() -> Vec<u8> {
    let mut pixels = vec![0_u8; 32 * 32 * 4];
    for vertical in 0..32 {
        for horizontal in 0..32 {
            let distance = (horizontal as f32 - 15.5).powi(2) + (vertical as f32 - 17.0).powi(2);
            let ear = (7..=11).contains(&horizontal) && (4..=12).contains(&vertical)
                || (20..=24).contains(&horizontal) && (4..=12).contains(&vertical);
            if distance < 155.0 || ear {
                let eye = (horizontal == 11 || horizontal == 20) && (14..=17).contains(&vertical);
                let color = if eye { INK } else { BLUE };
                let offset = (vertical * 32 + horizontal) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color.to_array());
            }
        }
    }
    pixels
}
