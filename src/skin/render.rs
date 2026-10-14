use super::{Scalar, Shape, Skin};
use crate::engine::Mood;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Stroke, TextureHandle, Vec2};
use std::{collections::BTreeMap, sync::Arc};

pub struct SkinRenderer {
    pub skin: Arc<Skin>,
    textures: BTreeMap<String, TextureHandle>,
}

fn state(mood: Mood) -> &'static str {
    match mood {
        Mood::Idle => "idle",
        Mood::Happy => "happy",
        Mood::Grumpy => "grumpy",
        Mood::Sleep => "sleep",
        Mood::Dragged => "dragged",
        Mood::Play => "play",
        Mood::Chase => "chase",
        Mood::Hiding => "hiding",
        Mood::Dizzy => "dizzy",
        Mood::Walk => "walk",
    }
}

fn point(point: &[Scalar; 2], time: f64, gaze: Vec2) -> Vec2 {
    Vec2::new(
        point[0].value(time, [gaze.x, gaze.y]),
        point[1].value(time, [gaze.x, gaze.y]),
    )
}

fn normalized(gaze: Vec2) -> Vec2 {
    if gaze.length_sq() > 1.0 {
        gaze.normalized()
    } else {
        gaze
    }
}

fn frame(frames: &[String], fps: f32, time: f64) -> &str {
    &frames[((time.max(0.0) * f64::from(fps)).floor() as u64 % frames.len() as u64) as usize]
}

impl SkinRenderer {
    pub fn new(skin: Arc<Skin>, context: &egui::Context) -> Self {
        let textures = skin
            .images
            .iter()
            .map(|(name, image)| {
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                );
                (
                    name.clone(),
                    context.load_texture(
                        format!("skin/{}/{name}", skin.manifest.id),
                        color,
                        egui::TextureOptions::LINEAR,
                    ),
                )
            })
            .collect();
        Self { skin, textures }
    }

    pub fn paint(
        &self,
        painter: &Painter,
        origin: Pos2,
        scale: f32,
        mood: Mood,
        time: f64,
        gaze: Vec2,
    ) {
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(Rect::from_min_size(
            origin,
            Vec2::from(self.skin.ui.canvas) * scale,
        )));
        let gaze = normalized(gaze);
        let state = state(mood);
        let scene = &self
            .skin
            .ui
            .states
            .get(state)
            .unwrap_or(&self.skin.ui.states["idle"])
            .offset;
        for layer in &self.skin.ui.layers {
            let node = layer.states.get(state).unwrap_or(&layer.node);
            if !node.visible {
                continue;
            }
            let offset = point(&node.offset, time, gaze)
                + if node.follow_scene {
                    point(scene, time, gaze)
                } else {
                    Vec2::ZERO
                };
            let pos = |value: &[Scalar; 2]| origin + (point(value, time, gaze) + offset) * scale;
            let fill = node
                .fill
                .as_ref()
                .map_or(Color32::TRANSPARENT, |color| self.skin.color(color));
            let stroke = node.stroke.as_ref().map_or(Stroke::NONE, |stroke| {
                Stroke::new(stroke.width * scale, self.skin.color(&stroke.color))
            });
            match &node.shape {
                Shape::Ellipse { center, radius } => {
                    painter.add(egui::epaint::EllipseShape {
                        center: pos(center),
                        radius: point(radius, time, gaze) * scale,
                        fill,
                        stroke,
                    });
                }
                Shape::Polygon { points } => {
                    painter.add(egui::Shape::convex_polygon(
                        points.iter().map(pos).collect(),
                        fill,
                        stroke,
                    ));
                }
                Shape::Line { points } => {
                    painter.line_segment([pos(&points[0]), pos(&points[1])], stroke);
                }
                Shape::Quadratic { points } => {
                    painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
                        points.each_ref().map(pos),
                        false,
                        Color32::TRANSPARENT,
                        stroke,
                    ));
                }
                Shape::Cubic { points } => {
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                        points.each_ref().map(pos),
                        false,
                        Color32::TRANSPARENT,
                        stroke,
                    ));
                }
                Shape::Rect {
                    origin,
                    size,
                    radius,
                } => {
                    let rect = Rect::from_min_size(pos(origin), point(size, time, gaze) * scale);
                    painter.rect_filled(rect, radius * scale, fill);
                    painter.rect_stroke(rect, radius * scale, stroke, egui::StrokeKind::Inside);
                }
                Shape::Text { center, text, size } => {
                    painter.text(
                        pos(center),
                        egui::Align2::CENTER_CENTER,
                        text,
                        egui::FontId::proportional(size * scale),
                        fill,
                    );
                }
                Shape::Image {
                    origin,
                    size,
                    frames,
                    fps,
                } => {
                    let texture = &self.textures[frame(frames, *fps, time)];
                    painter.image(
                        texture.id(),
                        Rect::from_min_size(pos(origin), point(size, time, gaze) * scale),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            }
        }
    }
}

