"""Generate grayscale printed-letter training data without real-card samples."""

import argparse, json, random
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter


def render(rng, fonts, label, lighting=False):
    size = 144
    background, ink = rng.randint(165, 255), rng.randint(0, 95)
    image = Image.new("L", (size, size), background)
    if label < 26:
        font = ImageFont.truetype(str(rng.choice(fonts)), 180)
        text = chr(65 + label)
        box = font.getbbox(text)
        glyph = Image.new("L", (box[2] - box[0] + 4, box[3] - box[1] + 4), background)
        ImageDraw.Draw(glyph).text((2 - box[0], 2 - box[1]), text, font=font, fill=ink)
        height = rng.randint(88, 137)
        width = min(
            135,
            max(8, round(glyph.width * height / glyph.height * rng.uniform(0.8, 1.12))),
        )
        glyph = glyph.resize((width, height), Image.Resampling.LANCZOS)
        image.paste(
            glyph,
            (
                (size - width) // 2 + rng.randint(-6, 6),
                (size - height) // 2 + rng.randint(-5, 5),
            ),
        )
        image = image.rotate(
            rng.uniform(-12, 12),
            resample=Image.Resampling.BICUBIC,
            fillcolor=background,
        )
    else:
        # Non-character fragments and stripes seen when geometry finds background.
        draw = ImageDraw.Draw(image)
        if rng.random() < 0.65:
            for _ in range(rng.randint(1, 6)):
                x, y = rng.randint(0, 140), rng.randint(0, 140)
                draw.rectangle(
                    (
                        x,
                        y,
                        min(143, x + rng.randint(2, 65)),
                        min(143, y + rng.randint(2, 100)),
                    ),
                    fill=rng.randint(20, 140),
                )
    draw = ImageDraw.Draw(image)
    if rng.random() < 0.3:
        edge = rng.choice(["top", "bottom", "left", "right"])
        thickness = rng.randint(1, 5)
        coords = {
            "top": (0, 0, size, thickness),
            "bottom": (0, size - thickness, size, size),
            "left": (0, 0, thickness, size),
            "right": (size - thickness, 0, size, size),
        }
        draw.rectangle(coords[edge], fill=rng.randint(0, 85))
    if rng.random() < 0.15:
        arr = np.asarray(image, dtype=np.float32).copy()
        arr += np.linspace(0, rng.uniform(0, 65), size)[None, :]
        image = Image.fromarray(np.clip(arr, 0, 255).astype("uint8"))
    # Camera cells often contain only 20–40 source pixels before normalization.
    width = rng.randint(24, 96)
    height = rng.randint(20, 80)
    image = image.resize((width, height), Image.Resampling.LANCZOS).resize(
        (96, 96), Image.Resampling.BILINEAR
    )
    array = np.asarray(image, dtype=np.float32)
    motion = rng.randint(1, 7)
    if rng.random() < 0.16:
        motion = rng.randint(13, 29)
        label = 26  # Intentionally unreadable drag must be rejected, not guessed.
    if motion > 1:
        axis = rng.randint(0, 1)
        padded = np.pad(
            array,
            ((motion, 0), (0, 0)) if axis == 0 else ((0, 0), (motion, 0)),
            mode="edge",
        )
        integral = np.cumsum(padded, axis=axis)
        array = (
            (integral[motion:] - integral[:-motion]) / motion
            if axis == 0
            else (integral[:, motion:] - integral[:, :-motion]) / motion
        )
    image = Image.fromarray(np.clip(array, 0, 255).astype("uint8")).filter(
        ImageFilter.GaussianBlur(rng.uniform(0, 1.2))
    )
    if lighting and rng.random() < 0.8:
        array = np.asarray(image, dtype=np.float32).copy()
        y, x = np.mgrid[:96, :96].astype(np.float32) / 95
        condition = rng.choice(("dim", "shadow", "glare", "gradient"))
        if condition == "dim":
            array *= rng.uniform(0.2, 0.65)
        elif condition == "shadow":
            axis = x * rng.uniform(-1, 1) + y * rng.uniform(-1, 1)
            shade = rng.uniform(0.15, 0.45)
            array *= shade + (1 - shade) / (1 + np.exp(
                -(axis - rng.uniform(-0.5, 0.5)) * rng.uniform(8, 40)
            ))
        elif condition == "gradient":
            array *= rng.uniform(0.2, 0.6) + x * rng.uniform(0.3, 0.6)
        else:
            spot = np.exp(-((x - rng.random()) / rng.uniform(0.18, 0.6)) ** 2
                          - ((y - rng.random()) / rng.uniform(0.18, 0.6)) ** 2)
            original_ink = array < (ink + background) / 2
            array += rng.uniform(100, 420) * spot
            # Once glare erases substantial ink, train rejection instead of
            # teaching the classifier to invent the missing strokes.
            erased = original_ink & (array >= 245)
            if erased.sum() > max(1, original_ink.sum()) * 0.2:
                label = 26
        array += np.random.default_rng(rng.getrandbits(32)).normal(0, rng.uniform(0, 3), array.shape)
        image = Image.fromarray(np.clip(array, 0, 255).astype(np.uint8))
    return np.asarray(image, dtype=np.uint8), label


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--fonts", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--train", type=int, default=26000)
    p.add_argument("--validation", type=int, default=2600)
    p.add_argument("--lighting", action="store_true")
    a = p.parse_args()
    fonts = sorted(f for f in Path(a.fonts).rglob("*") if f.suffix.lower() in (".ttf", ".ttc"))
    train = [f for f in fonts if not f.name.lower().startswith(("verdana", "trebuchet"))]
    valid = [f for f in fonts if f not in train]
    if not train or not valid:
        raise ValueError("Need train fonts and held-out Verdana/Trebuchet fonts")
    data = {}
    metadata = {}
    for split, count, choices, seed in [
        ("train", a.train, train, 20261017),
        ("validation", a.validation, valid, 20261018),
    ]:
        rng = random.Random(seed)
        labels = np.array(
            [26 if i % 10 == 0 else i % 26 for i in range(count)], dtype=np.int64
        )
        rendered = [render(rng, choices, int(y), a.lighting) for y in labels]
        images = np.stack([r[0] for r in rendered])
        labels = np.array([r[1] for r in rendered], dtype=np.int64)
        data[split + "_images"] = images
        data[split + "_labels"] = labels
        metadata[split] = {
            "count": count,
            "fonts": [f.name for f in choices],
            "seed": seed,
            "lighting": a.lighting,
        }
        print(split, count, flush=True)
    np.savez_compressed(a.output, **data)
    Path(a.output).with_suffix(".json").write_text(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
