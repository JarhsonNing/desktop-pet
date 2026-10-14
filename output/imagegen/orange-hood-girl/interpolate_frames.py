"""Offline RIFE inbetweens for the generated sprite, with transparent PNG output."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from PIL import Image


def run(command):
    result = subprocess.run([str(part) for part in command], capture_output=True, timeout=120)
    if result.returncode:
        raise RuntimeError(result.stderr.decode('utf8', errors='replace')[-1500:])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--rife', default='.artifacts/rife/rife-ncnn-vulkan.exe')
    parser.add_argument('--model', default='.artifacts/rife/rife-v4.6')
    args = parser.parse_args()
    source, output = Path(args.source).resolve(), Path(args.output).resolve()
    rife, model = Path(args.rife).resolve(), Path(args.model).resolve()
    ui = json.loads((source / 'pet.json').read_text(encoding='utf8'))
    sprite = ui['layers'][0]
    output.mkdir(parents=True, exist_ok=False)
    report = {}
    cleanup = Path.home() / '.codex/skills/.system/imagegen/scripts/remove_chroma_key.py'
    for state in ui['states']:
        node = sprite['node'] if state == 'idle' else sprite['states'][state]
        fps = node['shape']['fps']
        factor = min(3, int(30 // fps))
        frames = [Image.open(source / path).convert('RGBA') for path in node['shape']['frames']]
        order = [0, 1, 2, 4, 5, 3] if state == 'grumpy' else list(range(len(frames)))
        frames = [frames[index] for index in order]
        inputs, predictions = output / 'work' / state / 'input', output / 'work' / state / 'prediction'
        directory = output / state
        for path in [inputs, predictions, directory]:
            path.mkdir(parents=True)
        # An extra first keyframe makes the final-to-first transition interpolatable.
        for index, frame in enumerate(frames + frames[:1]):
            keyed = Image.new('RGB', frame.size, '#00ff00')
            keyed.paste(frame, mask=frame.getchannel('A'))
            keyed.save(inputs / f'{index:03}.png')
        run([rife, '-i', inputs, '-o', predictions, '-m', model, '-g', '0',
             '-n', (len(frames) + 1) * factor, '-f', '%08d.png'])
        files = sorted(predictions.glob('*.png'))
        count = len(frames) * factor
        if len(files) != (len(frames) + 1) * factor:
            raise RuntimeError(f'{state}: missing model predictions')
        for index, path in enumerate(files[:count]):
            target = directory / f'{index:03}.png'
            if index % factor == 0:
                # Preserve each authored keyframe and its original alpha exactly.
                frames[index // factor].save(target)
            else:
                run([sys.executable, '-X', 'utf8', cleanup, '--input', path, '--out', target,
                     '--key-color', '#00ff00', '--soft-matte', '--spill-cleanup'])
                # The interpolator can retain saturated green between moving limbs.
                # This reference palette contains no green: remove that matte residue.
                with Image.open(target) as image:
                    image = image.convert('RGBA')
                    image.putdata([(0, 0, 0, 0) if g > r * 1.05 and g > b * 1.3
                                   else (r, g, b, a) for r, g, b, a in image.get_flattened_data()])
                    image.save(target)
        report[state] = {'original_frames': len(frames), 'frames': count,
                         'fps': fps * factor, 'loop_seconds': len(frames) / fps,
                         'keyframe_order': order, 'method': 'RIFE v4.6; original keyframes preserved; intermediate alpha recovered from green matte'}
        print(state, report[state], flush=True)
    (output / 'interpolation-report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf8')


if __name__ == '__main__':
    main()
