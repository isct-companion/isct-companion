"""Export the alphabet classifier to LiteRT, the Android runtime.

The float ONNX source is rebuilt as the native PyTorch module
(`alphabet_model.load`, checked against ONNX Runtime), converted with
litert-torch with a dynamic batch of one to ten, then reduced with
ai-edge-quantizer. `--precision fp16` (the bundled model) stores float16
weights and computes in float32. `--precision int8` quantizes weights and
activations (`static_wi8_ai8`), calibrated on the same 1000 training-split
cells as the ONNX INT8 export (`alphabet_quantize.py`), never on the
validation or test splits it is judged on. The input stays NCHW
[batch, 1, 96, 96] in [-1, 1]; with one channel that is the same memory as
NHWC, and Core reads the declared shape.

litert-torch has no Windows build: run this in WSL or on macOS/Linux
(docs/verification/matrix-size-speed.md). The output is a candidate only: it
must pass `matrix_video.py evaluate` on the held-out card before it replaces
`crates/app-ocr/models/alphabet/alphabet-printed-int8.tflite`.

    python scripts/ocr/alphabet_litert.py --model FP32.onnx \\
        --data .cache/data/dataset-v6/training.npz --output INT8.tflite
"""

import argparse
import hashlib
import json
import tempfile
from pathlib import Path

import numpy as np

from alphabet_quantize import PER_CLASS, SEED


def single_channel(model):
    """The same network without the one-to-three channel expand: the three
    channels are equal, so the first convolution's weights summed over them
    give the same output from the one channel. Export-only; the converter
    cannot carry a dynamic batch through the expand or the head's Flatten."""
    import copy
    import torch
    from torch import nn

    folded = copy.deepcopy(model).eval()
    first = folded.features[0][0]
    single = nn.Conv2d(1, first.out_channels, first.kernel_size, first.stride, first.padding,
                       bias=first.bias is not None)
    single.weight.data = first.weight.data.sum(1, keepdim=True)
    if first.bias is not None:
        single.bias.data = first.bias.data.clone()
    folded.features[0][0] = single

    class Exported(nn.Module):
        def __init__(self):
            super().__init__()
            self.features, self.head = folded.features, folded.head

        def forward(self, x):
            # Global average pooling as a mean over height and width already
            # gives [batch, 1280]; the head's Flatten is skipped for the same
            # reason as the expand.
            return self.head[1:](self.features(x).mean(dim=(2, 3)))

    exported = Exported().eval()
    probe = torch.rand(4, 1, 96, 96) * 2 - 1
    with torch.no_grad():
        error = (exported(probe) - model.eval()(probe)).abs().max().item()
    if error > 1e-3:
        raise RuntimeError(f"Folded first convolution differs by {error}")
    return exported


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True, help="training.npz with train_images")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--float-output", type=Path, help="Also keep the unquantized LiteRT model")
    parser.add_argument("--precision", choices=["int8", "fp16"], default="int8",
                        help="int8: weights and activations; fp16: weights only, float compute")
    args = parser.parse_args()

    import litert_torch
    import torch
    import alphabet_model
    from ai_edge_litert.interpreter import Interpreter
    from ai_edge_quantizer import quantizer, recipe

    module = single_channel(alphabet_model.load(args.model))
    sample = torch.zeros(10, 1, 96, 96)
    batch = torch.export.Dim("batch", min=1, max=10)
    with tempfile.TemporaryDirectory() as work:
        float_model = Path(args.float_output or Path(work) / "float.tflite")
        litert_torch.convert(module, (sample,), dynamic_shapes=({0: batch},)).export(str(float_model))

        # The same calibration cells as the ONNX export, in its normalization.
        data = np.load(args.data, allow_pickle=False)
        images, labels = data["train_images"], data["train_labels"]
        rng = np.random.default_rng(SEED)
        pick = np.concatenate([
            rng.choice(np.flatnonzero(labels == c), min(PER_CLASS, int((labels == c).sum())), replace=False)
            for c in np.unique(labels)])
        cells = images[pick].astype(np.float32)[:, None] / 127.5 - 1.
        signatures = Interpreter(model_path=str(float_model)).get_signature_list()
        key, signature = next(iter(signatures.items()))
        name = signature["inputs"][0]
        calibration = {key: [{name: cells[i:i + 10]} for i in range(0, len(cells), 10)]}

        quantization = quantizer.Quantizer(str(float_model))
        if args.precision == "int8":
            quantization.load_quantization_recipe(recipe.static_wi8_ai8())
            result = quantization.quantize(quantization.calibrate(calibration))
        else:
            # Half-size weights cast back to float before each op: the model's
            # arithmetic stays float32, so it reads like the float source.
            quantization.add_weight_only_config(
                regex=".*", operation_name=recipe.TFLOperationName.ALL_SUPPORTED, num_bits=16,
                algorithm_key=recipe.AlgorithmName.FLOAT_CASTING)
            result = quantization.quantize()
        result.export_model(str(args.output), overwrite=True)

    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    print(json.dumps({"precision": args.precision,
                      "calibrationCells": len(cells) if args.precision == "int8" else 0,
                      "sourceSha256": digest(args.model),
                      "outputSha256": digest(args.output), "outputBytes": args.output.stat().st_size}))


if __name__ == "__main__":
    main()
