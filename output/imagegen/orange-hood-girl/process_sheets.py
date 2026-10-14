from pathlib import Path
from PIL import Image
import subprocess,sys
root=Path('output/imagegen/orange-hood-girl')
states=['idle','happy','grumpy','sleep','dragged','play','chase','hiding','dizzy','walk']
# Process only completed model outputs. The original generated sheets remain intact.
for state in states:
    raw=root/'sheets'/f'{state}.png'
    if not raw.exists():
        continue
    alpha=root/'transparent-sheets'/f'{state}.png'
    alpha.parent.mkdir(exist_ok=True)
    if not alpha.exists():
        with Image.open(raw) as original:
            native = original.convert('RGBA')
            histogram = native.getchannel('A').histogram()
            has_alpha = sum(histogram[:9]) > native.width * native.height * .2
            if has_alpha:
                # Preserve generated alpha; remove only invisible alpha noise.
                native.putalpha(native.getchannel('A').point(lambda a: 0 if a <= 8 else a))
                native.save(alpha)
        if not has_alpha:
            subprocess.run([sys.executable,r'C:/Users/DELL/.codex/skills/.system/imagegen/scripts/remove_chroma_key.py','--input',str(raw),'--out',str(alpha),'--key-color','#FF00FF','--soft-matte','--transparent-threshold','20','--opaque-threshold','100','--spill-cleanup'],check=True)
    with Image.open(alpha) as sheet:
        w,h=sheet.size
        if w%3 or h%2:
            raise SystemExit(f'{state}: sheet dimensions {w}x{h} do not divide into 3x2')
        target=root/'frames'/state
        target.mkdir(parents=True,exist_ok=True)
        for i in range(6):
            x,y=i%3,i//3
            frame=sheet.crop((x*w//3,y*h//2,(x+1)*w//3,(y+1)*h//2)).convert('RGBA')
            frame=frame.resize((256,256),Image.Resampling.LANCZOS)
            # The model enlarged heads in seated/crouched states. Use one fixed
            # scale per state to restore reference proportions without per-frame cropping.
            scale={'sleep':.82,'play':.86,'hiding':.85}.get(state,1)
            if scale!=1:
                side=round(256*scale)
                small=frame.resize((side,side),Image.Resampling.LANCZOS)
                frame=Image.new('RGBA',(256,256))
                frame.paste(small,((256-side)//2,256-side))
            if frame.getchannel('A').getbbox() is None:
                raise SystemExit(f'{state} frame {i}: empty subject')
            frame.save(target/f'{i:03}.png')
    print(state,'processed',w,h)
