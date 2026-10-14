use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const STATES: [&str; 10] = [
    "idle", "happy", "grumpy", "sleep", "dragged", "play", "chase", "hiding", "dizzy", "walk",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    pub ui: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PetUi {
    pub canvas: [f32; 2],
    pub interaction_bounds: [f32; 4],
    pub gaze_origin: [f32; 2],
    pub gaze_distance: f32,
    pub chase_anchor: [f32; 2],
    pub palette: BTreeMap<String, String>,
    pub bubble: Bubble,
    pub states: BTreeMap<String, Animation>,
    pub layers: Vec<Layer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bubble {
    pub rect: [f32; 4],
    pub hidden_height: f32,
    pub text_inset: [f32; 2],
    pub font_size: f32,
    pub radius: f32,
    pub fill: String,
    pub text_color: String,
    pub stroke: Outline,
    pub restore_button: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    #[serde(default)]
    pub offset: [Scalar; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: String,
    pub node: Node,
    #[serde(default)]
    pub states: BTreeMap<String, Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub shape: Shape,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default = "yes")]
    pub follow_scene: bool,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub offset: [Scalar; 2],
    #[serde(default)]
    pub fill: Option<String>,
    #[serde(default)]
    pub stroke: Option<Outline>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    Ellipse {
        center: [Scalar; 2],
        radius: [Scalar; 2],
    },
    Polygon {
        points: Vec<[Scalar; 2]>,
    },
    Line {
        points: [[Scalar; 2]; 2],
    },
    Quadratic {
        points: [[Scalar; 2]; 3],
    },
    Cubic {
        points: [[Scalar; 2]; 4],
    },
    Rect {
        origin: [Scalar; 2],
        size: [Scalar; 2],
        radius: f32,
    },
    Text {
        center: [Scalar; 2],
        text: String,
        size: f32,
    },
    Image {
        origin: [Scalar; 2],
        size: [Scalar; 2],
        frames: Vec<String>,
        fps: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outline {
    pub color: String,
    pub width: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Scalar {
    Constant(f32),
    Binding {
        base: f32,
        #[serde(default)]
        sine: Option<Wave>,
        #[serde(default)]
        gaze: Option<Gaze>,
    },
}

impl Default for Scalar {
    fn default() -> Self {
        Self::Constant(0.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wave {
    pub amplitude: f32,
    pub speed: f32,
    #[serde(default)]
    pub phase: f32,
    #[serde(default)]
    pub absolute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gaze {
    pub axis: Axis,
    pub amount: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
}

impl Scalar {
    pub fn value(&self, time: f64, gaze: [f32; 2]) -> f32 {
        match self {
            Self::Constant(value) => *value,
            Self::Binding {
                base,
                sine,
                gaze: binding,
            } => {
                let wave = sine.as_ref().map_or(0.0, |wave| {
                    let value = (time as f32 * wave.speed + wave.phase).sin();
                    wave.amplitude * if wave.absolute { value.abs() } else { value }
                });
                let eye = binding.as_ref().map_or(0.0, |binding| {
                    gaze[match binding.axis {
                        Axis::X => 0,
                        Axis::Y => 1,
                    }] * binding.amount
                });
                base + wave + eye
            }
        }
    }

    pub fn range(&self) -> [f32; 2] {
        match self {
            Self::Constant(value) => [*value, *value],
            Self::Binding { base, sine, gaze } => {
                let amount = sine.as_ref().map_or(0.0, |wave| wave.amplitude.abs())
                    + gaze.as_ref().map_or(0.0, |gaze| gaze.amount.abs());
                [base - amount, base + amount]
            }
        }
    }
}
