# Mochi skin v1 contract

Use the real app validator for acceptance. Current project source takes precedence over this reference.

Skin root: `manifest.json`, `pet.json` (or manifest `ui` path), optional PNGs. `.mochi` = `MOCHISK\0`, little-endian `u32` version 1, ZIP payload with offsets relative to payload. Plain ZIP and source directories are accepted. No wrapper directory inside the archive.

Manifest: `format: "mochi.skin"`, `version: 1`, `id` (1–64 ASCII letters/digits/`-`/`_`), `name` (1–64 characters), optional `author` (up to 128), `ui` (relative path). Arbitrary Chinese names may lack glyphs in the embedded font; a Latin app display name can accompany a Chinese character name in delivery.

Top-level UI:

- `canvas: [width,height]`, each 64–1024.
- `interaction_bounds: [x,y,width,height]`, inside canvas, at least 24×24; used for monitor recovery, not a substitute for shape/alpha hit testing.
- `gaze_origin`, `chase_anchor`: coordinate pairs inside canvas; `gaze_distance`: positive number.
- `palette: {name: "#RRGGBB" or "#RRGGBBAA"}`, names use ASCII letters/digits/`-`/`_`.
- `states: {mood: {offset: [scalar,scalar]}}`; `idle` required, missing scene states fall back to idle.
- `layers: [{id,node,states?: {mood: completeNode}}]`, back to front; missing layer variant uses default node.
- `bubble`: fields below.

Moods: `idle`, `happy`, `grumpy`, `sleep`, `dragged`, `play`, `chase`, `hiding`, `dizzy`, `walk`. Every mood needs a visible interactive layer.

## Nodes and bindings

Common fields: `shape`, `visible` (default true), `follow_scene` (true), `interactive` (false), `offset` ([0,0]), optional `fill`, optional `stroke: {color,width}`. State variants are **complete replacements**, not patches.

| shape.type | fields |
| --- | --- |
| ellipse | `center`, `radius`: scalar pairs |
| polygon | `points`: 3–64 pairs, ordered convex polygon |
| line | `points`: 2 pairs |
| quadratic / cubic | `points`: 3 / 4 pairs; stroked open Bézier paths |
| rect | `origin`, `size`: pairs; `radius`: fixed number |
| text | `center`: pair; `text`: string; `size`: fixed number; color from fill; non-interactive |
| image | `origin`, `size`: pairs; `frames`: 1–64 PNG paths; `fps`: 0.1–30 |

Sizes/radii stay positive across the binding's range. There is no SVG import, rig, per-node rotation, script execution, opacity binding, or custom action. Curves need a stroke; their fill is not rendered. Transparent fills and PNG alpha below 16/255 pass clicks through.

A scalar is a number or:

```json
{"base":0,"sine":{"amplitude":1.2,"speed":2.094395,"phase":0,"absolute":false},"gaze":{"axis":"x","amount":2}}
```

Both bindings are optional. Value = base + amplitude × sin(time × speed + phase) + normalized gaze-axis × amount. Absolute takes the sine's absolute value, not easing or a delayed blink. Seconds and radians/second; gaze is limited to the unit circle.

## Bubble and resource limits

Bubble: `rect: [x,y,w,h]`, `hidden_height`, `text_inset`, `font_size`, `radius`, `fill`, `text_color`, `stroke`, `restore_button: [x,y,w,h]` relative to bubble. Normal/hidden rectangles fit canvas. Button fits hidden height and is at least 60×18. Its action stays controlled by the app.

128 layers, 64 palette entries; coordinates/binding extents ±4096; positive sizes up to 2048; stroke 0–64. Package 16 MiB, expanded 32 MiB, each asset 8 MiB, 256 file/directory entries. PNG max 2048 per side, all distinct referenced images total at most 12,582,912 pixels. Exact-case `/` paths; no absolute/parent paths, backslashes, symlinks, duplicate names, or Windows device names. Package/unpack refuse existing outputs.