impl Skin {
    pub fn animation_interval(&self, mood: Mood) -> Option<std::time::Duration> {
        let state = state(mood);
        self.ui
            .layers
            .iter()
            .filter_map(|layer| {
                let node = layer.states.get(state).unwrap_or(&layer.node);
                match &node.shape {
                    Shape::Image { frames, fps, .. }
                        if node.visible && frames.iter().any(|frame| frame != &frames[0]) =>
                    {
                        Some(*fps)
                    }
                    _ => None,
                }
            })
            .max_by(f32::total_cmp)
            .map(|fps| std::time::Duration::from_secs_f64(1.0 / f64::from(fps)))
    }

    pub fn hit_test(
        &self,
        local: [f32; 2],
        scale: f32,
        mood: Mood,
        time: f64,
        gaze: [f32; 2],
    ) -> bool {
        if !scale.is_finite()
            || scale <= 0.0
            || !time.is_finite()
            || gaze.iter().any(|value| !value.is_finite())
            || local.iter().any(|value| !value.is_finite())
        {
            return false;
        }
        let gaze = normalized(Vec2::from(gaze));
        let local = Vec2::from(local) / scale;
        if local.x < 0.0
            || local.y < 0.0
            || local.x > self.ui.canvas[0]
            || local.y > self.ui.canvas[1]
        {
            return false;
        }
        let state = state(mood);
        let scene = &self
            .ui
            .states
            .get(state)
            .unwrap_or(&self.ui.states["idle"])
            .offset;
        self.ui.layers.iter().any(|layer| {
            let node = layer.states.get(state).unwrap_or(&layer.node);
            if !node.visible || !node.interactive {
                return false;
            }
            let local = local
                - point(&node.offset, time, gaze)
                - if node.follow_scene {
                    point(scene, time, gaze)
                } else {
                    Vec2::ZERO
                };
            let filled = node
                .fill
                .as_ref()
                .is_some_and(|color| self.color(color).a() >= 16);
            let outlined = node
                .stroke
                .as_ref()
                .is_some_and(|stroke| stroke.width > 0.0 && self.color(&stroke.color).a() >= 16);
            let width = if outlined {
                node.stroke.as_ref().unwrap().width / 2.0
            } else {
                0.0
            } + 0.5;
            if !filled && !outlined && !matches!(node.shape, Shape::Image { .. }) {
                return false;
            }
            if !outlined
                && matches!(
                    node.shape,
                    Shape::Line { .. } | Shape::Quadratic { .. } | Shape::Cubic { .. }
                )
            {
                return false;
            }
            match &node.shape {
                Shape::Ellipse { center, radius } => {
                    let delta = local - point(center, time, gaze);
                    let radius = point(radius, time, gaze) + Vec2::splat(width);
                    let outer = (delta.x / radius.x).powi(2) + (delta.y / radius.y).powi(2) <= 1.0;
                    let inner = (radius - Vec2::splat(width * 2.0)).max(Vec2::splat(0.001));
                    outer
                        && (filled
                            || outlined
                                && (delta.x / inner.x).powi(2) + (delta.y / inner.y).powi(2) >= 1.0)
                }
                Shape::Polygon { points } => {
                    let points: Vec<Vec2> = points
                        .iter()
                        .map(|value| point(value, time, gaze))
                        .collect();
                    let mut sign = 0.0_f32;
                    let inside = points
                        .iter()
                        .zip(points.iter().cycle().skip(1))
                        .take(points.len())
                        .all(|(start, end)| {
                            let edge = *end - *start;
                            let offset = local - *start;
                            let cross = edge.x * offset.y - edge.y * offset.x;
                            if cross.abs() < 0.001 {
                                true
                            } else if sign == 0.0 {
                                sign = cross.signum();
                                true
                            } else {
                                cross.signum() == sign
                            }
                        })
                        && sign != 0.0;
                    filled && inside
                        || outlined
                            && points
                                .iter()
                                .zip(points.iter().cycle().skip(1))
                                .take(points.len())
                                .any(|(start, end)| segment_hit(local, *start, *end, width))
                }
                Shape::Line { points } => segment_hit(
                    local,
                    point(&points[0], time, gaze),
                    point(&points[1], time, gaze),
                    width,
                ),
                Shape::Quadratic { points } => curve_hit(
                    local,
                    points
                        .each_ref()
                        .map(|value| point(value, time, gaze))
                        .as_slice(),
                    width,
                ),
                Shape::Cubic { points } => curve_hit(
                    local,
                    points
                        .each_ref()
                        .map(|value| point(value, time, gaze))
                        .as_slice(),
                    width,
                ),
                Shape::Rect {
                    origin,
                    size,
                    radius,
                } => {
                    let delta = local - point(origin, time, gaze);
                    let size = point(size, time, gaze);
                    let stroke = node.stroke.as_ref().map_or(0.0, |stroke| stroke.width);
                    rounded_hit(delta, size, *radius)
                        && (filled
                            || outlined
                                && !rounded_hit(
                                    delta - Vec2::splat(stroke),
                                    size - Vec2::splat(stroke * 2.0),
                                    (*radius - stroke).max(0.0),
                                ))
                }
                Shape::Image {
                    origin,
                    size,
                    frames,
                    fps,
                } => {
                    let uv = (local - point(origin, time, gaze)) / point(size, time, gaze);
                    if uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0 {
                        return false;
                    }
                    let image = &self.images[frame(frames, *fps, time)];
                    image
                        .get_pixel(
                            (uv.x * image.width() as f32) as u32,
                            (uv.y * image.height() as f32) as u32,
                        )
                        .0[3]
                        >= 16
                }
                Shape::Text { .. } => false,
            }
        })
    }

