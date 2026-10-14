# Transparent PNG pet animation

The current renderer already plays `shape.type: image` frame sequences. Do not replace the loader or invent an APNG/sprite-atlas schema. Import a sheet into separate PNG files and retain JSON only for state bindings, image placement/FPS, metadata and the recovery bubble.

## Artwork and motion

Create a coherent character reference before animating. Preserve face, proportions, markings, lighting and camera across frames. Request an actual transparent background, no baked checkerboard, text, grid lines or labels. A sprite sheet is useful only when cells align exactly; inspect AI sheets for drift, missing cells and duplicated poses before importing. Do not claim an animated poster with only whole-body movement is natural character animation.

Keep all frames on the same canvas, with a stable foot baseline and consistent head/body anchors. Do not crop each frame independently. Bake fur texture, shading, eye movement, blinks, ear flicks, weight transfer and tail follow-through into the artwork. Match first/last loop poses; do not duplicate the first frame at the end unless a deliberate hold is intended. The runtime loops in declared order at constant FPS, with no per-frame duration, pose blending or one-shot playback. Hold a pose by repeating its frame path in manually authored JSON.

For human characters, specify which anatomical hand performs a gesture and keep that side consistent throughout the loop. Inspect both keyframes and interpolated hand/arm closeups for side swaps, disappearing hands, detached wrists and limbs crossing clothing; reduce pose changes or redraw faulty keyframes before interpolation.

PNG frame alpha controls click-through. Images are loaded and decoded once; original alpha must survive import and packaging. Inspect on both light and dark backgrounds for halos. A white/colored background or drawn checkerboard is not transparency. The importer rejects wholly opaque and empty frames; it does not remove backgrounds or judge art quality.

## Inputs

Directory mode: `idle/000.png`, `idle/001.png`, `happy/000.png`, etc. Frames use natural filename ordering (`2` before `10`). All ten state folders are required by default. With `--allow-idle-fallback`, missing folders reuse idle and are reported explicitly. Unknown folders are ignored.

Sheet mode: one PNG, no gutters, ten rows in this exact order:

`idle, happy, grumpy, sleep, dragged, play, chase, hiding, dizzy, walk`

Each row has the same `--columns` frame count. The sheet must divide exactly into its cells. If rows need different lengths, export individual frame folders instead.

```sh
python scripts/import_png.py --project <project> --frames <frames> --output <skin> --id my-cat --name "My Cat" --fps 12
python scripts/import_png.py --project <project> --sheet <sheet.png> --columns 6 --output <skin> --id my-cat --name "My Cat" --fps 12
```

Both modes require Pillow and a fresh output directory. No image resampling or background removal occurs; original frame resolution is retained, and a fixed aspect-preserving display rectangle is fitted below the bubble. The native validator must accept the result before the output directory is published. Per-state FPS can subsequently be changed in `pet.json`.

## Resource planning and playback

Maximum 64 frames per state, 256 package entries, 12,582,912 total pixels among unique referenced PNGs; 2048 maximum frame side. Directory import counts all output frames, not just each state's budget. Practical examples: ten states × six 256×256 frames = 3,932,160 pixels; ten × twelve 180×180 = 3,888,000 pixels. Metadata adds two entries. The actual package also enforces compressed/expanded size limits; use the real packer after import.

Native previews support 1–30 FPS. The app raises its redraw budget for the current visible multi-frame PNG animation, up to 30 FPS; static images retain the vector baseline (idle 2, active 12). Quiet remains 1 FPS and low-power remains 4 FPS. JSON `fps` can be 0.1–30; event timing and low-power policies still affect actual display cadence.

Six keyframes played faster shorten the action rather than filling pose gaps. For smooth movement, author more temporal samples or evaluate frame interpolation, preserving the loop duration. Ten states × eighteen 256×256 PNGs = 11,796,480 pixels, within the new budget. More frames increase decoded memory. Neural/flow interpolation can deform faces, fingers or crossing limbs: inspect intermediate frames and loops, preserve originals for comparison, and never claim interpolated frames are newly hand-drawn poses.

After import, use `skin_tool.py check`, `preview` and `package`, inspect actual frames and all state poses, and deliver the `.mochi` plus source/assets. Keep generators, sheet sources and reports outside the skin folder unless they are intentional packaged assets.
