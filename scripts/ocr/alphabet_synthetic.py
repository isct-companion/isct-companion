"""Generate grayscale printed-letter training data without real-card samples."""

import argparse, hashlib, json, random, subprocess
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter


def prepare(images, binary):
    records = []
    for start in range(0, len(images), 256):
        chunk = images[start:start + 256]
        result = subprocess.run([str(binary.resolve())], input=chunk.tobytes(),
                                capture_output=True, check=True)
        if len(result.stdout) != len(chunk) * (1 + 2 * 96 * 96):
            raise RuntimeError("Invalid preprocessing stream length")
        records.append(np.frombuffer(result.stdout, dtype=np.uint8).reshape(len(chunk), -1))
    records = np.concatenate(records)
    return (records[:, 0], records[:, 1:9217].reshape(-1, 96, 96),
            records[:, 9217:].reshape(-1, 96, 96))


def check_preprocessing(fonts, binary, model, output):
    import onnxruntime as ort
    from alphabet_calibration import fuse, infer
    rng = random.Random(20261023)
    cells = np.stack([render(rng, fonts, i % 27, lighting=True)[0] for i in range(512)])
    flags, original, corrected = prepare(cells, binary)
    result = subprocess.run([str(binary.resolve()), "--recognize"],
                            input=cells.tobytes(), capture_output=True, check=True)
    rust = [json.loads(line)[0] for line in result.stdout.splitlines()]
    options = ort.SessionOptions()
    options.intra_op_num_threads = 2
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(str(model), options, providers=["CPUExecutionProvider"])
    predictions, scores, conflicts = fuse(infer(session, original), infer(session, corrected), flags)
    texts = ["" if value == 26 else chr(65 + value) for value in predictions]
    if len(rust) != len(cells) or texts != [r["text"] for r in rust]:
        raise RuntimeError("Rust/Python prediction parity failed")
    error = float(np.max(np.abs(scores - [r["confidence"] for r in rust])))
    if error > 1e-5:
        raise RuntimeError(f"Rust/Python confidence parity failed: {error}")
    report = dict(sources=len(cells), rejected=int((flags == 0).sum()),
                  corrected=int((flags == 2).sum()), conflicts=int(conflicts.sum()),
                  confidenceMaxAbsError=error, lettersIdentical=True,
                  modelSha256=hashlib.sha256(model.read_bytes()).hexdigest(),
                  preparationSha256=hashlib.sha256(binary.read_bytes()).hexdigest())
    Path(output).write_text(json.dumps(report, indent=2))
    print(json.dumps(report))


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
        # Blur width alone cannot establish that a glyph is unreadable. The
        # production-prepared dataset quarantines it instead of teaching "blank".
        if label != 26:
            label = -1
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
            # Erased area does not tell us whether the remaining strokes are
            # identifiable. Quarantine these labels instead of inventing truth.
            erased = original_ink & (array >= 245)
            if label != 26 and erased.sum() > max(1, original_ink.sum()) * 0.2:
                label = -1
        array += np.random.default_rng(rng.getrandbits(32)).normal(0, rng.uniform(0, 3), array.shape)
        image = Image.fromarray(np.clip(array, 0, 255).astype(np.uint8))
    return np.asarray(image, dtype=np.uint8), label


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--fonts", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--train", type=int, default=26000)
    p.add_argument("--validation", type=int, default=2600)
    p.add_argument("--seed", type=int, default=20261017)
    p.add_argument("--lighting", action="store_true")
    p.add_argument("--prepare-binary", type=Path, required=True,
                   help="Production Rust alphabet_prepare executable")
    p.add_argument("--check-model", type=Path,
                   help="Only check Rust/Python prediction parity on 512 public synthetic cells")
    a = p.parse_args()
    fonts = sorted(f for f in Path(a.fonts).rglob("*") if f.suffix.lower() in (".ttf", ".ttc"))
    if a.check_model:
        check_preprocessing(fonts, a.prepare_binary, a.check_model, a.output)
        return
    held_out = ("verdana", "trebuchet", "georgia")
    train = [f for f in fonts if not f.name.lower().startswith(held_out)]
    valid = [f for f in fonts if f.name.lower().startswith("verdana")]
    if not train or not valid:
        raise ValueError("Need train fonts and held-out Verdana/Trebuchet fonts")
    data = {}
    metadata = {}
    splits = [
        ("train", a.train, train, a.seed),
        ("validation", a.validation, valid, a.seed + 1),
        ("calibration", a.validation, [f for f in fonts if f.name.lower().startswith("trebuchet")], a.seed + 4),
        ("test", a.validation, [f for f in fonts if f.name.lower().startswith("georgia")], a.seed + 5),
    ]
    for split, count, choices, seed in splits:
        if not choices:
            raise ValueError(f"No fonts for {split}")
        rng = random.Random(seed)
        labels = np.array(
            [26 if i % 10 == 0 else i % 26 for i in range(count)], dtype=np.int64
        )
        rendered = [render(rng, choices, int(y), a.lighting and i % 2 == 0)
                    for i, y in enumerate(labels)]
        images = np.stack([r[0] for r in rendered])
        labels = np.array([r[1] for r in rendered], dtype=np.int64)
        # Batches bound memory and preserve one source observation across its
        # correlated raw/corrected views. Rust owns all preprocessing.
        flags, images, corrected = prepare(images, a.prepare_binary)
        # A normalized rectangle/fragment can be pixel-identical to sans-serif
        # I/L/T. Synthetic provenance cannot make those pixels a known blank.
        # Keep only explicit no-glyph rejections as certain blank labels.
        labels[(labels == 26) & (flags > 0)] = -1
        ambiguous = int((labels < 0).sum())
        if split == "train":
            keep, dual = (flags > 0) & (labels >= 0), (flags == 2) & (labels >= 0)
            images = np.concatenate((images[keep], corrected[dual]))
            labels = np.concatenate((labels[keep], labels[dual]))
        else:
            data[split + "_corrected_images"] = corrected
            data[split + "_flags"] = flags
            # Keep rejected and ambiguous sources visible in evaluation. Only
            # known labels contribute to accuracy/NLL; ambiguity has its own count.
        metadata[split] = {"rejectedSources": int((flags == 0).sum()),
                           "correctedSources": int((flags == 2).sum())}
        data[split + "_images"] = images
        data[split + "_labels"] = labels
        metadata.setdefault(split, {}).update({
            "sourceCount": count, "modelViews": len(images),
            "ambiguousSources": ambiguous,
            "fonts": [f.name for f in choices],
            "fontSha256": [hashlib.sha256(f.read_bytes()).hexdigest() for f in choices],
            "seed": seed,
            "lighting": a.lighting,
        })
        print(split, metadata[split], flush=True)
    metadata["preparationSha256"] = hashlib.sha256(a.prepare_binary.read_bytes()).hexdigest()
    metadata["lightingSourceFraction"] = 0.5 if a.lighting else 0
    metadata["ambiguousLabel"] = -1
    metadata["generatorSha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    np.savez_compressed(a.output, **data)
    Path(a.output).with_suffix(".json").write_text(json.dumps(metadata, indent=2))


if __name__ == "__main__":
    main()
