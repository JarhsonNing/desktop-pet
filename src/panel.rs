use crate::art;
use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Vec2};

pub const BACKGROUND: Color32 = Color32::from_rgb(244, 247, 252);
pub const MUTED: Color32 = Color32::from_rgb(112, 125, 149);
pub const BORDER: Color32 = Color32::from_rgb(228, 234, 244);
pub const SOFT_BLUE: Color32 = Color32::from_rgb(235, 239, 255);

pub fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(18.0)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(22.0)
        .outer_margin(2.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

pub fn heading(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).size(17.0).strong().color(art::INK));
    if !description.is_empty() {
        ui.label(RichText::new(description).size(13.0).color(MUTED));
    }
    ui.add_space(12.0);
}

pub fn badge(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(SOFT_BLUE)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(art::BLUE));
        });
}

pub fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE))
        .fill(art::BLUE)
        .stroke(Stroke::NONE)
        .corner_radius(10.0)
}

pub fn navigation(ui: &mut egui::Ui, index: usize, selected: bool, label: &str) -> egui::Response {
    let response = ui.add_sized(
        [ui.available_width(), 48.0],
        egui::Button::new(RichText::new(label).size(14.0).color(if selected {
            art::BLUE
        } else {
            MUTED
        }))
        .fill(if selected { SOFT_BLUE } else { Color32::WHITE })
        .stroke(Stroke::NONE)
        .corner_radius(11.0),
    );
    let painter = ui.painter();
    let center = Pos2::new(response.rect.left() + 21.0, response.rect.center().y);
    let color = if selected { art::BLUE } else { MUTED };
    let stroke = Stroke::new(1.6_f32, color);
    match index {
        0 => {
            for offset in [
                Vec2::new(-5.0, -4.0),
                Vec2::new(0.0, -6.0),
                Vec2::new(5.0, -4.0),
            ] {
                painter.circle_filled(center + offset, 2.0, color);
            }
            painter.circle_filled(center + Vec2::new(0.0, 3.0), 4.0, color);
        }
        1 => {
            for (y, x) in [(-5.0, 2.0), (0.0, -3.0), (5.0, 3.0)] {
                painter.line_segment(
                    [center + Vec2::new(-7.0, y), center + Vec2::new(7.0, y)],
                    stroke,
                );
                painter.circle_filled(center + Vec2::new(x, y), 2.5, color);
            }
        }
        2 => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, Vec2::new(13.0, 16.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            for y in [-3.0, 1.0, 5.0] {
                painter.line_segment(
                    [center + Vec2::new(-3.0, y), center + Vec2::new(3.0, y)],
                    stroke,
                );
            }
        }
        3 => {
            let points = [
                (-3.0, -6.0),
                (-7.0, -4.0),
                (-9.0, 0.0),
                (-5.0, 2.0),
                (-4.0, 0.0),
                (-4.0, 7.0),
                (4.0, 7.0),
                (4.0, 0.0),
                (5.0, 2.0),
                (9.0, 0.0),
                (7.0, -4.0),
                (3.0, -6.0),
                (2.0, -3.0),
                (-2.0, -3.0),
                (-3.0, -6.0),
            ];
            painter.add(egui::Shape::line(
                points
                    .into_iter()
                    .map(|(x, y)| center + Vec2::new(x, y))
                    .collect(),
                stroke,
            ));
        }
        4 => {
            let rect = egui::Rect::from_center_size(center, Vec2::new(13.0, 16.0));
            painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(
                egui::Rect::from_center_size(center + Vec2::new(0.0, -7.0), Vec2::new(7.0, 4.0)),
                1.0,
                color,
            );
            painter.line_segment(
                [center + Vec2::new(-3.0, 0.0), center + Vec2::new(3.0, 0.0)],
                stroke,
            );
            painter.line_segment(
                [center + Vec2::new(-3.0, 4.0), center + Vec2::new(2.0, 4.0)],
                stroke,
            );
        }
        _ => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center + Vec2::new(0.0, 3.0), Vec2::new(17.0, 10.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [center + Vec2::new(0.0, -8.0), center + Vec2::new(0.0, 2.0)],
                stroke,
            );
            painter.line_segment(
                [center + Vec2::new(-4.0, -2.0), center + Vec2::new(0.0, 2.0)],
                stroke,
            );
            painter.line_segment(
                [center + Vec2::new(4.0, -2.0), center + Vec2::new(0.0, 2.0)],
                stroke,
            );
        }
    }
    response
}

pub fn toggle(ui: &mut egui::Ui, value: &mut bool, title: &str, description: &str) -> bool {
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new((width - 62.0).max(100.0), 44.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width((width - 62.0).max(100.0));
                ui.label(RichText::new(title).size(14.0));
                ui.label(RichText::new(description).size(12.0).color(MUTED));
            },
        );
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(42.0, 24.0), egui::Sense::click());
        if response.clicked() {
            *value = !*value;
            response.mark_changed();
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title)
        });
        ui.painter().rect_filled(
            rect,
            12.0,
            if *value {
                art::BLUE
            } else {
                Color32::from_rgb(218, 225, 237)
            },
        );
        let center = Pos2::new(
            if *value {
                rect.right() - 12.0
            } else {
                rect.left() + 12.0
            },
            rect.center().y,
        );
        ui.painter().circle_filled(center, 9.0, Color32::WHITE);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(3.0),
                14.0,
                Stroke::new(1.0_f32, art::BLUE),
                egui::StrokeKind::Outside,
            );
        }
        response.changed()
    })
    .inner
}
