"""Import transparent PNG sequences or a ten-row sprite sheet into a Mochi skin."""
import argparse
import copy
import json
import math
from pathlib import Path
import re
import tempfile

from skin_tool import SKILL, STATES, check, project_path

MAX_PIXELS = 12 * 1024 * 1024
MAX_FILES = 256


def natural_key(path):
    return [int(part) if part.isdigit() else part.casefold()
            for part in re.split(r'(\d+)', path.name)]


def read_frames(args):
    from PIL import Image

    result = {}
    pixel_count = 0
    file_count = 0
    if args.frames:
        root = Path(args.frames).resolve()
        if not root.is_dir():
            raise ValueError('Frame directory does not exist')
        for state in STATES:
            directory = root / state
            files = sorted((path for path in directory.glob('*')
                            if path.is_file() and path.suffix.lower() == '.png'), key=natural_key)
            if len(files) > 64:
                raise ValueError(f'{state}: at most 64 frames')
            if files:
                images = []
                for path in files:
                    with Image.open(path) as image:
                        if image.format != 'PNG' or getattr(image, 'n_frames', 1) != 1:
                            raise ValueError(f'{path}: expected a single-frame PNG')
                        pixel_count += image.width * image.height
                        file_count += 1
                        if max(image.size) > 2048 or pixel_count > MAX_PIXELS or file_count + 2 > MAX_FILES:
                            raise ValueError('Frames exceed 2048px sides / 12M total pixels / 256 files')
                        images.append(image.convert('RGBA'))
                result[state] = images
    else:
        if not args.columns or not 1 <= args.columns <= 64:
            raise ValueError('Sprite sheet requires --columns between 1 and 64')
        with Image.open(args.sheet) as sheet:
            if sheet.format != 'PNG' or getattr(sheet, 'n_frames', 1) != 1:
                raise ValueError('Sprite sheet must be a single-frame PNG')
            width, height = sheet.size
            if width % args.columns or height % len(STATES):
                raise ValueError('Sheet must divide evenly into columns and ten state rows')
            size = (width // args.columns, height // len(STATES))
            if width * height > MAX_PIXELS or max(size) > 2048:
                raise ValueError('Sheet exceeds the 12M total pixel / 2048px frame limit')
            sheet = sheet.convert('RGBA')
            for row, state in enumerate(STATES):
                result[state] = [sheet.crop((column * size[0], row * size[1],
                                             (column + 1) * size[0], (row + 1) * size[1]))
                                 for column in range(args.columns)]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument('--frames', help='Directory containing idle/, happy/, etc.')
    inputs.add_argument('--sheet', help='PNG grid: ten state rows, no gutters or labels')
    parser.add_argument('--columns', type=int, help='Frames in each sprite sheet row')
    parser.add_argument('--allow-idle-fallback', action='store_true')
    parser.add_argument('--project')
    parser.add_argument('--output', required=True)
    parser.add_argument('--id', required=True)
    parser.add_argument('--name', required=True)
    parser.add_argument('--author', default='')
    parser.add_argument('--fps', type=float, default=12)
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', args.id):
        raise ValueError('Skin id requires 1–64 ASCII letters/digits/-/_')
    if not math.isfinite(args.fps) or not .1 <= args.fps <= 30:
        raise ValueError('FPS must be between 0.1 and 30')
    if args.frames and args.columns is not None:
        raise ValueError('--columns is only used with --sheet')
    output = Path(args.output).resolve()
    if output.exists():
        raise ValueError('Output already exists; use a fresh directory')
    project = project_path(args.project)
    frames = read_frames(args)
    if 'idle' not in frames:
        raise ValueError('An idle animation is required')
    missing = [state for state in STATES if state not in frames]
    if missing and not args.allow_idle_fallback:
        raise ValueError(f'Missing states: {", ".join(missing)}; explicitly allow idle fallback if intended')
    sizes = {image.size for images in frames.values() for image in images}
    if len(sizes) != 1:
        raise ValueError('All frames must share one canvas size; preserve the same ground/head anchors')
    width, height = next(iter(sizes))
    if min(width, height) < 1 or max(width, height) > 2048:
        raise ValueError('Frame size must be between 1 and 2048 pixels')
    total_frames = sum(map(len, frames.values()))
    if total_frames + 2 > MAX_FILES or width * height * total_frames > MAX_PIXELS:
        raise ValueError('Frames exceed 256 files / 12M total pixels; reduce frame count or dimensions')
    for state, images in frames.items():
        for index, image in enumerate(images):
            minimum, maximum = image.getchannel('A').getextrema()
            if minimum == 255 or maximum < 16:
                raise ValueError(f'{state} frame {index}: requires a visible subject and real transparent background')
    ui = json.loads((SKILL / 'assets' / 'base-ui.json').read_text(encoding='utf-8'))
    # Keep the existing recovery bubble; fit one stable image rectangle below it.
    factor = min(248 / width, 206 / height)
    size = [width * factor, height * factor]
    origin = [160 - size[0] / 2, 290 - size[1]]
    ui['states'] = {state: {} for state in STATES}
    ui['layers'] = []
    manifest = {'format': 'mochi.skin', 'version': 1, 'id': args.id,
                'name': args.name, 'author': args.author, 'ui': 'pet.json'}
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='mochi-png-', dir=output.parent) as temporary:
        source = Path(temporary) / 'skin'
        images_dir = source / 'images'
        images_dir.mkdir(parents=True)
        nodes = {}
        for state, images in frames.items():
            names = []
            for index, image in enumerate(images):
                name = f'images/{state}-{index:03}.png'
                image.save(source / name)
                names.append(name)
            nodes[state] = {'interactive': True, 'follow_scene': False,
                            'shape': {'type': 'image', 'origin': origin, 'size': size,
                                      'frames': names, 'fps': args.fps}}
        ui['layers'] = [{'id': 'sprite', 'node': nodes['idle'],
                         'states': {state: copy.deepcopy(nodes.get(state, nodes['idle']))
                                    for state in STATES if state != 'idle'}}]
        for name, content in [('manifest.json', manifest), ('pet.json', ui)]:
            (source / name).write_text(json.dumps(content, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        report = check(project, source)
        source.rename(output)
    report.update({'source': str(output), 'frame_size': [width, height],
                   'png_count': total_frames, 'total_pixels': width * height * total_frames,
                   'idle_fallback_states': missing})
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (ImportError, OSError, ValueError, RuntimeError) as error:
        raise SystemExit(str(error))
