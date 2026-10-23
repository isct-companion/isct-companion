#!/usr/bin/env python3
"""Matrix geometry replay report: stage overlays, capture-to-draw timeline, metrics.

Inputs are trace JSONL files with one line per frame, written by
`cargo run -p isct-ocr --release --features training-tools --example
matrix_track_replay -- FRAMES` or by `matrix_cv_reference.py`:

    {"frame": "0001.jpg", "frameId": 0, "t": 0.0, "pipeline": "...",
     "stages": {"coarse": QUAD, ...}, "final": QUAD | null,
     "ms": {"total": 51.2, ...}}

QUAD is the 7x10 body (A1 top-left, clockwise) in normalized image coordinates.
A live `matrix-scan/latest.json` from a debug app (`--live`) adds the measured
in-app timeline. Three metrics are always reported separately:

* processing FPS: frames the geometry loop finishes per second;
* localisation rate: located frames / processed frames;
* box-draw latency: box drawn on screen minus capture of the frame it came from.

Geometry failures are measured against labels from an independent source
(`--labels NAME=PATH`, e.g. SIFT alignment to a still photo or hand clicks):
a located body whose worst corner is off by more than a quarter cell is wrong.
Frames, labels and overlays are private card images: keep `--out` local.
"""
import argparse
import json
import math
from pathlib import Path

import cv2
import numpy as np

COLORS = {  # BGR, one per stage name; unknown stages cycle through EXTRA.
    "coarse": (0, 165, 255), "color": (255, 0, 255), "luma": (255, 255, 0),
    "detect": (0, 165, 255), "fit": (255, 0, 255), "track": (255, 255, 0),
    "smoothed": (0, 255, 0), "final": (0, 0, 255),
}
EXTRA = [(128, 0, 255), (0, 128, 255), (255, 128, 0)]


def load_image(frames, name):
    """A frame image, or the luma plane of a desktop recording's `.planes` file."""
    if not name.endswith(".planes"):
        return cv2.imread(str(frames / name))
    index = {}
    for line in (frames / "frames.jsonl").read_text().splitlines():
        if line.strip():
            v = json.loads(line)
            index[v["file"]] = (v["width"], v["height"])
    w, h = index[name]
    luma = np.fromfile(frames / name, np.uint8)[: w * h].reshape(h, w)
    return cv2.cvtColor(luma, cv2.COLOR_GRAY2BGR)


def load_trace(path):
    rows = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    return rows


def load_labels(path):
    data = json.loads(Path(path).read_text())
    return {name: (v["body"] if isinstance(v, dict) else v) for name, v in data.items()}


def grid_points(body, size, cols=10, rows=7):
    """Project the full 11x8 line grid of a body quad (homography from corners)."""
    w, h = size
    src = np.float32([[0, 0], [cols, 0], [cols, rows], [0, rows]])
    dst = np.float32([[x * w, y * h] for x, y in body])
    H = cv2.getPerspectiveTransform(src, dst)
    u, v = np.meshgrid(np.arange(cols + 1), np.arange(rows + 1))
    pts = np.stack([u, v], -1).reshape(-1, 1, 2).astype(np.float32)
    return cv2.perspectiveTransform(pts, H).reshape(rows + 1, cols + 1, 2)


def corner_error(body, label, size):
    """Worst corner distance in cells (cell width taken from the label)."""
    w, h = size
    a = np.array(body) * [w, h]
    b = np.array(label) * [w, h]
    cell = (np.linalg.norm(b[1] - b[0]) + np.linalg.norm(b[2] - b[3])) / 20
    return float(np.max(np.linalg.norm(a - b, axis=1)) / cell), float(np.max(np.linalg.norm(a - b, axis=1)))


def simulate(rows, camera_fps, display_hz, transport_ms):
    """Latest-frame-only loop over recorded capture times and measured costs.

    Frames arrive at their capture time. A free processor always takes the
    newest arrived frame and drops older ones. A finished box is drawn at the
    next display refresh. Box age is how much older the box's frame is than the
    newest camera frame on screen at that moment.
    """
    times = [r["t"] for r in rows]
    vsync = 1 / display_hz
    now, i, events = times[0], 0, []
    while True:
        # newest arrived frame not older than the last processed one
        j = max(i, int(np.searchsorted(times, now, side="right")) - 1)
        if j >= len(rows):
            break
        start = max(now, times[j])
        cost = (rows[j]["ms"]["total"] + transport_ms) / 1000
        done = start + cost
        drawn = math.ceil(done / vsync) * vsync
        shown = times[max(0, int(np.searchsorted(times, drawn, side="right")) - 1)]
        events.append({"index": j, "start": start, "done": done, "drawn": drawn,
                       "located": rows[j]["final"] is not None,
                       "latency": drawn - times[j], "age": shown - times[j]})
        now, i = done, j + 1
        if i >= len(rows):
            break
    duration = (times[-1] - times[0]) + 1 / camera_fps
    return events, duration


