# Expressive movement within the renderer

Design a readable silhouette first. For a cute creature, head/body proportion, eye spacing, muzzle, ears, and paws should identify the species at small size. Choose a clear palette and contrast for both light and dark desktops.

## Connected motion

- Torso supplies the primary rhythm; head is a coherent group. Eyes, nose, mouth, highlights, markings, and whiskers share the same local head offset. Ground shadow uses `follow_scene: false`.
- Secondary movement may lag by about 0.2–0.7 radians. Tail control points can have progressing phases so its tip follows its base. Small ear-tip shifts imply a pivot without unsupported rotation fields.
- Idle/sleep: shallow breathing, gentle tail, mild gaze. Vector idle redraws at 2 FPS; multi-frame PNGs can request their own rate up to 30 FPS, while quiet remains 1 FPS. Fast tremors or narrow blink spikes strobe or disappear. Discrete blinks need PNG frames and still obey the redraw budget.
- Happy/play: somewhat larger motion, raised/alternating paws, a face change, faster tail. Keep head connected; avoid rapid whole-body shaking.
- Walk/chase: opposing feet separated by pi, a smaller torso bob, and stable face. Suggest contact and weight transfer, not every part floating together.
- Dragged: dangling paws, modest stretch, ears/tail settling. Dizzy: small asymmetric bounce and distinct face; no unsupported rigid spin.
- Grumpy/hiding: brows, eyelids, paws, and a small prop can tell the state. Keep recovery controls visible.

## Loops and transitions

For a seamless loop of T seconds, choose speeds `2*pi*k/T` for integer k. Vary amplitudes, phases, and harmonic counts instead of moving all parts identically. Render a full period and compare last/first frames.

Bounds include both node and scene offset, radius, outline, and gaze. Bézier control-box bounds are conservative; inspect frames when a control-box warning occurs. The v1 format switches whole nodes between moods with no blending/easing. Similar shared anchors and resting centers reduce visible jumps. Explain this limit if genuine transition blending is requested; do not invent JSON fields.

## Preview interpretation

The native exporter uses actual `SkinRenderer`, but does not simulate behavior logic, cursor chase, or icon hiding. Sampling rate evaluates the authored pose. Vector animation retains active 12 / idle 2 FPS; current multi-frame PNGs can raise the budget up to 30 FPS. Low power remains 4 FPS and quiet 1; input events can add draws. Motion must work under these policies.

Inspect both gallery and moving frames. Format validation alone cannot prove natural movement. Look for disconnected features, wrong foot timing, clipping, blank variants, excessive bounce, or loop jumps; fix observed causes.
