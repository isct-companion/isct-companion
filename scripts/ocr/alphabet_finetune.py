"""One supervised fine-tune of the specified ONNX model.

Training data is a prepared NPZ (`matrix_video.py build` for card recordings,
`alphabet_synthetic.py --prepare-binary` for synthetic print); `--replay-data`
mixes a second one in, and `--replay-fraction` fixes its share of each epoch,
so a few real cards cannot crowd out the synthetic alphabet or the reverse.
`--augment P` applies camera-like perturbations on the GPU to that share of
training batches. Checkpoints are selected on the validation split only.
"""

import argparse, hashlib, json, random, time
from pathlib import Path
import numpy as np
import onnx
import onnxruntime as ort

ort.disable_telemetry_events()
import torch
from torch import nn
import alphabet_model
from alphabet_calibration import metrics as fused_metrics, require_prepared_data, preserves_qualified_coverage


def evaluate(model, images, labels, device, batch=256, corrected=None, flags=None):
    if flags is None or corrected is None:
        raise ValueError("Evaluation requires production Rust preparation records")
    model.eval()
    def predict(values):
        with torch.inference_mode():
            return np.concatenate([
                model(values[start:start + batch].to(device).float().unsqueeze(1)
                      / 127.5 - 1).cpu().numpy()
                for start in range(0, len(values), batch)])
    return fused_metrics(predict(images), predict(corrected), labels.numpy(), flags)


def augment(x, probability, generator):
    """Camera-like perturbations of prepared inputs in [-1, 1], N×1×96×96.

    The production preprocessing already centres and scales the ink, so the
    geometric jitter stays small; blur, exposure and noise cover hand motion,
    focus and lighting that recordings of a few cards cannot.
    """
    n = len(x)
    chosen = torch.rand(n, device=x.device, generator=generator) < probability
    if not chosen.any():
        return x
    u = lambda lo, hi: torch.empty(n, device=x.device).uniform_(lo, hi, generator=generator)
    angle = u(-4, 4).deg2rad()
    scale = u(0.9, 1.1)
    theta = torch.zeros(n, 2, 3, device=x.device)
    theta[:, 0, 0] = angle.cos() / scale
    theta[:, 0, 1] = -angle.sin() / scale
    theta[:, 1, 0] = angle.sin() / scale
    theta[:, 1, 1] = angle.cos() / scale
    theta[:, :, 2] = torch.stack((u(-3, 3), u(-3, 3)), 1) / 48
    grid = nn.functional.affine_grid(theta, x.shape, align_corners=False)
    y = nn.functional.grid_sample(x, grid, padding_mode="border", align_corners=False)
    # Blur: an isotropic Gaussian or a short motion streak, one per sample.
    sigma = u(0.01, 0.9)
    radius = torch.arange(-3, 4, device=x.device, dtype=x.dtype)
    gauss = torch.exp(-radius[None] ** 2 / (2 * sigma[:, None] ** 2))
    gauss = gauss / gauss.sum(1, keepdim=True)
    kernel = gauss[:, :, None] * gauss[:, None, :]
    motion = torch.rand(n, device=x.device, generator=generator) < 0.3
    if motion.any():
        length = torch.randint(2, 5, (n,), device=x.device, generator=generator)
        streak = (radius.abs()[None] <= length[:, None] / 2).to(x.dtype)
        streak = streak / streak.sum(1, keepdim=True)
        horizontal = torch.rand(n, device=x.device, generator=generator) < 0.5
        line = torch.zeros(n, 7, 7, device=x.device, dtype=x.dtype)
        line[:, 3, :] = streak
        line = torch.where(horizontal[:, None, None], line, line.transpose(1, 2))
        kernel = torch.where(motion[:, None, None], line, kernel)
    y = nn.functional.conv2d(nn.functional.pad(y, (3, 3, 3, 3), mode="replicate")
                             .transpose(0, 1), kernel[:, None], groups=n).transpose(0, 1)
    # Exposure and contrast about the paper level, then sensor noise.
    paper = y.flatten(1).quantile(0.9, dim=1)[:, None, None, None]
    y = (y - paper) * u(0.55, 1.15)[:, None, None, None] + paper + u(-0.2, 0.1)[:, None, None, None]
    y = y + torch.randn(y.shape, device=x.device, generator=generator) * u(0, 0.05)[:, None, None, None]
    y = y.clamp(-1, 1)
    return torch.where(chosen[:, None, None, None], y, x)


