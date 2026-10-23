#!/usr/bin/env python3
"""One-command local anonymization -> direct TSUBAME iqrsh replay -> 6s-window plot.

Requires ffmpeg/ffprobe, cargo, SSH alias tsubame, numpy and matplotlib.
No raw video, card geometry, original ordering or private source paths are uploaded.
--stage permits resuming an expensive export; provenance prevents stale code/data reuse.
"""

import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import secrets
import shlex
import shutil
import struct
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[2]
PIXELS = 96 * 96


def call(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def exact(stream, count):
    chunks = bytearray()
    while len(chunks) < count:
        part = stream.read(count - len(chunks))
        if not part:
            raise RuntimeError("Native exporter ended before its grayscale payload")
        chunks.extend(part)
    return chunks


def jpegs(stream):
    data = bytearray()
    while block := stream.read(65536):
        data.extend(block)
        while (end := data.find(b"\xff\xd9")) >= 0:
            jpeg = bytes(data[: end + 2])
            del data[: end + 2]
            if not jpeg.startswith(b"\xff\xd8"):
                raise RuntimeError("Invalid ffmpeg JPEG stream")
            yield jpeg
    if data:
        raise RuntimeError("Truncated ffmpeg JPEG stream")


def source_hashes():
    paths = list((ROOT / "crates/isct-ocr/src").rglob("*.rs"))
    paths += [
        ROOT / "crates/isct-core/src/matrix_scan.rs",
        ROOT / "crates/isct-core/src/matrix_scan/capture.rs",
        ROOT / "crates/isct-ocr/examples/matrix_stream_replay.rs",
        ROOT / "crates/isct-ocr/models/alphabet/alphabet-printed.onnx",
    ]
    return {
        str(p.relative_to(ROOT)): digest(p) for p in paths if "tests" not in p.parts
    }


def export(args, out):
    import numpy as np

    expected = "".join(Path(args.expected).read_text().split())
    if len(expected) != 70 or any(
        c not in "ABCDEFGHIJKLMNOPQRSTUVWXYZ" for c in expected
    ):
        raise ValueError("Expected file must contain exactly 70 uppercase letters")
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target))
    call(
        [
            "cargo",
            "build",
            "-p",
            "isct-ocr",
            "--example",
            "matrix_stream_replay",
            "--locked",
        ],
        cwd=ROOT,
        env=env,
    )
    probe = json.loads(
        subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_frames",
                "-show_entries",
                "frame=best_effort_timestamp_time:format=duration",
                "-of",
                "json",
                args.video,
            ]
        )
    )
    stamps = [float(f["best_effort_timestamp_time"]) for f in probe["frames"]]
    origin = stamps[0]
    stamps = [t - origin for t in stamps]
    duration = float(probe["format"]["duration"])
    video_hash = digest(args.video)
    private_path = out / "private-local.json"
    previous = json.loads(private_path.read_text()) if private_path.exists() else {}
    if (
        previous.get("videoSha256") == video_hash
        and previous.get("expected") == expected
    ):
        order = previous["permutation"]
    else:
        order = list(range(70))
        secrets.SystemRandom().shuffle(order)
    (out / "private-local.json").write_text(
        json.dumps(
            {
                "video": str(Path(args.video).resolve()),
                "videoSha256": video_hash,
                "permutation": order,
                "expected": expected,
                "sources": source_hashes(),
            },
            indent=2,
        )
    )
    bundle = out / "bundle"
    bundle.mkdir(exist_ok=True)
    decoder = subprocess.Popen(
        [
            "ffmpeg",
            "-v",
            "error",
            "-threads",
            "2",
            "-i",
            args.video,
            "-map",
            "0:v:0",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "mjpeg",
            "-q:v",
            "2",
            "-f",
            "image2pipe",
            "-",
        ],
        stdout=subprocess.PIPE,
    )
    native = subprocess.Popen(
        [args.target / "debug/examples/matrix_stream_replay", "export"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )
    records = []
    offset = 0
    try:
        with open(bundle / "patches.bin", "wb") as pixels:
            for i, jpeg in enumerate(jpegs(decoder.stdout)):
                if i >= len(stamps):
                    raise RuntimeError("Decoder/frame timestamp count mismatch")
                native.stdin.write(struct.pack("<I", len(jpeg)))
                native.stdin.write(jpeg)
                native.stdin.flush()
                row = json.loads(native.stdout.readline())
                quality = row["quality"]
                if quality:
                    raw = np.frombuffer(
                        exact(native.stdout, 70 * PIXELS), dtype=np.uint8
                    ).reshape(70, PIXELS)
                    pixels.write(raw[order].tobytes())
                    records.append(
                        {
                            "t": stamps[i],
                            "ms": row["ms"],
                            "offset": offset,
                            "quality": [quality[j] for j in order],
                        }
                    )
                    offset += 70 * PIXELS
                else:
                    records.append(
                        {"t": stamps[i], "ms": row["ms"], "offset": None, "quality": []}
                    )
                if i % 100 == 0:
                    print(f"local export {i + 1}/{len(stamps)}", flush=True)
        native.stdin.close()
        if native.wait() or decoder.wait():
            raise RuntimeError("Export subprocess failed")
        if len(records) != len(stamps):
            raise RuntimeError("Incomplete video export")
    finally:
        for child in [native, decoder]:
            if child.poll() is None:
                child.kill()
                child.wait()
    (bundle / "manifest.json").write_text(
        json.dumps(
            {
                "duration": duration,
                "expected": "".join(expected[i] for i in order),
                "frames": records,
            }
        )
    )
    (out / "export.json").write_text(
        json.dumps(
            {
                "frames": len(records),
                "located": sum(r["offset"] is not None for r in records),
                "sourceHashes": source_hashes(),
                "patchesSha256": digest(bundle / "patches.bin"),
                "manifestSha256": digest(bundle / "manifest.json"),
            },
            indent=2,
        )
    )


def package_source(out):
    source = out / "source"
    source.mkdir(exist_ok=True)
    # Compile the exact production domain module in a minimal crate. It has no shell/DB dependencies.
    core = source / "crates/isct-core"
    (core / "src/matrix_scan").mkdir(parents=True, exist_ok=True)
    (core / "Cargo.toml").write_text(
        '[package]\nname="isct-core"\nversion="1.0.0-rc.1"\nedition="2021"\n[features]\nts=[]\n[dependencies]\nserde={version="1",features=["derive"]}\nthiserror="2"\nuuid={version="1",features=["v4"]}\n'
    )
    (core / "src/lib.rs").write_text("pub mod matrix_scan;\n")
    for name in ["matrix_scan.rs", "matrix_scan/capture.rs"]:
        shutil.copy2(ROOT / "crates/isct-core/src" / name, core / "src" / name)
    ocr = source / "crates/isct-ocr"
    ocr.mkdir(exist_ok=True)
    for name in ["Cargo.toml", "build.rs"]:
        shutil.copy2(ROOT / "crates/isct-ocr" / name, ocr / name)
    shutil.copytree(
        ROOT / "crates/isct-ocr/src",
        ocr / "src",
        dirs_exist_ok=True,
        ignore=shutil.ignore_patterns("tests"),
    )
    (ocr / "examples").mkdir(exist_ok=True)
    # Only this example is needed; remove explicit training example entries from the packaging manifest.
    import re

    cargo = (ocr / "Cargo.toml").read_text()
    cargo = re.sub(
        r'\[\[example\]\]\nname = "(?:alphabet_prepare|matrix_dataset)"\nrequired-features = \["training-tools"\]\n',
        "",
        cargo,
    )
    # The cluster's glibc is older than the downloaded static ORT build.
    # Load its compatible manylinux runtime; production Rust/model code is unchanged.
    cargo = cargo.replace('"download-binaries", "tls-rustls"', '"load-dynamic"')
    (ocr / "Cargo.toml").write_text(cargo)
    shutil.copy2(
        ROOT / "crates/isct-ocr/examples/matrix_stream_replay.rs",
        ocr / "examples/matrix_stream_replay.rs",
    )
    shutil.copytree(ROOT / "crates/isct-ocr/models", ocr / "models", dirs_exist_ok=True)
    (source / "Cargo.toml").write_text(
        '[workspace]\nmembers=["crates/isct-core","crates/isct-ocr"]\nresolver="2"\n[profile.dev]\nopt-level=2\n'
    )
    shutil.copy2(ROOT / "Cargo.lock", source / "Cargo.lock")


def package(out):
    evidence = json.loads((out / "export.json").read_text())
    if evidence["sourceHashes"] != source_hashes():
        raise RuntimeError(
            "Production OCR/Core source changed since export; rerun --stage export"
        )
    for name, key in [
        ("patches.bin", "patchesSha256"),
        ("manifest.json", "manifestSha256"),
    ]:
        if digest(out / "bundle" / name) != evidence[key]:
            raise RuntimeError(
                "Export bundle changed or is incomplete; rerun --stage export"
            )
    package_source(out)
    source = out / "source"
    with tarfile.open(out / "upload.tar.gz", "w:gz") as tar:
        tar.add(source, arcname="source")
        tar.add(out / "bundle", arcname="bundle")
        tar.add(out / "export.json", arcname="export.json")


def remote(args, out):
    package(out)
    root = args.remote_dir
    toolchain = args.toolchain_dir or root + "/toolchain"
    if not toolchain.startswith("/") or any(
        c not in "/-_.0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        for c in toolchain
    ):
        raise ValueError("Use an absolute, simple toolchain directory")
    if not root.startswith("/") or any(
        c not in "/-_.0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        for c in root
    ):
        raise ValueError("Use an absolute, simple remote directory")
    call(["ssh", args.host, shlex.join(["mkdir", "-p", root])])
    call(["scp", out / "upload.tar.gz", f"{args.host}:{root}/upload.tar.gz"])
    script = f"""#!/bin/bash
set -euo pipefail
# SGE separates QRSH_COMMAND arguments with non-UTF-8 bytes; Rust build scripts enumerate UTF-8 environment variables.
unset QRSH_COMMAND
cd {shlex.quote(root)}
export CARGO_HOME={shlex.quote(toolchain)}/cargo
export RUSTUP_HOME={shlex.quote(toolchain)}/rustup
export PATH="$CARGO_HOME/bin:$PATH"
export ORT_DYLIB_PATH={shlex.quote(args.ort_library)}
test -f "$ORT_DYLIB_PATH"
if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
  curl -fsSL https://sh.rustup.rs -o rustup-init.sh
  sh rustup-init.sh -y --no-modify-path --profile minimal --default-toolchain 1.98.1
fi
tar -xzf upload.tar.gz
cd source
cargo build -p isct-ocr --example matrix_stream_replay -j 4
hostname
./target/debug/examples/matrix_stream_replay run ../bundle
"""
    (out / "remote.sh").write_text(script)
    call(["scp", out / "remote.sh", f"{args.host}:{root}/remote.sh"])
    with open(out / "iqrsh.log", "w") as log:
        proc = subprocess.Popen(
            [
                "ssh",
                args.host,
                shlex.join(
                    ["iqrsh", "-l", "h_rt=01:00:00", "/bin/bash", root + "/remote.sh"]
                ),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        for line in proc.stdout:
            print(line, end="", flush=True)
            log.write(line)
            log.flush()
        if proc.wait():
            raise RuntimeError("iqrsh replay failed; inspect iqrsh.log")
    call(["scp", f"{args.host}:{root}/bundle/results.json", out / "results.json"])
    call(["scp", f"{args.host}:{root}/source/Cargo.lock", out / "remote-Cargo.lock"])


def plot(out):
    os.environ.setdefault("MPLCONFIGDIR", str(out / ".matplotlib"))
    os.environ.setdefault("XDG_CACHE_HOME", str(out / ".cache"))
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.ticker import MultipleLocator

    report = json.loads((out / "results.json").read_text())
    rows = report["windows"]
    success = sum(r["success"] for r in rows)
    wrong = sum(r["wrongConfirmed"] > 0 for r in rows)
    fig, (ax, coverage) = plt.subplots(
        2,
        1,
        figsize=(13, 6.8),
        sharex=True,
        gridspec_kw={"height_ratios": [3.6, 1]},
        layout="constrained",
    )
    x = [r["start"] for r in rows]
    ax.plot(
        x,
        [r["correct"] / 70 * 100 for r in rows],
        color="#a8b2bf",
        lw=1.5,
        label="Provisional correct / 70",
    )
    ax.plot(
        x,
        [r["confirmedCorrect"] / 70 * 100 for r in rows],
        color="#176b87",
        lw=2,
        marker="o",
        ms=3,
        label="Confirmed correct / 70",
    )
    good = [r for r in rows if r["success"]]
    ax.scatter(
        [r["start"] for r in good],
        [100] * len(good),
        color="#16966c",
        s=32,
        zorder=5,
        label="Whole card completed correctly",
    )
    bad = [r for r in rows if r["wrongConfirmed"]]
    ax.scatter(
        [r["start"] for r in bad],
        [r["confirmedCorrect"] / 70 * 100 for r in bad],
        marker="x",
        color="#cc5b43",
        s=35,
        zorder=6,
        label="Contains a wrong confirmed cell",
    )
    ax.set(
        ylabel="Correct cells (%)",
        ylim=(-2, 104),
        title=f"Independent 6-second scans · one start per second\nWhole-card success: {success}/{len(rows)} ({100 * success / len(rows):.1f}%) · windows with wrong confirmed cells: {wrong}",
    )
    ax.grid(axis="y", alpha=0.18)
    ax.legend(loc="lower left", fontsize=9, ncol=2)
    coverage.bar(x, [r["locatedFrames"] for r in rows], color="#b8ccd4", width=0.8)
    coverage.set(
        xlabel="Window start in source video (s) — point t evaluates [t, t+6)",
        ylabel="Located\nframes",
    )
    coverage.xaxis.set_major_locator(MultipleLocator(5))
    coverage.xaxis.set_minor_locator(MultipleLocator(1))
    for axis in [ax, coverage]:
        axis.spines[["top", "right"]].set_visible(False)
        axis.set_xlim(-0.5, x[-1] + 0.5)
    fig.supxlabel(
        "Warm model · measured local capture + TSUBAME CPU OCR · overlapping windows from one video",
        fontsize=9,
        color="#596570",
    )
    fig.savefig(out / "six-second-accuracy.png", dpi=180)
    fig.savefig(out / "six-second-accuracy.svg")
    keys = [
        "start",
        "end",
        "confirmedCorrect",
        "correct",
        "wrongConfirmed",
        "success",
        "elapsedMs",
        "frames",
        "locatedFrames",
        "timedOut",
    ]
    with open(out / "windows.csv", "w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=keys, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    summary = {
        "windows": len(rows),
        "wholeCardSuccesses": success,
        "wholeCardSuccessRate": success / len(rows),
        "windowsWithWrongConfirmedCells": wrong,
        "meanConfirmedAccuracy": sum(r["confirmedCorrect"] for r in rows)
        / (70 * len(rows)),
        "meanProvisionalAccuracy": sum(r["correct"] for r in rows) / (70 * len(rows)),
        "batchMedianMs": report["batchMedianMs"],
        "batchP95Ms": report["batchP95Ms"],
        "clock": report["clock"],
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))


def main():
    p = argparse.ArgumentParser(
        description=__doc__,
        epilog=(
            "Example: python scripts/ocr/matrix_queue_replay.py --video VIDEO --expected EXPECTED "
            "--out .cache/matrix-replay --host tsubame --remote-dir REMOTE_DIR --ort-library REMOTE_LIB"
        ),
    )
    p.add_argument("--video")
    p.add_argument("--expected")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--target", type=Path, default=ROOT / "target")
    p.add_argument("--host", default="tsubame")
    p.add_argument("--remote-dir", help="Absolute remote experiment directory")
    p.add_argument(
        "--ort-library",
        help="Absolute remote manylinux libonnxruntime.so path supporting API 24",
    )
    p.add_argument(
        "--toolchain-dir",
        help="Reuse an explicitly selected isolated remote Rust installation",
    )
    p.add_argument(
        "--stage", choices=["all", "export", "remote", "plot"], default="all"
    )
    args = p.parse_args()
    args.out = args.out.resolve()
    args.target = args.target.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    if args.stage in ["all", "remote"] and (
        not args.remote_dir or not args.ort_library
    ):
        p.error("--remote-dir and --ort-library are required for TSUBAME replay")
    if args.stage in ["all", "export"]:
        if not args.video or not args.expected:
            p.error("--video and --expected are required for export")
        export(args, args.out)
    if args.stage in ["all", "remote"]:
        remote(args, args.out)
    if args.stage in ["all", "plot"]:
        plot(args.out)


if __name__ == "__main__":
    main()
