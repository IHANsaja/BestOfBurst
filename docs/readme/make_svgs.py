"""Generate the README artwork in the Grease Pencil style.

GitHub shows SVGs as images without web fonts, so all text is converted to vector paths
from the bundled Archivo and Martian Mono fonts. Run from the repository root:

    python docs/readme/make_svgs.py
"""
import math
import os

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "docs", "readme")
FONTS = os.path.join(ROOT, "assets", "fonts")

INK, TABLE, FILM, GRAPHITE, RED, SAFE, EDGE, ASH = "#121211", "#E6E4DE", "#1B1A18", "#5C5A55", "#D42A1C", "#FF5A40", "#D9A441", "#A9A69D"
RULE_DARK, RULE_DAY, INK2 = "#2E2D2A", "#C4C1B8", "#3C3B37"

_fonts = {}


def font(name):
    if name not in _fonts:
        f = TTFont(os.path.join(FONTS, name))
        _fonts[name] = (f, f.getGlyphSet(), f.getBestCmap(), f["head"].unitsPerEm, f["hmtx"])
    return _fonts[name]


XP = "Archivo-Expanded-ExtraBold.ttf"
XP9 = "Archivo-Expanded-Black.ttf"
BODY = "Archivo-Regular.ttf"
MONO = "MartianMono-Condensed-Medium.ttf"


def text_width(fname, text, size, spacing=0.0):
    _, _, cmap, upm, hmtx = font(fname)
    w = 0.0
    for ch in text:
        g = cmap.get(ord(ch))
        if g:
            w += hmtx[g][0] * size / upm + spacing
    return w - spacing if text else 0.0


def text_path(fname, text, size, x, y, fill, spacing=0.0, anchor="start"):
    """SVG <path> for `text` with its baseline at y."""
    _, gs, cmap, upm, hmtx = font(fname)
    if anchor == "middle":
        x -= text_width(fname, text, size, spacing) / 2
    elif anchor == "end":
        x -= text_width(fname, text, size, spacing)
    s = size / upm
    pen = SVGPathPen(gs)
    cx = x
    for ch in text:
        g = cmap.get(ord(ch))
        if not g:
            continue
        tp = TransformPen(pen, (s, 0, 0, -s, cx, y))
        gs[g].draw(tp)
        cx += hmtx[g][0] * s + spacing
    return f'<path d="{pen.getCommands()}" fill="{fill}"/>'


LOOP = "M22 60 C 10 18, 90 4, 100 40 C 108 74, 44 98, 14 74 C 2 62, 12 36, 36 26"


def loop(x, y, w, h, color, width):
    """The hand-drawn keeper loop stretched over a box (preserveAspectRatio none)."""
    sx, sy = w / 120.0, h / 100.0
    return (f'<path transform="translate({x:.1f},{y:.1f}) scale({sx:.4f},{sy:.4f})" d="{LOOP}" fill="none" '
            f'stroke="{color}" stroke-width="{width / ((sx + sy) / 2):.2f}" stroke-linecap="round"/>')


def burst(x, y, size, dark):
    """The BestOfBurst logo (F1 dark / F2 light) at (x, y)."""
    bg, ray, lit, eye, dot = (INK, GRAPHITE, SAFE, TABLE, SAFE) if dark else (TABLE, INK, RED, INK, RED)
    k = size / 135.0
    rays = "".join(
        f'<polygon points="-4,-27 4,-27 7,-52 -7,-52" transform="rotate({a})" fill="{lit if a == 30 else ray}"/>'
        for a in range(0, 360, 30)
    )
    return f'''<g transform="translate({x},{y}) scale({k:.4f})">
<rect width="135" height="135" fill="{bg}"/>
<g transform="translate(67.5,67.5)">{rays}
<circle r="17" fill="none" stroke="{eye}" stroke-width="3"/>
<polygon points="7.6,2.4 1.8,7.8 -5.9,5.4 -7.6,-2.4 -1.8,-7.8 5.9,-5.4" fill="none" stroke="{eye}" stroke-width="2"/>
<circle r="2.6" fill="{dot}"/></g>
<path transform="translate(87.5,32.9) rotate(30) scale(0.36,0.58) translate(-55,-51)" d="{LOOP}" fill="none" stroke="{lit}" stroke-width="5.2" stroke-linecap="round"/>
</g>'''


