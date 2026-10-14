mod definition;
mod package;
mod render;

pub use definition::*;
pub use package::{load, pack, unpack, validate_path};
pub use render::SkinRenderer;

use eframe::egui::Color32;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, OnceLock},
};

pub struct Skin {
    pub manifest: Manifest,
    pub ui: PetUi,
    pub(crate) images: BTreeMap<String, image::RgbaImage>,
    colors: BTreeMap<String, Color32>,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn parse_color(text: &str) -> io::Result<Color32> {
    let hex = text
        .strip_prefix('#')
        .ok_or_else(|| invalid("Color must use #RRGGBB or #RRGGBBAA"))?;
    if !matches!(hex.len(), 6 | 8) || !hex.is_ascii() {
        return Err(invalid("Color must use #RRGGBB or #RRGGBBAA"));
    }
    let mut channels = [255; 4];
    for (index, channel) in channels.iter_mut().take(hex.len() / 2).enumerate() {
        *channel = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| invalid("Invalid color"))?;
    }
    Ok(Color32::from_rgba_unmultiplied(
        channels[0],
        channels[1],
        channels[2],
        channels[3],
    ))
}

impl Skin {
    pub fn builtin() -> Arc<Self> {
        static SKIN: OnceLock<Arc<Skin>> = OnceLock::new();
        SKIN.get_or_init(|| {
            Arc::new(
                Self::from_files(BTreeMap::from([
                    (
                        "manifest.json".into(),
                        include_bytes!("../../skins/default/manifest.json").to_vec(),
                    ),
                    (
                        "pet.json".into(),
                        include_bytes!("../../skins/default/pet.json").to_vec(),
                    ),
                ]))
                .expect("Built-in skin must be valid"),
            )
        })
        .clone()
    }

    pub fn color(&self, reference: &str) -> Color32 {
        self.colors
            .get(reference)
            .copied()
            .unwrap_or(Color32::TRANSPARENT)
    }

