use desktop_pet::engine::Mood;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

pub const INK: Color32 = Color32::from_rgb(44, 48, 70);
pub const BLUE: Color32 = Color32::from_rgb(80, 100, 230);
pub const CORAL: Color32 = Color32::from_rgb(245, 143, 122);
pub const CREAM: Color32 = Color32::from_rgb(255, 248, 233);

pub fn limit_length(vector: Vec2, maximum: f32) -> Vec2 {
    if vector.length() > maximum {
        vector.normalized() * maximum
    } else {
        vector
    }
}

pub fn pet(painter: &Painter, origin: Pos2, scale: f32, mood: Mood, now: f64, gaze: Vec2) {
    let phase = now as f32;
    let lively = matches!(mood, Mood::Happy | Mood::Play | Mood::Walk | Mood::Chase);
    let bounce = if mood == Mood::Dizzy {
        -(phase * 7.0).sin().abs() * 6.0
    } else if lively {
        (phase * 8.0).sin() * 5.0
    } else if mood == Mood::Dragged {
        (phase * 15.0).sin() * 3.0
    } else {
        (phase * 2.0).sin() * 1.3
    };
    let position = |horizontal: f32, vertical: f32| {
        origin + Vec2::new(horizontal * scale, (vertical + bounce) * scale)
    };
    let outline = Stroke::new(2.8 * scale, INK);
    let ellipse = |center: Pos2, radius: Vec2, color: Color32, stroke: Stroke| {
        painter.add(egui::epaint::EllipseShape {
            center,
            radius: radius * scale,
            fill: color,
            stroke,
        });
    };
    ellipse(
        origin + Vec2::new(130.0, 222.0) * scale,
        Vec2::new(58.0, 7.0),
        Color32::from_black_alpha(24),
        Stroke::NONE,
    );
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [
            position(187.0, 194.0),
            position(225.0, 190.0),
            position(209.0, 150.0),
            position(197.0, 172.0),
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(9.0 * scale, INK),
    ));
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
        [
            position(187.0, 194.0),
            position(225.0, 190.0),
            position(209.0, 150.0),
            position(197.0, 172.0),
        ],
        false,
        Color32::TRANSPARENT,
        Stroke::new(4.0 * scale, CREAM),
    ));
    let foot_swing = if mood == Mood::Dragged {
        (phase * 20.0).sin() * 9.0
    } else if lively {
        (phase * 8.0).cos() * 5.0
    } else {
        0.0
    };
    ellipse(
        position(100.0, 213.0 + foot_swing),
        Vec2::new(17.0, 10.0),
        CREAM,
        outline,
    );
    ellipse(
        position(162.0, 213.0 - foot_swing),
        Vec2::new(17.0, 10.0),
        CREAM,
        outline,
    );
    painter.add(Shape::convex_polygon(
        vec![
            position(83.0, 131.0),
            position(82.0, 91.0),
            position(111.0, 117.0),
        ],
        CREAM,
        outline,
    ));
    painter.add(Shape::convex_polygon(
        vec![
            position(150.0, 115.0),
            position(179.0, 90.0),
            position(177.0, 132.0),
        ],
        CREAM,
        outline,
    ));
    painter.add(Shape::convex_polygon(
        vec![
            position(88.0, 117.0),
            position(88.0, 102.0),
            position(101.0, 116.0),
        ],
        CORAL,
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        vec![
            position(161.0, 115.0),
            position(173.0, 102.0),
            position(172.0, 119.0),
        ],
        BLUE,
        Stroke::NONE,
    ));
    let squash = if mood == Mood::Sleep {
        0.84
    } else if mood == Mood::Dragged {
        1.12
    } else {
        1.0
    };
    ellipse(
        position(130.0, 164.0),
        Vec2::new(64.0, 52.0 * squash),
        CREAM,
        outline,
    );
    ellipse(
        position(72.0, if mood == Mood::Dragged { 155.0 } else { 182.0 }),
        Vec2::new(11.0, 16.0),
        CREAM,
        outline,
    );
    ellipse(
        position(188.0, if mood == Mood::Happy { 148.0 } else { 182.0 }),
        Vec2::new(11.0, 16.0),
        CREAM,
        outline,
    );
    ellipse(
        position(91.0, 169.0),
        Vec2::new(11.0, 6.0),
        CORAL.gamma_multiply(0.7),
        Stroke::NONE,
    );
    ellipse(
        position(169.0, 169.0),
        Vec2::new(11.0, 6.0),
        CORAL.gamma_multiply(0.7),
        Stroke::NONE,
    );
    for horizontal in [108.0, 151.0] {
        if matches!(mood, Mood::Sleep | Mood::Happy) {
            let curve = if mood == Mood::Happy { -6.0 } else { 3.0 };
            painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                [
                    position(horizontal - 7.0, 153.0),
                    position(horizontal, 153.0 + curve),
                    position(horizontal + 7.0, 153.0),
                ],
                false,
                Color32::TRANSPARENT,
                Stroke::new(3.0 * scale, INK),
            ));
        } else if mood == Mood::Dizzy {
            painter.line_segment(
                [
                    position(horizontal - 5.0, 148.0),
                    position(horizontal + 5.0, 158.0),
                ],
                outline,
            );
            painter.line_segment(
                [
                    position(horizontal + 5.0, 148.0),
                    position(horizontal - 5.0, 158.0),
                ],
                outline,
            );
        } else {
            let look = limit_length(gaze, 1.0) * 3.0;
            ellipse(
                position(horizontal + look.x, 153.0 + look.y),
                Vec2::new(4.3, 6.0),
                INK,
                Stroke::NONE,
            );
            painter.circle_filled(
                position(horizontal + look.x + 1.2, 151.0 + look.y),
                1.2 * scale,
                Color32::WHITE,
            );
        }
    }
    if mood == Mood::Grumpy {
        painter.line_segment([position(101.0, 139.0), position(116.0, 144.0)], outline);
        painter.line_segment([position(145.0, 144.0), position(159.0, 139.0)], outline);
    }
    painter.circle_filled(position(130.0, 165.0), 3.0 * scale, CORAL);
    if mood == Mood::Sleep {
        ellipse(
            position(131.0, 178.0),
            Vec2::new(4.0, 5.0),
            INK,
            Stroke::NONE,
        );
        painter.text(
            position(183.0, 104.0),
            egui::Align2::CENTER_CENTER,
            "z Z",
            egui::FontId::proportional(20.0 * scale),
            BLUE,
        );
    } else {
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [
                position(121.0, 176.0),
                position(130.0, if mood == Mood::Grumpy { 168.0 } else { 186.0 }),
                position(139.0, 176.0),
            ],
            false,
            Color32::TRANSPARENT,
            Stroke::new(2.5 * scale, INK),
        ));
    }
    painter.add(Shape::convex_polygon(
        vec![
            position(118.0, 198.0),
            position(130.0, 206.0),
            position(142.0, 198.0),
        ],
        BLUE,
        Stroke::NONE,
    ));
    if matches!(mood, Mood::Play | Mood::Chase) {
        let ball_horizontal = 45.0 + (phase * 3.0).sin() * 18.0;
        painter.circle_filled(position(ball_horizontal, 209.0), 12.0 * scale, BLUE);
        painter.circle_filled(
            position(ball_horizontal - 3.0, 205.0),
            3.0 * scale,
            Color32::WHITE.gamma_multiply(0.8),
        );
    }
    if mood == Mood::Hiding {
        let bag = Rect::from_center_size(position(130.0, 195.0), Vec2::new(51.0, 31.0) * scale);
        painter.rect_filled(bag, 9.0 * scale, BLUE);
        painter.text(
            bag.center(),
            egui::Align2::CENTER_CENTER,
            "ICONS",
            egui::FontId::proportional(10.0 * scale),
            Color32::WHITE,
        );
    }
    if mood == Mood::Happy {
        painter.text(
            position(205.0, 121.0),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(24.0 * scale),
            CORAL,
        );
        painter.text(
            position(63.0, 110.0),
            egui::Align2::CENTER_CENTER,
            "+",
            egui::FontId::proportional(17.0 * scale),
            BLUE,
        );
    }
}

pub fn hit_test(local: Pos2, scale: f32) -> bool {
    let horizontal = (local.x / scale - 130.0) / 80.0;
    let vertical = (local.y / scale - 158.0) / 68.0;
    horizontal * horizontal + vertical * vertical < 1.0
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_corners_are_not_interactive() {
        assert!(!hit_test(Pos2::new(0.0, 0.0), 1.0));
        assert!(hit_test(Pos2::new(130.0, 160.0), 1.0));
        assert!(hit_test(Pos2::new(195.0, 240.0), 1.5));
        assert_eq!(tray_rgba().len(), 4096);
    }
}