def svg(w, h, body, title):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" role="img">'
            f'<title>{title}</title>{body}</svg>\n')


def write(name, content):
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        f.write(content)


def hero(dark):
    W, H = 1280, 420
    bg, fg, muted, mark = (INK, TABLE, ASH, SAFE) if dark else (TABLE, INK, GRAPHITE, RED)
    parts = [f'<rect width="{W}" height="{H}" fill="{bg}"/>']
    parts.append(burst(72, 110, 200, dark))
    x0 = 340
    parts.append(text_path(MONO, "BESTOFBURST  ·  PHOTO CULLING FOR WINDOWS", 15, x0, 112, muted, 2.2))
    avail = W - x0 - 64
    size = 64.0
    while text_width(XP, "TEN THOUSAND FRAMES.", size, -size * 0.025) > avail:
        size -= 1
    sp = -size * 0.025
    parts.append(text_path(XP, "TEN THOUSAND FRAMES.", size, x0, 196, fg, sp))
    xs = x0 + size * 0.45
    cw = text_width(XP, "CIRCLE", size, sp)
    base = 196 + size * 1.15
    parts.append(text_path(XP, "CIRCLE", size, xs, base, fg, sp))
    parts.append(text_path(XP, " THE BEST.", size, xs + cw, base, fg, sp))
    # The drawn loop covers about x 6-87 % and y 20-85 % of its box; leave a margin round the word.
    cap = size * 0.72
    m = size * 0.22
    lw = (cw + 2 * m) / 0.81
    lh = (cap + 2 * m) / 0.65
    parts.append(loop(xs - m - 0.06 * lw, base - cap - m - 0.20 * lh, lw, lh, mark, 4.5))
    parts.append(text_path(BODY, "Drop a shoot. Get the best frame of every moment, circled and ready.", 22, x0, 340, muted))
    parts.append(text_path(BODY, "Offline on your PC. Your originals are never touched.", 22, x0, 372, muted))
    parts.append(f'<rect x="0" y="{H - 6}" width="{W}" height="6" fill="{mark}"/>')
    return svg(W, H, "".join(parts), "BestOfBurst: ten thousand frames, circle the best")


def wave(x, y, w, color, width):
    sx = w / 120.0
    d = "M4 12 C 20 2, 34 2, 46 10 C 58 18, 70 20, 84 10 C 98 0, 106 2, 116 8"
    return (f'<path transform="translate({x},{y}) scale({sx:.4f},1)" d="{d}" fill="none" stroke="{color}" '
            f'stroke-width="{width}" stroke-linecap="round"/>')


