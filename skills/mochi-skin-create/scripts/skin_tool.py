"""Mochi skin authoring helper: native validation/packaging and actual-renderer previews."""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import tomllib
import zipfile

STATES = ['idle', 'happy', 'grumpy', 'sleep', 'dragged', 'play', 'chase', 'hiding', 'dizzy', 'walk']
SKILL = Path(__file__).resolve().parents[1]


def run(command, cwd=None, timeout=120):
    result = subprocess.run([str(part) for part in command], cwd=cwd, capture_output=True, timeout=timeout)
    if result.returncode:
        diagnostic = (result.stdout + result.stderr).decode('utf-8', errors='replace').strip()
        raise RuntimeError(f'Command failed ({result.returncode}): {command[0]}\n{diagnostic}')
    return result


def project_path(value):
    candidate = Path(value).resolve() if value else Path.cwd().resolve()
    candidates = [candidate] if value else [candidate, *candidate.parents]
    for root in candidates:
        manifest = root / 'Cargo.toml'
        if manifest.is_file() and tomllib.loads(manifest.read_text(encoding='utf-8')).get('package', {}).get('name') == 'desktop-pet':
            return root
    raise RuntimeError('Cannot find desktop-pet project; pass --project PATH')


def engine(root):
    executable = 'desktop-pet.exe' if sys.platform == 'win32' else 'desktop-pet'
    candidates = [root / 'target' / profile / executable for profile in ['debug', 'release']]
    existing = [path for path in candidates if path.is_file()]
    if not existing:
        run(['cargo', 'build', '--locked', '--bin', 'desktop-pet'], root)
        existing = [candidates[0]]
    return max(existing, key=lambda path: path.stat().st_mtime)


def read_ui(source):
    if source.is_dir():
        manifest = json.loads((source / 'manifest.json').read_text(encoding='utf-8'))
        return manifest, json.loads((source / manifest['ui']).read_text(encoding='utf-8'))
    data = source.read_bytes()
    if data.startswith(b'MOCHISK\0'):
        data = data[12:]
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        manifest = json.loads(archive.read('manifest.json'))
        return manifest, json.loads(archive.read(manifest['ui']))


def scalar(value, time, gaze):
    if isinstance(value, (float, int)):
        return value
    result = value['base']
    if 'sine' in value and value['sine'] is not None:
        wave = value['sine']
        phase = math.sin(time * wave['speed'] + wave.get('phase', 0))
        result += wave['amplitude'] * (abs(phase) if wave.get('absolute', False) else phase)
    if 'gaze' in value and value['gaze'] is not None:
        binding = value['gaze']
        result += gaze[0 if binding['axis'] == 'x' else 1] * binding['amount']
    return result


def pair(value, time, gaze):
    return [scalar(axis, time, gaze) for axis in value]


def node_bounds(node, scene, time, gaze):
    shape = node['shape']
    offset = pair(node.get('offset', [0, 0]), time, gaze)
    if node.get('follow_scene', True):
        offset = [a + b for a, b in zip(offset, pair(scene.get('offset', [0, 0]), time, gaze))]
    kind = shape['type']
    if kind == 'ellipse':
        center, radius = pair(shape['center'], time, gaze), pair(shape['radius'], time, gaze)
        bounds = [center[0]-radius[0], center[1]-radius[1], center[0]+radius[0], center[1]+radius[1]]
    elif kind in ['rect', 'image']:
        origin, size = pair(shape['origin'], time, gaze), pair(shape['size'], time, gaze)
        bounds = [*origin, origin[0]+size[0], origin[1]+size[1]]
    elif kind == 'text':
        center = pair(shape['center'], time, gaze)
        width = len(shape['text']) * shape['size'] * .6
        bounds = [center[0]-width/2, center[1]-shape['size']/2, center[0]+width/2, center[1]+shape['size']/2]
    else:
        points = [pair(point, time, gaze) for point in shape['points']]
        bounds = [min(p[0] for p in points), min(p[1] for p in points), max(p[0] for p in points), max(p[1] for p in points)]
    width = (node.get('stroke') or {}).get('width', 0) / 2
    return [bounds[0]+offset[0]-width, bounds[1]+offset[1]-width, bounds[2]+offset[0]+width, bounds[3]+offset[1]+width]


def check(root, source):
    run([engine(root), '--check-skin', source], root)
    manifest, ui = read_ui(source)
    warnings = {}
    for mood in STATES:
        scene = ui['states'].get(mood, ui['states']['idle'])
        for layer in ui['layers']:
            node = layer.get('states', {}).get(mood, layer['node'])
            if not node.get('visible', True):
                continue
            for gaze in [[0, 0], [.8, .6], [-.8, -.6]]:
                for index in range(81):
                    time = index / 10
                    bounds = node_bounds(node, scene, time, gaze)
                    if bounds[0] < -.5 or bounds[1] < -.5 or bounds[2] > ui['canvas'][0]+.5 or bounds[3] > ui['canvas'][1]+.5:
                        warnings[f'{mood}/{layer["id"]}'] = {'state': mood, 'layer': layer['id'], 'time': time, 'bounds': [round(v, 2) for v in bounds], 'note': 'Control/text bounds are conservative; inspect the actual render.'}
                        break
    return {'skin': manifest['id'], 'native_validation': 'passed', 'states': STATES, 'bounds_warnings': list(warnings.values())}


