"""Historical raw-cell ablation (PIL geometry), NOT production pipeline parity.

For model promotion use matrix_lighting/matrix_letter_boundaries with the Rust
matrix_capture replay. For training use alphabet_synthetic --prepare-binary;
this script's 9% crop is intentionally retained only for old ablation records.
"""

import argparse, hashlib, json, time
from pathlib import Path
import numpy as np
import onnxruntime as ort

ort.disable_telemetry_events()
from PIL import Image, ImageOps


def rectify(image, quads, size=96, margin=0.09, padding=0):
    cells = []
    for quad in quads:
        a, b, c, d = np.asarray(quad, dtype=float) * [image.width, image.height]
        dx, dy = a - b + c - d
        bx, by = b - c
        cx, cy = d - c
        det = bx * cy - cx * by
        g, h = (
            ((dx * cy - cx * dy) / det, (bx * dy - dx * by) / det)
            if abs(det) > 1e-8
            else (0.0, 0.0)
        )
        ax, ay = b - a + g * b
        ex, ey = d - a + h * d
        scale = (1 - 2 * margin) / (size - 2 * padding - 1)
        z = 1 + margin * (g + h)
        coefficients = (
            ax * scale / z,
            ex * scale / z,
            (a[0] + margin * (ax + ex)) / z,
            ay * scale / z,
            ey * scale / z,
            (a[1] + margin * (ay + ey)) / z,
            g * scale / z,
            h * scale / z,
        )
        crop = image.transform(
            (size - 2 * padding, size - 2 * padding),
            Image.Transform.PERSPECTIVE,
            coefficients,
            Image.Resampling.BILINEAR,
        ).convert("L")
        if padding:
            crop = ImageOps.expand(
                crop, border=padding, fill=int(np.median(np.asarray(crop)))
            )
        cells.append(crop)
    return cells


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--image", required=True)
    p.add_argument("--geometry", required=True)
    p.add_argument("--expected", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--invert", action="store_true")
    p.add_argument("--margin", type=float, default=0.09)
    p.add_argument("--padding", type=int, default=0)
    args = p.parse_args()
    start = time.perf_counter()
    image = Image.open(args.image).convert("RGB")
    cells = rectify(
        image,
        json.loads(Path(args.geometry).read_text()),
        margin=args.margin,
        padding=args.padding,
    )
    if args.invert:
        cells = [ImageOps.invert(c) for c in cells]
    x = np.stack([np.asarray(c, dtype=np.float32) / 127.5 - 1 for c in cells])[:, None]
    prep = (time.perf_counter() - start) * 1000
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 2
    opts.inter_op_num_threads = 1
    session = ort.InferenceSession(
        args.model, sess_options=opts, providers=["CPUExecutionProvider"]
    )
    input = session.get_inputs()[0]
    print(
        "input:",
        input.name,
        input.shape,
        "output:",
        session.get_outputs()[0].shape,
        flush=True,
    )
    batch = 1 if isinstance(input.shape[0], int) else 10

    def infer():
        return np.concatenate(
            [
                session.run(None, {input.name: x[i : i + batch]})[0]
                for i in range(0, len(x), batch)
            ]
        )

    infer()
    times = []
    for _ in range(5):
        start = time.perf_counter()
        logits = infer()
        times.append((time.perf_counter() - start) * 1000)
    logits -= logits.max(axis=1, keepdims=True)
    prob = np.exp(logits)
    prob /= prob.sum(axis=1, keepdims=True)
    pred = prob.argmax(axis=1)
    score = prob.max(axis=1)
    expected = Path(args.expected).read_text().strip()
    assert len(expected) == len(cells) == 70
    labels = "ABCDEFGHIJKLMNOPQRSTUVWXYZ_"
    results = [
        dict(
            cell=f"{chr(65 + i % 10)}{i // 10 + 1}",
            expected=expected[i],
            predicted=labels[v],
            confidence=float(score[i]),
        )
        for i, v in enumerate(pred)
    ]
    correct = sum(r["predicted"] == r["expected"] for r in results)
    qualified = [r for r in results if r["confidence"] >= 0.8 and r["predicted"] != "_"]
    report = dict(
        modelSha256=hashlib.sha256(Path(args.model).read_bytes()).hexdigest(),
        sourceSha256=hashlib.sha256(Path(args.image).read_bytes()).hexdigest(),
        runtime=ort.__version__,
        invert=args.invert,
        margin=args.margin,
        padding=args.padding,
        batch=batch,
        correct=correct,
        total=70,
        qualified=len(qualified),
        wrongQualified=sum(r["predicted"] != r["expected"] for r in qualified),
        preprocessMs=prep,
        inferenceMeanMs=float(np.mean(times)),
        inferenceP95Ms=float(np.percentile(times, 95)),
        results=results,
    )
    Path(args.output).write_text(json.dumps(report, indent=2))
    print(json.dumps({k: v for k, v in report.items() if k != "results"}), flush=True)
    np.savez(
        Path(args.output).with_suffix(".npz"),
        images=np.stack([np.asarray(c) for c in cells]),
        labels=np.array([ord(c) - 65 for c in expected]),
    )


if __name__ == "__main__":
    main()