def marks():
    W, H = 1280, 300
    parts = [f'<rect width="{W}" height="{H}" fill="{TABLE}"/>']
    cells = [("KEEPER", "Circled: the best of the moment"), ("MAYBE", "Good, but not the pick"),
             ("SET ASIDE", "Out of focus, too dark..."), ("CLOSE CALL", "Nearly equal: your eye decides")]
    cw = W / 4
    for i, (title, sub) in enumerate(cells):
        x = i * cw
        fx, fy, fw, fh = x + 52, 34, cw - 104, 150
        parts.append(f'<rect x="{fx}" y="{fy}" width="{fw}" height="{fh}" fill="{FILM}"/>')
        for k in range(int(fw // 14)):
            parts.append(f'<rect x="{fx + 8 + k * 14}" y="{fy + 5}" width="6" height="2.5" fill="#3A3936"/>')
            parts.append(f'<rect x="{fx + 8 + k * 14}" y="{fy + fh - 8}" width="6" height="2.5" fill="#3A3936"/>')
        px, py, pw, ph = fx + 14, fy + 16, fw - 28, fh - 40
        op = ' opacity="0.45"' if title == "SET ASIDE" else ""
        parts.append(f'<g{op}><rect x="{px}" y="{py}" width="{pw}" height="{ph}" fill="#5C5A55"/>'
                     f'<rect x="{px}" y="{py + ph * 0.55}" width="{pw}" height="{ph * 0.45}" fill="#3C3B37"/>'
                     f'<circle cx="{px + pw * 0.72}" cy="{py + ph * 0.32}" r="{ph * 0.14:.1f}" fill="{EDGE}"/></g>')
        if title == "KEEPER":
            parts.append(loop(px - 10, py - 8, pw + 20, ph + 16, SAFE, 4))
        elif title == "MAYBE":
            parts.append(wave(px + pw * 0.1, py + ph - 2, pw * 0.8, SAFE, 3.5))
        elif title == "SET ASIDE":
            parts.append(f'<path d="M{px + 8} {py + 6} C {px + pw * 0.35} {py + ph * 0.25}, {px + pw * 0.65} {py + ph * 0.7}, {px + pw - 8} {py + ph - 6}" fill="none" stroke="{TABLE}" stroke-width="3.5" stroke-linecap="round"/>')
            parts.append(f'<path d="M{px + pw - 8} {py + 6} C {px + pw * 0.65} {py + ph * 0.3}, {px + pw * 0.35} {py + ph * 0.72}, {px + 8} {py + ph - 6}" fill="none" stroke="{TABLE}" stroke-width="3.5" stroke-linecap="round"/>')
        else:
            parts.append(loop(px - 10, py - 8, pw + 20, ph + 16, SAFE, 4))
            parts.append(text_path(XP9, "?", 34, fx + fw - 10, fy + 40, SAFE, anchor="end"))
        parts.append(text_path(MONO, f"▸ {4821 + i}", 11, fx + fw - 10, fy + fh - 13, EDGE, 1.0, anchor="end")
                     if False else text_path(MONO, f"{4821 + i}", 11, fx + fw - 10, fy + fh - 13, EDGE, 1.0, anchor="end"))
        parts.append(text_path(XP, title, 20, x + cw / 2, 232, INK, 0.6, anchor="middle"))
        parts.append(text_path(BODY, sub, 16, x + cw / 2, 260, INK2, anchor="middle"))
    return svg(W, H, "".join(parts), "The marks: keeper, maybe, set aside, close call")


def pipeline(dark):
    W, H = 1280, 230
    bg, fg, muted, rule, mark = (INK, TABLE, ASH, RULE_DARK, SAFE) if dark else (TABLE, INK, GRAPHITE, RULE_DAY, RED)
    steps = [("01", "FIND PHOTOS", "JPEG · RAW · HEIC · PNG · JXL"), ("02", "CHECK QUALITY", "Focus · exposure · noise · colour"),
             ("03", "STACK BURSTS", "Same moment, one stack"), ("04", "PICK THE BEST", "Best frame circled, ready")]
    parts = [f'<rect width="{W}" height="{H}" fill="{bg}"/>']
    cw = W / 4
    for i, (n, t, d) in enumerate(steps):
        x = i * cw
        if i:
            parts.append(f'<rect x="{x}" y="36" width="1" height="{H - 72}" fill="{rule}"/>')
        parts.append(text_path(MONO, n, 14, x + 36, 76, mark, 2.0))
        parts.append(text_path(XP, t, 22, x + 36, 124, fg, 0.4))
        parts.append(text_path(BODY, d, 17, x + 36, 160, muted))
        if i < 3:
            ax = x + cw - 26
            parts.append(f'<path d="M{ax - 22} 71 L{ax} 71 M{ax - 7} 64 L{ax} 71 L{ax - 7} 78" fill="none" stroke="{mark}" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>')
    return svg(W, H, "".join(parts), "How it works: find photos, check quality, stack bursts, pick the best")


os.makedirs(OUT, exist_ok=True)
write("hero-dark.svg", hero(True))
write("hero-light.svg", hero(False))
write("marks.svg", marks())
write("pipeline-dark.svg", pipeline(True))
write("pipeline-light.svg", pipeline(False))
print("written to", OUT)
