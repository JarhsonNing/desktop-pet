---
name: mochi-skin-create
description: Create and refine Mochi desktop-pet skins from a character idea or reference, with expressive state bindings, animation, editable UI files, previews, and a validated .mochi package. Use for this project's desktop-pet skins, not ChatGPT Work Pets or unrelated website themes.
---

# Mochi Skin Creator

Turn the user's idea into an original, editable skin that the Mochi desktop app can load. Deliver the skin source directory, a `.mochi` package, and visual evidence. Keep the requested character and style; choose reasonable details when the brief is short.

## Establish the format and design

- Read [references/format.md](references/format.md) for the v1 contract and [references/motion.md](references/motion.md) for movement design and preview limits.
- Locate the `desktop-pet` project from the working directory, or take an explicit `--project` path. The project's `docs/SKINS.md`, `src/skin/definition.rs`, and `--check-skin` command are authoritative if the bundled reference differs.
- Prefer transparent PNG animation frames for polished pet artwork, painted fur, lighting, discrete blinks, and natural pose changes. JSON remains a small state/playback/bubble configuration rather than a drawing. Use editable vector layers when the user requests a vector style or procedural geometry.
- For new bitmap artwork, apply the available image-generation skill and respect its tool/fallback rules. If generation is unavailable, report that limitation and complete the import workflow or use user-provided assets; do not silently substitute geometric art or recolor old screenshots and claim it is refined new artwork.
- Define a distinct silhouette, face, palette, and material accents. For a new character, redesign geometry and state poses rather than merely recoloring `skins/default`.
- For simple requests, proceed with a brief stated assumption. Ask only when an absent reference, licensing choice, or contradictory requirement prevents useful work.

## Build the skin

For PNG artwork, read [references/png-animation.md](references/png-animation.md). Import user-provided or generated transparent frames, then preview and package:

```sh
python scripts/import_png.py --project <project> --frames <frame-directory> --output <source-directory> --id <skin-id> --name <display-name> --fps 12
```

The importer also accepts a ten-row PNG sprite sheet with `--sheet <png> --columns <frames-per-row>`. It preserves alpha, writes individual frames, fits the image below the recovery bubble, and runs the actual validator. Pillow is required. Distinct character states require distinct authored artwork; `--allow-idle-fallback` is an explicit partial-skin option, not proof that all moods were created.

For a vector skin or manual JSON authoring, start with:

```sh
python scripts/skin_tool.py init --output <source-directory> --id <skin-id> --name <display-name>
```

Resolve the helper relative to this skill folder, not the project. Its initial body is a placeholder: replace it with the user's character. Keep generation scripts and previews outside the skin directory so the package contains only intended definitions/assets.

- Author `manifest.json`, `pet.json`, and referenced PNGs. Retain the recovery button's program binding and valid layout.
- Bind all ten existing moods. Fallbacks may be intentional, but make the main poses recognizably different: awake, happy, grumpy, asleep, held, playing, chasing, hiding, dizzy, and walking.
- Keep connected facial features on the same head motion. Use modest phase offsets for secondary motion; opposing feet in a walk; calm breathing in idle/sleep. Do not drive every layer with the same large bounce.
- A state variant replaces its whole node. Repeat its required fill, outline, interaction flag, and local offset. Do not invent rotation, opacity expressions, rigging, transition easing, or new actions absent from the schema.
- Keep the interactive silhouette inside the canvas throughout animation. The recovery bubble must fit during `hiding`. Do not change application authorization, desktop files, or behavior code to make a cosmetic skin work.

## Validate and preview

```sh
python scripts/skin_tool.py check --project <project> --skin <source-directory>
python scripts/skin_tool.py preview --project <project> --skin <source-directory> --output <preview-directory> --mood happy --seconds 3 --fps 12
python scripts/skin_tool.py package --project <project> --skin <source-directory> --output <skin-id>.mochi
```

`check` uses the actual app validator and samples animation bounds. `preview` uses the project's native `examples/skin_preview.rs` and actual `SkinRenderer` to capture a ten-state gallery and animated PNG frames. With Pillow available, it also creates a GIF and checks that frames change. Init, check, and package need only Python's standard library; use the bundled workspace Python if Pillow is needed.

Inspect the gallery and GIF or spaced frames. Check species/style, face alignment, silhouette, tail/ear continuity, foot contact, clipping, transparency, and loop seam. Refine when observation warrants it; structural validity is not proof of attractive or natural motion. Read the report and do not claim unavailable or failed tests passed.

Preview rendering never runs the behavior engine or desktop pranks. For normal app loading, launch the package using a fresh `--data-dir` plus `--settings --exit-after 4 --screenshot <file>`, with all prank consent disabled. If native graphics are unavailable, retain package/source and report incomplete visual validation; do not change drivers or desktop settings.

## Deliver

Link the `.mochi` package, editable source, and preview; describe the character and movement actually implemented. Include a load command or the control-panel「皮肤」entry. State validation results and material limits. Preview FPS is a sampling rate. Multi-frame PNG skins can raise app redraws up to 30 FPS, but quiet and low-power modes retain their limits. More FPS alone cannot smooth sparse keyframes; preserve loop duration when adding inbetweens and inspect for deformation.