def init(args):
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', args.id):
        raise RuntimeError('Skin id must contain 1–64 ASCII letters/digits/-/_')
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    ui = json.loads((SKILL / 'assets' / 'base-ui.json').read_text(encoding='utf-8'))
    manifest = {'format': 'mochi.skin', 'version': 1, 'id': args.id, 'name': args.name, 'author': args.author, 'ui': 'pet.json'}
    for name, value in [('manifest.json', manifest), ('pet.json', ui)]:
        (output / name).write_bytes((json.dumps(value, ensure_ascii=False, indent=2)+'\n').encode('utf-8'))
    return {'source': str(output), 'placeholder': True}


def preview(args, root, source):
    report = check(root, source)
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    frames = round(args.seconds * args.fps)
    if not 1 <= args.fps <= 30 or not 1 <= frames <= 120:
        raise RuntimeError('Preview requires 1–30 FPS and 1–120 frames')
    if not (root / 'examples' / 'skin_preview.rs').is_file():
        raise RuntimeError('This project needs examples/skin_preview.rs for native previews')
    run(['cargo', 'build', '--locked', '--example', 'skin_preview'], root)
    name = 'skin_preview.exe' if sys.platform == 'win32' else 'skin_preview'
    native = root / 'target' / 'debug' / 'examples' / name
    report['native_renderer'] = 'SkinRenderer'
    try:
        run([native, '--skin', source, '--output', output, '--gallery'], root)
        animation = output / 'frames'
        run([native, '--skin', source, '--output', animation, '--mood', args.mood, '--fps', args.fps, '--frames', frames], root)
        files = sorted(animation.glob('frame-*.png'))
        if len(files) != frames or not (output / 'gallery.png').is_file():
            raise RuntimeError('Native capture did not produce all requested images')
        report.update({'native_preview': 'passed', 'mood': args.mood, 'sample_fps': args.fps, 'frame_count': len(files), 'unique_frames': len({hashlib.sha256(path.read_bytes()).hexdigest() for path in files})})
        try:
            from PIL import Image, ImageChops, ImageStat
            images = [Image.open(path).convert('RGB') for path in files]
            durations = [round((index+1)*100/args.fps)*10 - round(index*100/args.fps)*10 for index in range(len(images))]
            images[0].save(output / 'animation.gif', save_all=True, append_images=images[1:], duration=durations, loop=0, disposal=2)
            difference = lambda a, b: sum(ImageStat.Stat(ImageChops.difference(a, b)).mean)/3
            steps = [difference(a, b) for a, b in zip(images, images[1:])]
            seam = difference(images[-1], images[0])
            report['animation_gif'] = 'animation.gif'
            report['loop_seam_mean_rgb_difference'] = round(seam, 3)
            report['max_step_mean_rgb_difference'] = round(max(steps, default=0), 3)
            report['loop_seam_needs_review'] = seam > max(max(steps, default=0)*3, 2)
        except ImportError:
            report['animation_gif'] = None
            report['gif_note'] = 'Pillow unavailable; inspect native PNG frames instead.'
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        report['native_preview'] = 'failed'
        report['error'] = str(error)
        (output / 'report.json').write_bytes((json.dumps(report, ensure_ascii=False, indent=2)+'\n').encode('utf-8'))
        raise
    (output / 'report.json').write_bytes((json.dumps(report, ensure_ascii=False, indent=2)+'\n').encode('utf-8'))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    creation = commands.add_parser('init')
    creation.add_argument('--output', required=True)
    creation.add_argument('--id', required=True)
    creation.add_argument('--name', required=True)
    creation.add_argument('--author', default='Mochi Skin Creator')
    for name in ['check', 'package', 'preview']:
        command = commands.add_parser(name)
        command.add_argument('--project')
        command.add_argument('--skin', required=True)
        if name != 'check':
            command.add_argument('--output', required=True)
        if name == 'preview':
            command.add_argument('--mood', choices=STATES, default='happy')
            command.add_argument('--seconds', type=float, default=3)
            command.add_argument('--fps', type=int, default=12)
    args = parser.parse_args()
    if args.command == 'init':
        report = init(args)
    else:
        root = project_path(args.project)
        source = Path(args.skin).resolve()
        if args.command == 'preview':
            report = preview(args, root, source)
        else:
            report = check(root, source)
            if args.command == 'package':
                output = Path(args.output).resolve()
                run([engine(root), '--pack-skin', source, '--output', output], root)
                run([engine(root), '--check-skin', output], root)
                report.update({'package': str(output), 'sha256': hashlib.sha256(output.read_bytes()).hexdigest(), 'package_validation': 'passed'})
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
