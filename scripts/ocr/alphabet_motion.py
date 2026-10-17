"""Evaluate recorded motion frames locally; no capture data is uploaded."""

import argparse
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image

from alphabet_eval import rectify


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--directory", required=True)
    parser.add_argument("--expected", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    ort.disable_telemetry_events()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 2
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(
        args.model, sess_options=options, providers=["CPUExecutionProvider"]
    )
    expected = Path(args.expected).read_text().strip()
    labels = np.array([ord(c) - 65 for c in expected])
    directory = Path(args.directory)
    reports = []
    for geometry in sorted(directory.glob("geometry-*.json")):
        image = directory / geometry.name.replace("geometry-", "frame-").replace(
            ".json", ".jpg"
        )
        if not image.exists():
            continue
        cells = rectify(
            Image.open(image).convert("RGB"), json.loads(geometry.read_text())
        )
        batch = np.stack([np.asarray(c, dtype=np.float32) / 127.5 - 1 for c in cells])[
            :, None
        ]
        logits = np.concatenate(
            [
                session.run(None, {"input": batch[i : i + 10]})[0]
                for i in range(0, len(batch), 10)
            ]
        )
        logits -= logits.max(1, keepdims=True)
        probabilities = np.exp(logits)
        probabilities /= probabilities.sum(1, keepdims=True)
        predicted = probabilities.argmax(1)
        confidence = probabilities.max(1)
        qualified = (confidence >= 0.8) & (predicted != 26)
        reports.append(
            dict(
                frame=int(geometry.stem.split("-")[1]),
                correct=int((predicted == labels).sum()),
                qualified=int(qualified.sum()),
                wrongQualified=int((qualified & (predicted != labels)).sum()),
                predicted=predicted.tolist(),
                confidence=confidence.tolist(),
            )
        )
    Path(args.output).write_text(json.dumps(reports, indent=2))
    print(
        json.dumps(
            dict(
                frames=len(reports),
                correct=sum(r["correct"] for r in reports),
                qualified=sum(r["qualified"] for r in reports),
                wrongQualified=sum(r["wrongQualified"] for r in reports),
            )
        )
    )


if __name__ == "__main__":
    main()