    pub(crate) fn from_files(files: BTreeMap<String, Vec<u8>>) -> io::Result<Self> {
        let manifest: Manifest = serde_json::from_slice(
            files
                .get("manifest.json")
                .ok_or_else(|| invalid("Missing manifest.json"))?,
        )
        .map_err(|error| invalid(format!("manifest.json: {error}")))?;
        if manifest.format != "mochi.skin" || manifest.version != 1 {
            return Err(invalid("Unsupported skin format or version"));
        }
        if manifest.id.is_empty()
            || manifest.id.len() > 64
            || !manifest
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || manifest.name.trim().is_empty()
            || manifest.name.chars().count() > 64
            || manifest.author.chars().count() > 128
        {
            return Err(invalid("Invalid skin metadata"));
        }
        validate_path(&manifest.ui)?;
        let ui: PetUi = serde_json::from_slice(
            files
                .get(&manifest.ui)
                .ok_or_else(|| invalid("Missing UI definition"))?,
        )
        .map_err(|error| invalid(format!("{}: {error}", manifest.ui)))?;
        let mut skin = Self {
            manifest,
            ui,
            images: BTreeMap::new(),
            colors: BTreeMap::new(),
        };
        let assets = skin.validate()?;
        let mut pixels = 0_u64;
        for asset in assets {
            let bytes = files
                .get(&asset)
                .ok_or_else(|| invalid(format!("Missing image: {asset}")))?;
            let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes))
                .map_err(|error| invalid(error.to_string()))?;
            let (width, height) = image::ImageDecoder::dimensions(&decoder);
            pixels += u64::from(width) * u64::from(height);
            if width == 0
                || height == 0
                || width > 2048
                || height > 2048
                || pixels > 12 * 1024 * 1024
            {
                return Err(invalid(
                    "Skin images exceed the 2048px / 12M total pixel limit",
                ));
            }
            let image = image::DynamicImage::from_decoder(decoder)
                .map_err(|error| invalid(error.to_string()))?
                .into_rgba8();
            skin.images.insert(asset, image);
        }
        Ok(skin)
    }

    fn validate(&mut self) -> io::Result<BTreeSet<String>> {
        let ui = &self.ui;
        if ui
            .canvas
            .iter()
            .any(|value| !value.is_finite() || !(64.0..=1024.0).contains(value))
            || !ui.gaze_distance.is_finite()
            || !(1.0..=4096.0).contains(&ui.gaze_distance)
            || ui.layers.is_empty()
            || ui.layers.len() > 128
            || !ui.states.contains_key("idle")
            || ui
                .states
                .keys()
                .any(|state| !STATES.contains(&state.as_str()))
        {
            return Err(invalid("Invalid canvas, states, or layer count"));
        }
        rect_inside(ui.interaction_bounds, ui.canvas)?;
        if ui.interaction_bounds[2] < 24.0 || ui.interaction_bounds[3] < 24.0 {
            return Err(invalid("Interaction bounds must be at least 24 by 24"));
        }
        for point in [ui.gaze_origin, ui.chase_anchor] {
            if point
                .iter()
                .zip(ui.canvas)
                .any(|(value, limit)| !value.is_finite() || *value < 0.0 || *value > limit)
            {
                return Err(invalid("Pet anchors must be inside the canvas"));
            }
        }
        if ui.palette.len() > 64 {
            return Err(invalid("Too many palette colors"));
        }
        for (name, value) in &ui.palette {
            if name.is_empty()
                || name.len() > 32
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(invalid("Invalid palette color name"));
            }
            self.colors.insert(name.clone(), parse_color(value)?);
        }
        let bubble = &ui.bubble;
        rect_inside(bubble.rect, ui.canvas)?;
        rect_inside(
            [
                bubble.rect[0],
                bubble.rect[1],
                bubble.rect[2],
                bubble.hidden_height,
            ],
            ui.canvas,
        )?;
        rect_inside(
            bubble.restore_button,
            [bubble.rect[2], bubble.hidden_height],
        )?;
        if bubble.restore_button[2] < 60.0
            || bubble.restore_button[3] < 18.0
            || !bubble.font_size.is_finite()
            || !(4.0..=64.0).contains(&bubble.font_size)
            || !bubble.radius.is_finite()
            || !(0.0..=128.0).contains(&bubble.radius)
            || bubble
                .text_inset
                .iter()
                .zip([bubble.rect[2], bubble.rect[3]])
                .any(|(value, limit)| !value.is_finite() || *value < 0.0 || *value * 2.0 >= limit)
        {
            return Err(invalid("Invalid bubble or recovery button layout"));
        }
        let mut references = vec![
            bubble.fill.clone(),
            bubble.text_color.clone(),
            bubble.stroke.color.clone(),
        ];
        valid_outline(&bubble.stroke)?;
        for animation in ui.states.values() {
            valid_point(&animation.offset)?;
        }
        let mut assets = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut interactive = false;
        for layer in &ui.layers {
            if layer.id.is_empty()
                || layer.id.len() > 64
                || !ids.insert(&layer.id)
                || layer
                    .states
                    .keys()
                    .any(|state| !STATES.contains(&state.as_str()))
            {
                return Err(invalid("Duplicate layer id or unknown state binding"));
            }
            for node in std::iter::once(&layer.node).chain(layer.states.values()) {
                valid_point(&node.offset)?;
                interactive |= node.visible && node.interactive;
                if let Some(fill) = &node.fill {
                    references.push(fill.clone());
                }
                if let Some(stroke) = &node.stroke {
                    valid_outline(stroke)?;
                    references.push(stroke.color.clone());
                }
                match &node.shape {
                    Shape::Ellipse { center, radius } => {
                        valid_point(center)?;
                        positive_point(radius)?;
                    }
                    Shape::Polygon { points } => {
                        if !(3..=64).contains(&points.len()) {
                            return Err(invalid("Polygon requires 3–64 points"));
                        }
                        for point in points {
                            valid_point(point)?;
                        }
                    }
                    Shape::Line { points } => {
                        for point in points {
                            valid_point(point)?;
                        }
                    }
                    Shape::Quadratic { points } => {
                        for point in points {
                            valid_point(point)?;
                        }
                    }
                    Shape::Cubic { points } => {
                        for point in points {
                            valid_point(point)?;
                        }
                    }
                    Shape::Rect {
                        origin,
                        size,
                        radius,
                    } => {
                        valid_point(origin)?;
                        positive_point(size)?;
                        if !radius.is_finite() || !(0.0..=128.0).contains(radius) {
                            return Err(invalid("Invalid corner radius"));
                        }
                    }
                    Shape::Text { center, text, size } => {
                        valid_point(center)?;
                        if node.interactive
                            || text.chars().count() > 128
                            || !size.is_finite()
                            || !(4.0..=128.0).contains(size)
                        {
                            return Err(invalid(
                                "Text must be non-interactive and at most 128 characters",
                            ));
                        }
                    }
                    Shape::Image {
                        origin,
                        size,
                        frames,
                        fps,
                    } => {
                        valid_point(origin)?;
                        positive_point(size)?;
                        if frames.is_empty()
                            || frames.len() > 64
                            || !fps.is_finite()
                            || !(0.1..=30.0).contains(fps)
                        {
                            return Err(invalid("Images require 1–64 PNG frames and 0.1–30 FPS"));
                        }
                        for frame in frames {
                            validate_path(frame)?;
                            if !frame.ends_with(".png") {
                                return Err(invalid("Only PNG image assets are supported"));
                            }
                            assets.insert(frame.clone());
                        }
                    }
                }
            }
        }
        if !interactive
            || STATES.iter().any(|state| {
                !ui.layers.iter().any(|layer| {
                    let node = layer.states.get(*state).unwrap_or(&layer.node);
                    node.visible && node.interactive
                })
            })
        {
            return Err(invalid("Skin must have an interactive layer"));
        }
        for reference in references {
            if !self.colors.contains_key(&reference) {
                self.colors
                    .insert(reference.clone(), parse_color(&reference)?);
            }
        }
        Ok(assets)
    }
}