def pct(values, q):
    return float(np.percentile(values, q)) if len(values) else None


def summarize(name, rows, labels, size_of, args):
    located = [r for r in rows if r["final"] is not None]
    totals = [r["ms"]["total"] for r in rows]
    events, duration = simulate(rows, args.camera_fps, args.display_hz, args.transport_ms)
    drawn = [e for e in events if e["located"]]
    summary = {
        "pipeline": rows[0].get("pipeline", name),
        "frames": len(rows),
        "located": len(located),
        "localisationRate": len(located) / len(rows),
        "processingMsP50": pct(totals, 50),
        "processingMsP95": pct(totals, 95),
        "throughputFps": 1000 / float(np.mean(totals)),
        "liveSimulation": {
            "cameraFps": args.camera_fps, "displayHz": args.display_hz,
            "transportMs": args.transport_ms,
            "processedFps": len(events) / duration,
            "locatedBoxesPerSecond": len(drawn) / duration,
            "drawLatencyMsP50": pct([e["latency"] * 1000 for e in drawn], 50),
            "drawLatencyMsP95": pct([e["latency"] * 1000 for e in drawn], 95),
            "boxAgeMsP50": pct([e["age"] * 1000 for e in drawn], 50),
            "boxAgeMsP95": pct([e["age"] * 1000 for e in drawn], 95),
        },
        "modes": {},
    }
    for r in rows:
        mode = r.get("mode", "detect")
        summary["modes"][mode] = summary["modes"].get(mode, 0) + 1
    errors = {}
    for label_name, label in labels.items():
        rows_with = [r for r in rows if r["frame"] in label]
        errs = []
        for r in rows_with:
            if r["final"] is None:
                continue
            e, px = corner_error(r["final"], label[r["frame"]], size_of(r["frame"]))
            errs.append(e)
            r.setdefault("errors", {})[label_name] = e
        n = len(rows_with)
        errors[label_name] = {
            "labelledFrames": n,
            "missed": n - len(errs),
            "wrongQuarterCell": sum(e > 0.25 for e in errs),
            "wrongHalfCell": sum(e > 0.5 for e in errs),
            "geometryFailureRate": ((n - len(errs)) + sum(e > 0.25 for e in errs)) / n if n else None,
            "wrongAmongLocated": sum(e > 0.25 for e in errs) / len(errs) if errs else None,
            "cornerErrorCellsP50": pct(errs, 50),
            "cornerErrorCellsP95": pct(errs, 95),
        }
    summary["labels"] = errors
    return summary, events


def draw_overlay(image, row, labels, path):
    h, w = image.shape[:2]
    out = image.copy()
    for label_name, label in labels.items():
        if row["frame"] in label:
            q = (np.array(label[row["frame"]]) * [w, h]).astype(np.int32)
            cv2.polylines(out, [q], True, (255, 255, 255), 5)
    extra = iter(EXTRA)
    stages = dict(row.get("stages", {}))
    for name, quad in stages.items():
        if quad is None:
            continue
        color = COLORS.get(name) or next(extra, (200, 200, 200))
        q = (np.array(quad) * [w, h]).astype(np.int32)
        cv2.polylines(out, [q], True, color, 2)
    if row["final"] is not None:
        g = grid_points(row["final"], (w, h))
        for line in list(g) + list(g.transpose(1, 0, 2)):
            cv2.polylines(out, [line.astype(np.int32)], False, COLORS["final"], 1)
    y = 30
    legend = [("label", (255, 255, 255))] + [(n, COLORS.get(n, (200, 200, 200))) for n in stages] + [("final grid", COLORS["final"])]
    for text, color in legend:
        cv2.putText(out, text, (20, y), cv2.FONT_HERSHEY_SIMPLEX, 0.9, (0, 0, 0), 4)
        cv2.putText(out, text, (20, y), cv2.FONT_HERSHEY_SIMPLEX, 0.9, color, 2)
        y += 32
    errs = row.get("errors", {})
    caption = f'{row["frame"]} t={row["t"]:.3f}s mode={row.get("mode", "-")} ms={row["ms"]["total"]:.1f} ' + " ".join(f"{k}={v:.2f}cell" for k, v in errs.items())
    cv2.putText(out, caption, (20, h - 20), cv2.FONT_HERSHEY_SIMPLEX, 0.9, (0, 0, 0), 4)
    cv2.putText(out, caption, (20, h - 20), cv2.FONT_HERSHEY_SIMPLEX, 0.9, (255, 255, 255), 2)
    scale = min(1, 1400 / w)
    cv2.imwrite(str(path), cv2.resize(out, None, fx=scale, fy=scale, interpolation=cv2.INTER_AREA))