def validation_views(data):
    require_prepared_data(data)
    return dict(corrected=torch.from_numpy(data["validation_corrected_images"].copy()),
                flags=data["validation_flags"])


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--data", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--resume")
    p.add_argument("--resume-reference", help="ONNX artifact that the resumed weights must match")
    p.add_argument("--replay-data", help="Original synthetic print dataset retained to prevent forgetting")
    p.add_argument("--device", choices=["cuda", "mps", "cpu"], default="cuda")
    p.add_argument("--batch-size", type=int, default=256)
    p.add_argument("--train-convs", type=int, default=0)
    p.add_argument("--epochs", type=int, default=8)
    p.add_argument("--learning-rate", type=float, default=2e-4)
    p.add_argument("--replay-fraction", type=float,
                   help="Share of each epoch drawn from --replay-data (default: by size)")
    p.add_argument("--augment", type=float, default=0.,
                   help="Probability that a training sample is perturbed on the GPU")
    a = p.parse_args()
    if not 0 <= a.augment <= 1 or (a.replay_fraction is not None and not 0 < a.replay_fraction < 1):
        p.error("--augment must lie in [0, 1] and --replay-fraction in (0, 1)")
    data = np.load(a.data)
    require_prepared_data(data)
    torch.manual_seed(20261017)
    np.random.seed(20261017)
    random.seed(20261017)
    torch.set_num_threads(4)
    if a.device == "cuda" and not torch.cuda.is_available():
        raise RuntimeError("GPU allocation required")
    torch.backends.cuda.matmul.allow_tf32 = True
    torch.backends.cudnn.allow_tf32 = True
    device = torch.device(a.device)
    output = Path(a.output)
    output.mkdir(parents=True, exist_ok=True)
    print(
        json.dumps(
            dict(
                gpu=torch.cuda.get_device_name(0) if a.device == "cuda" else a.device,
                torch=torch.__version__,
                sourceSha256=hashlib.sha256(Path(a.model).read_bytes()).hexdigest(),
            )
        ),
        flush=True,
    )
    model = alphabet_model.load(a.model)
    images = torch.from_numpy(data["train_images"].copy())
    labels = torch.from_numpy(data["train_labels"].copy())
    val_images = torch.from_numpy(data["validation_images"].copy())
    val_labels = torch.from_numpy(data["validation_labels"].copy())
    views = validation_views(data)
    replay_validation = None
    replay_views = {}
    weights = torch.ones(len(labels), dtype=torch.float64)
    if a.replay_data:
        replay = np.load(a.replay_data)
        replay_views = validation_views(replay)
        extra = torch.from_numpy(replay["train_labels"].copy())
        if a.replay_fraction is not None:
            weights = torch.cat((torch.full((len(labels),), (1 - a.replay_fraction) / len(labels)),
                                 torch.full((len(extra),), a.replay_fraction / len(extra))))
        else:
            weights = torch.ones(len(labels) + len(extra), dtype=torch.float64)
        images = torch.cat((images, torch.from_numpy(replay["train_images"].copy())))
        labels = torch.cat((labels, extra))
        replay_validation = (
            torch.from_numpy(replay["validation_images"].copy()),
            torch.from_numpy(replay["validation_labels"].copy()),
        )
    sample = images[:8].float().unsqueeze(1) / 127.5 - 1
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 2
    opts.inter_op_num_threads = 1
    s = ort.InferenceSession(
        a.model, sess_options=opts, providers=["CPUExecutionProvider"]
    )
    reference = s.run(None, {s.get_inputs()[0].name: sample.numpy()})[0]
    with torch.no_grad():
        actual = model(sample).numpy()
    difference = float(np.max(np.abs(reference - actual)))
    print("conversionMaxAbsError", difference, flush=True)
    if difference > 1e-3:
        raise RuntimeError("Converted model changed pretrained outputs")
    del s
    if a.resume:
        model.load_state_dict(torch.load(a.resume, weights_only=True, map_location="cpu"))
        if a.resume_reference:
            check = ort.InferenceSession(a.resume_reference, sess_options=opts,
                                        providers=["CPUExecutionProvider"])
            reference = check.run(None, {check.get_inputs()[0].name: sample.numpy()})[0]
            with torch.no_grad():
                delta = float(np.max(np.abs(reference - model(sample).numpy())))
            print("resumeMaxAbsError", delta, flush=True)
            if delta > 1e-3:
                raise RuntimeError("Resumed checkpoint differs from reference artifact")
    model = model.to(device)
    if a.train_convs:
        convolutions = [module for module in model.modules() if isinstance(module, nn.Conv2d)]
        model.requires_grad_(False)
        for module in convolutions[-a.train_convs:]:
            module.requires_grad_(True)
        for module in model.modules():
            if isinstance(module, nn.Linear):
                module.requires_grad_(True)
    optimizer = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=a.learning_rate, weight_decay=1e-4)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=a.epochs)
    history = []
    generator = torch.Generator(device=device).manual_seed(20261024)
    start = time.perf_counter()
    baseline = evaluate(model, val_images, val_labels, device, batch=a.batch_size, **views)
    print(json.dumps({"baseline": baseline}), flush=True)
    baseline_replay = evaluate(model, *replay_validation, device, batch=a.batch_size, **replay_views) if replay_validation else None
    if baseline_replay:
        print(json.dumps({"baselineReplay": baseline_replay}), flush=True)
    # Accuracy first, on the validation cards: fewer confident errors, then
    # more confident correct cells. The initial checkpoint stays selected if
    # no epoch beats it.
    def rank(m):
        return (-m["wrongQualified"], -m["wrongHigh"], m["correctQualified"], m["accuracy"], -m["loss"])
    best = rank(baseline)
    selected_epoch = 0
    torch.save(model.state_dict(), output / "best.pt")
    for epoch in range(a.epochs):
        model.train()
        if a.train_convs:
            for module in model.modules():
                if isinstance(module, nn.modules.batchnorm._BatchNorm):
                    module.eval()
        # One epoch draws as many samples as there are, weighted by source.
        order = torch.multinomial(weights, len(labels), replacement=True)
        running = 0.0
        count = 0
        for indices in order.split(a.batch_size):
            x = images[indices].to(device).float().unsqueeze(1) / 127.5 - 1
            if a.augment:
                x = augment(x, a.augment, generator)
            y = labels[indices].to(device)
            optimizer.zero_grad(set_to_none=True)
            with torch.autocast(a.device, dtype=torch.bfloat16, enabled=a.device=="cuda"):
                loss = nn.functional.cross_entropy(model(x), y, label_smoothing=0.01)
            if not torch.isfinite(loss):
                raise RuntimeError("Non-finite training loss")
            loss.backward()
            optimizer.step()
            running += loss.item() * len(y)
            count += len(y)
        metrics = evaluate(model, val_images, val_labels, device, batch=a.batch_size, **views)
        if replay_validation:
            metrics["replay"] = evaluate(model, *replay_validation, device, batch=a.batch_size, **replay_views)
        metrics.update(
            epoch=epoch + 1,
            trainLoss=running / count,
            elapsedSeconds=time.perf_counter() - start,
        )
        history.append(metrics)
        print(json.dumps(metrics), flush=True)
        key = rank(metrics)
        eligible = preserves_qualified_coverage(baseline, metrics)
        if baseline_replay:
            # Synthetic print must not gain confident errors or lose coverage.
            # Confidence on its quarantined (unreadable) glyphs is checked
            # after temperature calibration (alphabet_calibration.py), which
            # owns the model's confidence scale.
            replay = metrics["replay"]
            eligible = eligible and (replay["wrongQualified"] <= baseline_replay["wrongQualified"]
                                     and replay["wrongHigh"] <= baseline_replay["wrongHigh"]
                                     and replay["correctQualified"] >= baseline_replay["correctQualified"])
        if eligible and key > best:
            best = key
            selected_epoch = epoch + 1
            torch.save(model.state_dict(), output / "best.pt")
        scheduler.step()
    model.load_state_dict(torch.load(output / "best.pt", weights_only=True, map_location="cpu"))
    model = model.cpu().eval()
    exported = output / "alphabet-printed.onnx"
    torch.onnx.export(
        model,
        (torch.zeros(1, 1, 96, 96),),
        str(exported),
        input_names=["input"],
        output_names=["logits"],
        dynamic_axes={"input": {0: "batch"}, "logits": {0: "batch"}},
        opset_version=17,
        dynamo=False,
    )
    onnx.checker.check_model(str(exported))
    opts = ort.SessionOptions()
    opts.intra_op_num_threads = 2
    check = ort.InferenceSession(
        str(exported), sess_options=opts, providers=["CPUExecutionProvider"]
    )
    reference = check.run(None, {"input": sample.numpy()})[0]
    with torch.no_grad():
        actual = model(sample).numpy()
    delta = float(np.max(np.abs(reference - actual)))
    if delta > 1e-3:
        raise RuntimeError(f"Export parity failure {delta}")
    report = dict(
        sourceSha256=hashlib.sha256(Path(a.model).read_bytes()).hexdigest(),
        modelSha256=hashlib.sha256(exported.read_bytes()).hexdigest(),
        trainingSamples=len(labels),
        validationSamples=len(val_labels),
        baseline=baseline,
        baselineReplay=baseline_replay,
        selectedEpoch=selected_epoch,
        selectionGates=["validation: preserves_qualified_coverage",
                        "replay: wrongQualified, wrongHigh must not increase; correctQualified must not decrease",
                        "replay ambiguousQualified: checked after calibration"],
        selectionOrder=["-wrongQualified", "-wrongHigh", "correctQualified", "accuracy", "-loss"],
        dataSha256=hashlib.sha256(Path(a.data).read_bytes()).hexdigest(),
        replayDataSha256=hashlib.sha256(Path(a.replay_data).read_bytes()).hexdigest() if a.replay_data else None,
        resumeSha256=hashlib.sha256(Path(a.resume).read_bytes()).hexdigest() if a.resume else None,
        resumeReferenceSha256=hashlib.sha256(Path(a.resume_reference).read_bytes()).hexdigest() if a.resume_reference else None,
        parameters=vars(a),
        history=history,
        exportMaxAbsError=delta,
        seconds=time.perf_counter() - start,
    )
    (output / "training.json").write_text(json.dumps(report, indent=2))
    print("EXPORTED", str(exported), flush=True)


if __name__ == "__main__":
    main()
