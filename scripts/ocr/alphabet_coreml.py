"""Export the alphabet classifier to Core ML, the macOS and iOS runtime.

The float ONNX source is rebuilt as the native PyTorch module
(`alphabet_model.load`, checked against ONNX Runtime), with the one-to-three
channel expand folded into the first convolution (`alphabet_litert.
single_channel`), traced for a fixed batch of ten and converted with
coremltools to an ML Program for iOS 17 / macOS 14 that computes in float32 and stores float16
weights: every float32 weight becomes a `constexpr_cast` of its float16
rounding, expanded once when the model loads, as the LiteRT export's
FLOAT_CASTING does. Computing in float16 instead (`ct.precision.FLOAT16`)
read like the float source on the Neural Engine but not on the CPU, where one
correct validation cell fell below the 0.8 vote threshold; Core runs this
model on the CPU on every device, Intel Macs included.

Every weight is written inline into the one specification file instead of a
`weights/weight.bin` blob, because Core loads the model from memory
(`MLModelAsset(specification:)`), and an in-memory ML Program must not
reference a blob file. The input stays NCHW in [-1, 1], named `cells`, but its
batch is fixed at ten, Core's batch: with a flexible batch (`ct.RangeDim`)
Core ML failed to resize for a batch smaller than the default ("Error in
dynamically resizing for sequence length"). Core pads a smaller batch and
reads its rows; inference keeps the rows independent. The output is
`logits`, [10, 27].

The output is a candidate only: it must pass `matrix_video.py evaluate` and the
model promotion gate (`crates/app-ocr/models/alphabet/README.md`) before it
replaces `crates/app-ocr/models/alphabet/alphabet-printed-fp16.mlmodel`.
Converting needs macOS (coremltools only predicts there):

    python scripts/ocr/alphabet_coreml.py --model FP32.onnx --output CANDIDATE.mlmodel
"""

import argparse
import hashlib
import json
import tempfile
from pathlib import Path

import numpy as np

from alphabet_litert import single_channel

# The oldest system the Core SDK declares (bindings/swift/Package.swift:
# iOS 17; macOS 14 reads the same specification, the desktop needs 15).
TARGET = "iOS17"
BATCH = 10


def float16_weights(program):
    """Replace each float32 weight of `program` by a load-time cast of its
    float16 rounding; the arithmetic stays float32."""
    from coremltools.converters.mil.mil import Builder as mb
    from coremltools.converters.mil.mil.scope import ScopeInfo

    cast = 0
    for function in program.functions.values():
        for op in list(function.operations):
            value = op.outputs[0].val if op.op_type == "const" else None
            # Scalars and short vectors (shapes, axes, epsilons) stay float32.
            if not isinstance(value, np.ndarray) or value.dtype != np.float32 or value.size < 10:
                continue
            if not np.isfinite(value.astype(np.float16)).all():
                raise ValueError(f"{op.name} does not fit float16")
            # The converter requires every new op to carry its source's scopes.
            scopes = [ScopeInfo(source=source, data=data) for source, data in op.scopes.items()]
            with function, mb.scope(*scopes):
                weight = mb.constexpr_cast(source_val=value.astype(np.float16),
                                           output_dtype="fp32", before_op=op)
            function.replace_uses_of_var_after_op(anchor_op=op, old_var=op.outputs[0],
                                                  new_var=weight)
            function.remove_ops([op])
            cast += 1
    if cast == 0:
        raise RuntimeError("No weight was stored as float16")
    return cast


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    import coremltools as ct
    import torch
    from coremltools.converters.mil.backend.mil import load as mil_load
    import alphabet_model

    # coremltools moves every constant of ten or more values into a blob file;
    # keep them all in the specification instead (see the module docstring).
    mil_load.should_use_weight_file = lambda *_args, **_kwargs: False

    source = alphabet_model.load(args.model)
    traced = torch.jit.trace(single_channel(source), torch.zeros(BATCH, 1, 96, 96))
    target = getattr(ct.target, TARGET)
    program = ct.convert(
        traced,
        inputs=[ct.TensorType(name="cells", shape=(BATCH, 1, 96, 96), dtype=np.float32)],
        outputs=[ct.TensorType(name="logits", dtype=np.float32)],
        convert_to="milinternal",
        minimum_deployment_target=target,
        compute_precision=ct.precision.FLOAT32,
    )
    weights = float16_weights(program)
    model = ct.convert(program, convert_to="mlprogram", minimum_deployment_target=target,
                       compute_precision=ct.precision.FLOAT32)
    with tempfile.TemporaryDirectory() as work:
        package = Path(work) / "alphabet.mlpackage"
        model.save(str(package))
        files = [p for p in package.rglob("*") if p.is_file() and p.name != "Manifest.json"]
        if [p.name for p in files] != ["model.mlmodel"]:
            raise RuntimeError(f"Weights left the specification: {[p.name for p in files]}")
        args.output.write_bytes(files[0].read_bytes())

    # The export reads like its source on a probe batch; the real judgement is
    # `matrix_video.py evaluate` on the held-out card.
    # Seven cells: a padded batch, as Core sends most of a card's last one.
    probe = (np.random.default_rng(0).random((7, 1, 96, 96)) * 2 - 1).astype(np.float32)
    with torch.no_grad():
        reference = source(torch.from_numpy(probe)).numpy()
    from matrix_video import CoreMlSession
    exported = CoreMlSession(args.output).run(None, {"cells": probe})[0]
    if not (exported.argmax(1) == reference.argmax(1)).all():
        raise RuntimeError("The Core ML export disagrees with its source on the probe batch")

    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    print(json.dumps({"target": TARGET, "weights": "float16", "compute": "float32",
                      "castWeights": weights,
                      "probeMaxLogitError": float(np.abs(exported - reference).max()),
                      "sourceSha256": digest(args.model),
                      "outputSha256": digest(args.output), "outputBytes": args.output.stat().st_size}))


if __name__ == "__main__":
    main()
