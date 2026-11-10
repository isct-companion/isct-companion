"""The alphabet classifier as a native PyTorch module, loaded from its ONNX file.

The bundled ONNX graph is a torchvision MobileNetV2 whose batch norms were
folded into the convolutions at export, with a Flatten-Linear-BatchNorm-ReLU-
Dropout-Linear head over 27 classes (A-Z, blank) and a one-to-three channel
expand in front. Rebuilding that structure and copying the weights in graph
order avoids generic ONNX-to-PyTorch converters, which fail on graphs this
trainer itself re-exported. `load` checks the result against ONNX Runtime.
"""

import numpy as np
import onnx
from onnx import numpy_helper
import onnxruntime as ort
import torch
from torch import nn
from torchvision.models import mobilenet_v2

ort.disable_telemetry_events()


class AlphabetNet(nn.Module):
    def __init__(self):
        super().__init__()
        self.features = mobilenet_v2().features
        for module in list(self.features.modules()):
            for name, child in module.named_children():
                if isinstance(child, nn.BatchNorm2d):
                    setattr(module, name, nn.Identity())
        for module in self.features.modules():
            if isinstance(module, nn.Conv2d) and module.bias is None:
                module.bias = nn.Parameter(torch.zeros(module.out_channels))
        self.head = nn.Sequential(nn.Flatten(), nn.Linear(1280, 256), nn.BatchNorm1d(256),
                                  nn.ReLU(), nn.Dropout(0.2), nn.Linear(256, 27))

    def forward(self, x):
        x = self.features(x.expand(-1, 3, -1, -1))
        return self.head(nn.functional.adaptive_avg_pool2d(x, 1))


def load(path, tolerance=1e-3):
    """AlphabetNet with the weights of the ONNX file at `path`, in eval mode."""
    graph = onnx.load(str(path)).graph
    weights = {i.name: numpy_helper.to_array(i).copy() for i in graph.initializer}
    model = AlphabetNet().eval()
    convolutions = [m for m in model.features.modules() if isinstance(m, nn.Conv2d)]
    nodes = [n for n in graph.node if n.op_type == "Conv"]
    gemms = [n for n in graph.node if n.op_type == "Gemm"]
    norms = [n for n in graph.node if n.op_type == "BatchNormalization"]
    if len(convolutions) != len(nodes) or len(gemms) != 2 or len(norms) != 1:
        raise ValueError("ONNX graph is not the bundled alphabet architecture")

    def assign(parameter, name):
        value = torch.from_numpy(weights[name])
        if parameter.shape != value.shape:
            raise ValueError(f"{name}: {tuple(value.shape)} does not fit {tuple(parameter.shape)}")
        parameter.data = value

    for module, node in zip(convolutions, nodes):
        assign(module.weight, node.input[1])
        assign(module.bias, node.input[2])
    for linear, node in zip((model.head[1], model.head[5]), gemms):
        assign(linear.weight, node.input[1])
        assign(linear.bias, node.input[2])
    for attribute, name in zip(("weight", "bias", "running_mean", "running_var"), norms[0].input[1:]):
        assign(getattr(model.head[2], attribute), name)
    sample = torch.rand(8, 1, 96, 96, generator=torch.Generator().manual_seed(0)) * 2 - 1
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    reference = session.run(None, {session.get_inputs()[0].name: sample.numpy()})[0]
    with torch.no_grad():
        difference = float(np.abs(reference - model(sample).numpy()).max())
    if difference > tolerance:
        raise RuntimeError(f"Rebuilt model differs from ONNX Runtime by {difference}")
    return model
