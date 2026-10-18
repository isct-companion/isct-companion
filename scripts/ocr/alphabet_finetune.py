"""One supervised fine-tune of the specified ONNX model on synthetic print."""

import argparse, hashlib, json, random, time
from pathlib import Path
import numpy as np
import onnx
import onnxruntime as ort

ort.disable_telemetry_events()
from onnx2torch import convert
import torch
from torch import nn


def evaluate(model, images, labels, device, batch=256):
    model.eval()
    correct = qualified = wrong_qualified = 0
    loss = 0.0
    with torch.inference_mode():
        for start in range(0, len(labels), batch):
            x = (
                images[start : start + batch].to(device).float().unsqueeze(1) / 127.5
                - 1
            )
            y = labels[start : start + batch].to(device)
            out = model(x)
            prob = out.softmax(1)
            score, pred = prob.max(1)
            correct += (pred == y).sum().item()
            q = (score >= 0.8) & (pred != 26)
            qualified += q.sum().item()
            wrong_qualified += (q & (pred != y)).sum().item()
            loss += nn.functional.cross_entropy(out, y, reduction="sum").item()
    return dict(
        correct=correct,
        total=len(labels),
        accuracy=correct / len(labels),
        qualified=qualified,
        wrongQualified=wrong_qualified,
        loss=loss / len(labels),
    )


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--data", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--resume")
    p.add_argument("--device", choices=["cuda", "mps", "cpu"], default="cuda")
    p.add_argument("--batch-size", type=int, default=256)
    p.add_argument("--train-convs", type=int, default=0)
    p.add_argument("--epochs", type=int, default=8)
    a = p.parse_args()
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
    graph = onnx.load(a.model)
    model = convert(graph).eval()
    data = np.load(a.data)
    images = torch.from_numpy(data["train_images"].copy())
    labels = torch.from_numpy(data["train_labels"].copy())
    val_images = torch.from_numpy(data["validation_images"].copy())
    val_labels = torch.from_numpy(data["validation_labels"].copy())
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
    model = model.to(device)
    if a.train_convs:
        convolutions = [module for module in model.modules() if isinstance(module, nn.Conv2d)]
        for module in convolutions[:-a.train_convs]:
            for parameter in module.parameters(): parameter.requires_grad_(False)
    optimizer = torch.optim.AdamW((p for p in model.parameters() if p.requires_grad), lr=2e-4, weight_decay=1e-4)
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=a.epochs)
    best = (-1.0, float("-inf"))
    history = []
    start = time.perf_counter()
    for epoch in range(a.epochs):
        model.train()
        order = torch.randperm(len(labels))
        running = 0.0
        count = 0
        for indices in order.split(a.batch_size):
            x = images[indices].to(device).float().unsqueeze(1) / 127.5 - 1
            y = labels[indices].to(device)
            optimizer.zero_grad(set_to_none=True)
            with torch.autocast(a.device, dtype=torch.bfloat16, enabled=a.device=="cuda"):
                loss = nn.functional.cross_entropy(model(x), y, label_smoothing=0.01)
            loss.backward()
            optimizer.step()
            running += loss.item() * len(y)
            count += len(y)
        metrics = evaluate(model, val_images, val_labels, device, batch=a.batch_size)
        metrics.update(
            epoch=epoch + 1,
            trainLoss=running / count,
            elapsedSeconds=time.perf_counter() - start,
        )
        history.append(metrics)
        print(json.dumps(metrics), flush=True)
        key = (metrics["accuracy"], -metrics["loss"])
        if key > best:
            best = key
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
        history=history,
        exportMaxAbsError=delta,
        seconds=time.perf_counter() - start,
    )
    (output / "training.json").write_text(json.dumps(report, indent=2))
    print("EXPORTED", str(exported), flush=True)


if __name__ == "__main__":
    main()
