"""Temperature fitting on held-out synthetic fonts, with production-view fusion.

Calibration fonts never select checkpoints; test fonts never fit temperature.
Exports are candidates only: full Rust acquisition/voting replay is still required.
"""

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
from scipy.optimize import minimize_scalar
from scipy.special import logsumexp, softmax

ort.disable_telemetry_events()


def require_prepared_data(data):
    required = {"validation_flags", "validation_corrected_images"}
    if not required.issubset(data.files):
        raise ValueError("Regenerate data with alphabet_synthetic.py --prepare-binary; raw-only NPZs are unsupported")
    if (data["train_labels"] < 0).any():
        raise ValueError("Ambiguous labels must be excluded from supervised training")


def fuse(raw, corrected, flags, temperature=1.0):
    probabilities = softmax(raw / temperature, axis=1)
    corrected_probabilities = softmax(corrected / temperature, axis=1)
    prediction = probabilities.argmax(1)
    confidence = probabilities.max(1)
    other = corrected_probabilities.argmax(1)
    other_score = corrected_probabilities.max(1)
    conflict = ((flags == 2) & (confidence >= .8) &
                (other_score >= .8) & (prediction != other))
    replace = (flags == 2) & ~conflict & (other_score > confidence)
    prediction[replace] = other[replace]
    confidence[replace] = other_score[replace]
    confidence[conflict] = .79
    prediction[flags == 0] = 26
    confidence[flags == 0] = 0
    return prediction, confidence, conflict


def metrics(raw, corrected, labels, flags, temperature=1.0):
    prediction, confidence, conflict = fuse(raw, corrected, flags, temperature)
    known = labels >= 0
    correct = prediction == labels
    qualified = (prediction != 26) & (confidence >= .8)
    high = (prediction != 26) & (confidence >= .9)
    # ECE describes fused non-rejected predictions; it is not a real-card
    # correctness probability and must be read alongside wrong-accepted counts.
    active = (flags > 0) & known
    ece = 0.
    bins = np.minimum((confidence * 10).astype(int), 9)
    for index in range(10):
        group = active & (bins == index)
        if group.any():
            ece += group.sum() / max(1, active.sum()) * abs(
                confidence[group].mean() - correct[group].mean())
    result = dict(sourceTotal=len(labels), total=int(known.sum()), correct=int(correct.sum()),
                  accuracy=float(correct[known].mean()), rejected=int(((flags == 0) & known).sum()),
                  ambiguous=int((~known).sum()),
                  ambiguousQualified=int((qualified & ~known).sum()),
                  qualified=int((qualified & known).sum()),
                  correctQualified=int((qualified & correct).sum()),
                  wrongQualified=int((qualified & ~correct & known).sum()),
                  high=int((high & known).sum()), wrongHigh=int((high & ~correct & known).sum()),
                  conflicts=int(conflict.sum()), ece=float(ece))
    result["loss"] = objective(temperature, raw, corrected, labels, flags)
    return result


def objective(temperature, raw, corrected, labels, flags):
    keep = (flags > 0) & (labels >= 0)
    dual = flags == 2
    if not keep.any():
        raise ValueError("No accepted calibration observations")
    def nll(logits):
        scaled = logits / temperature
        return logsumexp(scaled, axis=1) - scaled[np.arange(len(labels)), labels]
    # Two views of the same observation have total weight one.
    return float(((nll(raw) + dual * nll(corrected)) / (1 + dual))[keep].mean())


def infer(session, images):
    name = session.get_inputs()[0].name
    logits = np.concatenate([session.run(None, {name: (
        images[start:start + 128, None].astype(np.float32) / 127.5 - 1)})[0]
        for start in range(0, len(images), 128)])
    if logits.shape != (len(images), 27) or not np.isfinite(logits).all():
        raise RuntimeError("Invalid model logits")
    return logits


def infer_views(session, data, split):
    flags = data[split + "_flags"]
    keep, dual = flags > 0, flags == 2
    raw = np.zeros((len(flags), 27), dtype=np.float32)
    if keep.any():
        raw[keep] = infer(session, data[split + "_images"][keep])
    corrected = raw.copy()
    if dual.any():
        corrected[dual] = infer(session, data[split + "_corrected_images"][dual])
    return raw, corrected, data[split + "_labels"], flags


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    ort.disable_telemetry_events()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(str(args.model), options, providers=["CPUExecutionProvider"])
    data = np.load(args.data)
    require_prepared_data(data)
    outputs = {}
    for split in ("calibration", "test"):
        outputs[split] = infer_views(session, data, split)
    fit = minimize_scalar(lambda t: objective(t, *outputs["calibration"]),
                          bounds=(.1, 10.), method="bounded")
    if not fit.success or not np.isfinite(fit.fun):
        raise RuntimeError("Temperature optimization failed")
    temperature = float(fit.x)
    report = dict(temperature=temperature,
                  modelSha256=hashlib.sha256(args.model.read_bytes()).hexdigest(),
                  dataSha256=hashlib.sha256(args.data.read_bytes()).hexdigest())
    for split, values in outputs.items():
        report[split] = {"before": metrics(*values),
                         "after": metrics(*values, temperature=temperature)}
    before, after = report["test"]["before"], report["test"]["after"]
    report["passesSyntheticGate"] = (
        after["wrongQualified"] <= before["wrongQualified"]
        and after["wrongHigh"] <= before["wrongHigh"]
        and after["ambiguousQualified"] <= before["ambiguousQualified"]
        and after["correctQualified"] >= before["correctQualified"]
        and after["correct"] >= before["correct"])
    graph = onnx.load(str(args.model))
    output_name = graph.graph.output[0].name
    internal_name = output_name + "_uncalibrated"
    for node in graph.graph.node:
        for i, name in enumerate(node.output):
            if name == output_name:
                node.output[i] = internal_name
        for i, name in enumerate(node.input):
            if name == output_name:
                node.input[i] = internal_name
    graph.graph.initializer.append(onnx.numpy_helper.from_array(
        np.array(temperature, dtype=np.float32), name="calibration_temperature"))
    graph.graph.node.append(onnx.helper.make_node(
        "Div", [internal_name, "calibration_temperature"], [output_name]))
    calibrated = args.output / "alphabet-calibrated.onnx"
    onnx.checker.check_model(graph)
    onnx.save(graph, str(calibrated))
    check = ort.InferenceSession(str(calibrated), options, providers=["CPUExecutionProvider"])
    errors = []
    for count in (1, 7, 10):
        images = data["test_images"][:count]
        error = float(np.max(np.abs(infer(check, images) - infer(session, images) / temperature)))
        if error > 1e-4:
            raise RuntimeError(f"Calibrated export mismatch: {error}")
        errors.append(error)
    report["exportMaxAbsError"] = max(errors)
    report["calibratedSha256"] = hashlib.sha256(calibrated.read_bytes()).hexdigest()
    (args.output / "calibration.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
