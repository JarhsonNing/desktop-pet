use super::{Scalar, Shape, Skin};
use base64::Engine;
use std::fmt::Write;

fn point(value: &[Scalar; 2]) -> [f32; 2] {
    [value[0].value(0.0, [0.0; 2]), value[1].value(0.0, [0.0; 2])]
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Skin {
    /// Static panel thumbnail built from the validated skin; the pet keeps its native renderer.
    pub fn preview_svg(&self) -> Result<String, image::ImageError> {
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\">",
            self.ui.canvas[0], self.ui.canvas[1]
        );
        let scene = self
            .ui
            .states
            .get("idle")
            .map(|s| point(&s.offset))
            .unwrap_or([0.0; 2]);
        let color = |value: &str| {
            let c = self.color(value);
            let [r, g, b, a] = c.to_srgba_unmultiplied();
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        };
        for layer in &self.ui.layers {
            let node = layer.states.get("idle").unwrap_or(&layer.node);
            if !node.visible {
                continue;
            }
            let mut offset = point(&node.offset);
            if node.follow_scene {
                offset[0] += scene[0];
                offset[1] += scene[1];
            }
            let fill = node
                .fill
                .as_deref()
                .map(color)
                .unwrap_or_else(|| "none".into());
            let (stroke, width) = node
                .stroke
                .as_ref()
                .map(|s| (color(&s.color), s.width))
                .unwrap_or(("none".into(), 0.0));
            let _ = write!(
                svg,
                "<g transform=\"translate({} {})\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">",
                offset[0], offset[1]
            );
            match &node.shape {
                Shape::Ellipse { center, radius } => {
                    let c = point(center);
                    let r = point(radius);
                    let _ = write!(
                        svg,
                        "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"/>",
                        c[0], c[1], r[0], r[1]
                    );
                }
                Shape::Rect {
                    origin,
                    size,
                    radius,
                } => {
                    let p = point(origin);
                    let s = point(size);
                    let _ = write!(
                        svg,
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{radius}\"/>",
                        p[0], p[1], s[0], s[1]
                    );
                }
                Shape::Polygon { points } => {
                    let list = points
                        .iter()
                        .map(|p| {
                            let p = point(p);
                            format!("{},{}", p[0], p[1])
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    let _ = write!(svg, "<polygon points=\"{list}\"/>");
                }
                Shape::Line { points } => {
                    let a = point(&points[0]);
                    let b = point(&points[1]);
                    let _ = write!(
                        svg,
                        "<path d=\"M {} {} L {} {}\" fill=\"none\"/>",
                        a[0], a[1], b[0], b[1]
                    );
                }
                Shape::Quadratic { points } => {
                    let p = points.clone().map(|p| point(&p));
                    let _ = write!(
                        svg,
                        "<path d=\"M {} {} Q {} {} {} {}\"/>",
                        p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1]
                    );
                }
                Shape::Cubic { points } => {
                    let p = points.clone().map(|p| point(&p));
                    let _ = write!(
                        svg,
                        "<path d=\"M {} {} C {} {} {} {} {} {}\"/>",
                        p[0][0], p[0][1], p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]
                    );
                }
                Shape::Text { center, text, size } => {
                    let p = point(center);
                    let _ = write!(
                        svg,
                        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" dominant-baseline=\"middle\" font-family=\"sans-serif\" font-size=\"{size}\">{}</text>",
                        p[0],
                        p[1],
                        escape(text)
                    );
                }
                Shape::Image {
                    origin,
                    size,
                    frames,
                    ..
                } => {
                    if let Some(image) = frames.first().and_then(|name| self.images.get(name)) {
                        let mut png = std::io::Cursor::new(Vec::new());
                        image.write_to(&mut png, image::ImageFormat::Png)?;
                        let data =
                            base64::engine::general_purpose::STANDARD.encode(png.into_inner());
                        let p = point(origin);
                        let s = point(size);
                        let _ = write!(
                            svg,
                            "<image href=\"data:image/png;base64,{data}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                            p[0], p[1], s[0], s[1]
                        );
                    }
                }
            }
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        Ok(format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg)
        ))
    }
}