    pub fn recovery_hit(&self, local: [f32; 2], scale: f32) -> bool {
        let bubble = &self.ui.bubble;
        let delta = Vec2::from(local) / scale - Vec2::new(bubble.rect[0], bubble.rect[1]);
        rounded_hit(
            delta,
            Vec2::new(bubble.rect[2], bubble.hidden_height),
            bubble.radius,
        )
    }
}

fn rounded_hit(point: Vec2, size: Vec2, radius: f32) -> bool {
    if point.x < 0.0 || point.y < 0.0 || point.x > size.x || point.y > size.y {
        return false;
    }
    let radius = radius.min(size.x / 2.0).min(size.y / 2.0);
    let nearest = Vec2::new(
        point.x.clamp(radius, size.x - radius),
        point.y.clamp(radius, size.y - radius),
    );
    (point - nearest).length_sq() <= radius * radius
}

fn segment_hit(point: Vec2, start: Vec2, end: Vec2, width: f32) -> bool {
    let segment = end - start;
    let along = if segment.length_sq() > 0.0 {
        ((point - start).dot(segment) / segment.length_sq()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (point - start - segment * along).length_sq() <= width * width
}

fn curve_hit(point: Vec2, controls: &[Vec2], width: f32) -> bool {
    let mut previous = controls[0];
    for step in 1..=24 {
        let fraction = step as f32 / 24.0;
        let mut values = [Vec2::ZERO; 4];
        values[..controls.len()].copy_from_slice(controls);
        for count in (1..controls.len()).rev() {
            for index in 0..count {
                values[index] = values[index] * (1.0 - fraction) + values[index + 1] * fraction;
            }
        }
        let current = values[0];
        if segment_hit(point, previous, current, width) {
            return true;
        }
        previous = current;
    }
    false
}