def live_report(path):
    """Measured in-app timeline from a debug build's matrix-scan/latest.json."""
    data = json.loads(Path(path).read_text())
    timeline = data.get("capture", {}).get("timeline") or []
    traces = {t["frameId"]: t for t in data.get("core", {}).get("frames") or []}
    if not timeline:
        return {"note": "no per-frame timeline in this report (recorded before frame ids)"}, []
    by_id = {e["frameId"]: e for e in timeline}
    elapsed = (timeline[-1]["respondedMs"] - timeline[0]["capturedMs"]) / 1000
    located = [e for e in timeline if traces.get(e["frameId"], {}).get("located")]
    draws = []
    for e in timeline:
        source = by_id.get(e.get("boundsFrameId"))
        if e.get("paintedMs") is None or source is None:
            continue
        draws.append({
            "frameId": e["frameId"], "boundsFrameId": source["frameId"],
            "latency": e["paintedMs"] - source["capturedMs"],
            "age": (e["paintedMediaMs"] - source["mediaMs"]) if e.get("paintedMediaMs") is not None else None,
            "ipc": e["respondedMs"] - e["sentMs"],
            "native": sum(traces.get(e["frameId"], {}).get(k, 0) for k in ("decodeMs", "geometryMs", "cropMs")),
        })
    return {
        "analysedFrames": len(timeline),
        "processingFps": len(timeline) / elapsed if elapsed > 0 else None,
        "localisationRate": len(located) / len(timeline),
        "drawLatencyMsP50": pct([d["latency"] for d in draws], 50),
        "drawLatencyMsP95": pct([d["latency"] for d in draws], 95),
        "boxAgeMsP50": pct([d["age"] for d in draws if d["age"] is not None], 50),
        "boxAgeMsP95": pct([d["age"] for d in draws if d["age"] is not None], 95),
        "roundTripMsP50": pct([d["ipc"] for d in draws], 50),
        "nativeMsP50": pct([d["native"] for d in draws], 50),
    }, draws


def plot_timeline(traces, out):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    fig, axes = plt.subplots(2, 1, figsize=(11, 7), sharex=True)
    for name, (rows, events) in traces.items():
        t = [r["t"] for r in rows]
        axes[0].plot(t, [r["ms"]["total"] for r in rows], ".", ms=3, label=name)
        axes[1].plot([e["drawn"] for e in events if e["located"]],
                     [e["latency"] * 1000 for e in events if e["located"]], "-", lw=1, label=f"{name} draw latency")
        axes[1].plot([e["drawn"] for e in events if e["located"]],
                     [e["age"] * 1000 for e in events if e["located"]], ":", lw=1, label=f"{name} box age")
    axes[0].set_ylabel("processing ms / frame")
    axes[1].set_ylabel("ms")
    axes[1].set_xlabel("capture time (s)")
    for ax in axes:
        ax.grid(alpha=.3)
        ax.legend(fontsize=8)
    fig.tight_layout()
    fig.savefig(out, dpi=120)
    plt.close(fig)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--frames", type=Path, required=True)
    p.add_argument("--trace", action="append", default=[], help="NAME=trace.jsonl")
    p.add_argument("--labels", action="append", default=[], help="NAME=labels.json")
    p.add_argument("--live", type=Path, help="matrix-scan/latest.json from a debug app")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--overlays", type=int, default=12, help="worst/missed frames drawn per trace")
    p.add_argument("--overlay-frames", nargs="*", default=[])
    p.add_argument("--camera-fps", type=float, default=30)
    p.add_argument("--display-hz", type=float, default=60)
    p.add_argument("--transport-ms", type=float, default=0, help="added per frame to simulate capture/IPC")
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    labels = {k: load_labels(v) for k, v in (s.split("=", 1) for s in args.labels)}
    sizes = {}

    def size_of(name):
        if name not in sizes:
            image = load_image(args.frames, name)
            sizes[name] = (image.shape[1], image.shape[0])
        return sizes[name]

    report, plotted = {}, {}
    for name, path in (s.split("=", 1) for s in args.trace):
        rows = load_trace(path)
        summary, events = summarize(name, rows, labels, size_of, args)
        report[name] = summary
        plotted[name] = (rows, events)
        chosen = set(args.overlay_frames)
        misses = [r["frame"] for r in rows if r["final"] is None and any(r["frame"] in l for l in labels.values())]
        worst = sorted((r for r in rows if r.get("errors")), key=lambda r: -max(r["errors"].values()))
        chosen.update(misses[: args.overlays // 2])
        chosen.update(r["frame"] for r in worst[: max(0, args.overlays - len(chosen))])
        directory = args.out / f"overlays-{name}"
        directory.mkdir(exist_ok=True)
        for r in rows:
            if r["frame"] in chosen:
                image = load_image(args.frames, r["frame"])
                draw_overlay(image, r, labels, directory / f'{Path(r["frame"]).stem}.png')
        with open(args.out / f"timeline-{name}.jsonl", "w") as f:
            for e in events:
                f.write(json.dumps({**e, "frame": rows[e["index"]]["frame"]}) + "\n")
    if args.live:
        report["live"], _ = live_report(args.live)
    if plotted:
        plot_timeline(plotted, args.out / "timeline.png")
    (args.out / "summary.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