fn valid_point(point: &[Scalar; 2]) -> io::Result<()> {
    for scalar in point {
        if scalar
            .range()
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 4096.0)
        {
            return Err(invalid("Coordinate or animation exceeds allowed bounds"));
        }
        if let Scalar::Binding {
            sine: Some(wave), ..
        } = scalar
            && (!wave.speed.is_finite()
                || !(0.0..=100.0).contains(&wave.speed)
                || !wave.phase.is_finite())
        {
            return Err(invalid("Invalid animation speed or phase"));
        }
    }
    Ok(())
}

fn positive_point(point: &[Scalar; 2]) -> io::Result<()> {
    valid_point(point)?;
    if point
        .iter()
        .any(|scalar| scalar.range()[0] <= 0.0 || scalar.range()[1] > 2048.0)
    {
        return Err(invalid("Shape size must stay positive and at most 2048"));
    }
    Ok(())
}

fn valid_outline(stroke: &Outline) -> io::Result<()> {
    if !stroke.width.is_finite() || !(0.0..=64.0).contains(&stroke.width) {
        return Err(invalid("Invalid stroke width"));
    }
    Ok(())
}

fn rect_inside(rect: [f32; 4], canvas: [f32; 2]) -> io::Result<()> {
    if rect.iter().any(|value| !value.is_finite())
        || rect[0] < 0.0
        || rect[1] < 0.0
        || rect[2] <= 0.0
        || rect[3] <= 0.0
        || rect[0] + rect[2] > canvas[0]
        || rect[1] + rect[3] > canvas[1]
    {
        return Err(invalid("Layout rectangle must fit inside its canvas"));
    }
    Ok(())
}
