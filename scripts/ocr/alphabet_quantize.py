"""Static INT8 (QDQ) quantization of the alphabet classifier for ONNX Runtime.

Weights are quantized per channel, activations per tensor with a 99.99th
percentile range calibrated on 40 cells per class drawn from the TRAINING
split of a camera-path dataset (`matrix_video.py build`), never from the
validation or test splits it is judged on. The output is a candidate only:
it must pass `matrix_video.py evaluate` on the held-out card, the Rust/Python
parity check (`alphabet_synthetic.py --check-model`) and session replay
before it replaces the bundled model.

    python scripts/ocr/alphabet_quantize.py --model FP32.onnx \
        --data .cache/data/dataset-v6/training.npz --output INT8.onnx
"""

import argparse
import hashlib
import json
import tempfile
from pathlib import Path

import numpy as np
import onnx
from onnxruntime.quantization import (CalibrationDataReader, CalibrationMethod, QuantFormat,
                                      QuantType, quantize_static)
from onnxruntime.quantization.shape_inference import quant_pre_process

PER_CLASS = 40
SEED = 20261108


class Cells(CalibrationDataReader):
    def __init__(self, name, images):
        self.name, self.images, self.at = name, images, 0

    def get_next(self):
        if self.at >= len(self.images):
            return None
        batch = self.images[self.at:self.at + 10]
        self.at += 10
        return {self.name: batch}


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True, help="training.npz with train_images")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    data = np.load(args.data, allow_pickle=False)
    images, labels = data["train_images"], data["train_labels"]
    rng = np.random.default_rng(SEED)
    pick = np.concatenate([
        rng.choice(np.flatnonzero(labels == c), min(PER_CLASS, int((labels == c).sum())), replace=False)
        for c in np.unique(labels)])
    # The production input: Rec.601 grayscale normalized to [-1, 1].
    cells = images[pick].astype(np.float32)[:, None] / 127.5 - 1.
    name = onnx.load(str(args.model)).graph.input[0].name
    with tempfile.TemporaryDirectory() as work:
        prepared = Path(work) / "prepared.onnx"
        quant_pre_process(str(args.model), str(prepared))
        quantize_static(str(prepared), str(args.output), Cells(name, cells),
                        quant_format=QuantFormat.QDQ, activation_type=QuantType.QUInt8,
                        weight_type=QuantType.QInt8, per_channel=True,
                        calibrate_method=CalibrationMethod.Percentile,
                        extra_options={"CalibPercentile": 99.99})
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    print(json.dumps({"calibrationCells": len(cells), "sourceSha256": digest(args.model),
                      "outputSha256": digest(args.output), "outputBytes": args.output.stat().st_size}))


if __name__ == "__main__":
    main()
