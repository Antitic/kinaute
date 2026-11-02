#!/usr/bin/env python3
"""Compare Kinaute renders with LibreOffice renders (same width) over many .pptx files.

usage: compare.py OUTDIR FILE.pptx...
Prints the mean absolute pixel difference per slide, worst first, and writes
side-by-side PNGs (Kinaute | LibreOffice) for the worst slides.
LibreOffice is only a *reference for regressions*: it is not Keynote nor PowerPoint.
"""
import subprocess, sys, os, glob, shutil
from PIL import Image, ImageChops, ImageStat

W = 960
out = sys.argv[1]
files = sys.argv[2:]
tool = os.path.join(os.path.dirname(__file__), "..", "target", "release", "kinaute-tool")
os.makedirs(out, exist_ok=True)
rows = []
for f in files:
    name = os.path.splitext(os.path.basename(f))[0]
    d = os.path.join(out, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    subprocess.run([tool, "render", f, os.path.join(d, "k"), str(W)], capture_output=True, timeout=120)
    try:
        subprocess.run(["soffice", "--headless", "--convert-to", "pdf", "--outdir", d, f], capture_output=True, timeout=300)
        pdf = os.path.join(d, name + ".pdf")
        subprocess.run(["pdftoppm", "-png", "-scale-to-x", str(W), "-scale-to-y", "-1", pdf, os.path.join(d, "lo")], capture_output=True, timeout=300)
    except Exception as e:
        print("LO failed", f, e)
        continue
    ks = sorted(glob.glob(os.path.join(d, "k", "*.png")))
    ls = sorted(glob.glob(os.path.join(d, "lo-*.png")))
    for i, (kp, lp) in enumerate(zip(ks, ls)):
        a = Image.open(kp).convert("RGB"); b = Image.open(lp).convert("RGB")
        if a.size != b.size:
            b = b.resize(a.size)
        diff = ImageChops.difference(a, b).convert("L")
        score = ImageStat.Stat(diff).mean[0]
        rows.append((score, name, i + 1, kp, lp, len(ks), len(ls)))
rows.sort(reverse=True)
mean = sum(r[0] for r in rows) / max(1, len(rows))
print(f"{len(rows)} slides compared, mean diff {mean:.2f}/255")
for score, name, i, kp, lp, nk, nl in rows[:15]:
    print(f"{score:6.2f}  {name} #{i}" + ("" if nk == nl else f"  (page count differs {nk} vs {nl})"))
for n, (score, name, i, kp, lp, nk, nl) in enumerate(rows[:8]):
    a = Image.open(kp).convert("RGB"); b = Image.open(lp).convert("RGB").resize(a.size)
    s = Image.new("RGB", (a.size[0] * 2 + 10, a.size[1]), "red"); s.paste(a, (0, 0)); s.paste(b, (a.size[0] + 10, 0))
    s.save(os.path.join(out, f"worst{n}_{name}_{i}.png"))
