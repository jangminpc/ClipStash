"""앱 아이콘(res/app.ico, res/paused.ico)을 그려서 만듭니다. Pillow 필요: pip install pillow"""
from PIL import Image, ImageDraw
import os


def draw(size, paused=False):
    s = size * 4  # 크게 그린 뒤 줄여서 부드럽게
    im = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    u = s / 32
    board = (110, 120, 130) if paused else (38, 110, 220)
    paper = (245, 247, 250)
    # 클립보드 판
    d.rounded_rectangle([4*u, 4*u, 28*u, 31*u], radius=3*u, fill=board)
    # 종이(겹친 두 장 → 기록 느낌)
    d.rounded_rectangle([9*u, 9*u, 26*u, 29*u], radius=1.5*u, fill=(200, 210, 225))
    d.rounded_rectangle([7*u, 7*u, 24*u, 27*u], radius=1.5*u, fill=paper)
    # 집게
    d.rounded_rectangle([11*u, 1.5*u, 21*u, 7.5*u], radius=1.5*u, fill=(60, 64, 72))
    d.ellipse([14.5*u, 2.5*u, 17.5*u, 5.5*u], fill=paper)
    # 글줄
    line = (150, 158, 170)
    for y in (12, 16, 20):
        d.rounded_rectangle([10*u, y*u, (21 if y != 20 else 17)*u, (y+1.6)*u], radius=0.8*u, fill=line)
    if paused:
        d.rectangle([17*u, 19*u, 19.5*u, 28*u], fill=(220, 60, 50))
        d.rectangle([22*u, 19*u, 24.5*u, 28*u], fill=(220, 60, 50))
    return im.resize((size, size), Image.LANCZOS)


here = os.path.join(os.path.dirname(__file__), "..", "res")
for name, paused in (("app.ico", False), ("paused.ico", True)):
    sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    big = draw(256, paused)
    imgs = [draw(n, paused) for n in sizes]
    big.save(os.path.join(here, name), sizes=[(n, n) for n in sizes], append_images=imgs)
    if not paused:
        draw(128).save(os.path.join(here, "icon-preview.png"))  # README 용
print("ok")
