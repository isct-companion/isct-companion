"""Local video -> production Rust crops -> reviewed, de-identified OCR dataset.

The plan assigns whole time segments before extraction. No OCR output is used
as truth. Reviews confirm geometry and readability; everything else is unknown.
"""

import argparse
from collections import Counter
import hashlib
from io import BytesIO
import json
from pathlib import Path
import random
import subprocess
import time

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

from alphabet_synthetic import prepare


SPLITS = ("train", "validation", "test")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text())


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def review_sheet(root, group, report, truth):
    sheet = Image.new("RGB", (10 * 96, 7 * 114), "white")
    draw = ImageDraw.Draw(sheet)
    for cell in report["cells"]:
        index = cell["cell"]
        x, y = index % 10 * 96, index // 10 * 114
        sheet.paste(Image.open(root / group / "crops" / cell["file"]), (x, y))
        draw.text((x + 2, y + 97), f"{index:02} {truth[index]}", fill="black")
    sheet.save(root / group / f"{report['frame']}-review.jpg", quality=90)


def load_plan(path):
    plan = read_json(path)
    gap_seconds = float(plan.get("gapSeconds", 2))
    if not np.isfinite(gap_seconds) or gap_seconds < 2:
        raise ValueError("Keep at least two seconds between splits from one recording")
    rows = plan["truth"]
    if len(rows) != 7 or any(len(row) != 10 or not row.isascii()
                             or not row.isalpha() or row != row.upper() for row in rows):
        raise ValueError("Truth must be seven manually checked rows of ten A-Z letters")
    if plan.get("truthSource") != "human-checked":
        raise ValueError("OCR predictions cannot supply ground truth")
    segments = plan["segments"]
    if {s["split"] for s in segments} != set(SPLITS):
        raise ValueError("Assign train, validation and untouched test segments first")
    groups = set()
    intervals = []
    for segment in segments:
        group = segment["group"]
        if not group or not all(c.isalnum() or c in "-_" for c in group) or group in groups:
            raise ValueError("Use unique safe segment group names")
        groups.add(group)
        start, end, fps = [float(segment[k]) for k in ("start", "end", "fps")]
        if not np.isfinite([start, end, fps]).all() or not 0 <= start < end or not 0 < fps <= 30:
            raise ValueError("Invalid segment bounds/frequency")
        video = Path(segment["video"]).expanduser().resolve()
        # Check content identity too: renaming a video must not bypass isolation.
        identity = digest(video)
        for other_id, a, b, split in intervals:
            if identity == other_id:
                gap = max(start, a) - min(end, b)
                if gap < 0 or (split != segment["split"] and gap < gap_seconds):
                    raise ValueError("Overlapping segments or insufficient cross-split time gap")
        intervals.append((identity, start, end, segment["split"]))
    return plan


def extract(args):
    plan = load_plan(args.plan)
    args.output.mkdir(parents=True, exist_ok=False)
    write_json(args.output / "plan.json", plan)
    records, review = [], {}
    for segment in plan["segments"]:
        group = segment["group"]
        directory = args.output / group
        directory.mkdir()
        frames, crops = directory / "frames", directory / "crops"
        frames.mkdir()
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin",
                        "-ss", str(segment["start"]), "-i", segment["video"],
                        "-t", str(segment["end"] - segment["start"]), "-an",
                        "-vf", f"fps={segment['fps']}", "-q:v", "2",
                        "-start_number", "0", str(frames / "capture-%05d.jpg")], check=True)
        subprocess.run([str(args.capture_binary.resolve()), str(frames), str(crops)], check=True)
        for report in map(json.loads, (crops / "frames.jsonl").read_text().splitlines()):
            frame = report["frame"]
            key = f"{group}/{frame}"
            report.update(key=key, group=group, split=segment["split"],
                          seconds=segment["start"] + int(frame.split("-")[-1]) / segment["fps"])
            records.append(report)
            if report["outcome"] != "located":
                continue
            review[key] = {"aligned": False, "readableCells": [], "note": "unreviewed"}
            review_sheet(args.output, group, report, "".join(plan["truth"]))
        print(json.dumps({"group": group, "frames": sum(r['group'] == group for r in records),
                          "located": sum(r['group'] == group and r['outcome'] == 'located' for r in records)}), flush=True)
    write_json(args.output / "records.json", records)
    write_json(args.output / "review.json", review)
    write_json(args.output / "provenance.json", {
        "planSha256": digest(args.plan), "captureBinarySha256": digest(args.capture_binary),
        "videos": {str(Path(s["video"]).resolve()): digest(s["video"]) for s in plan["segments"]},
        "scriptSha256": digest(__file__), "localOnly": True})


def assist(args):
    """Local annotation aid, never the app detector or proof of readability.

    OpenCV owns feature matching and robust homography estimation. Each result
    still needs a visual alignment/readability review before supervision.
    https://docs.opencv.org/4.x/d1/de0/tutorial_py_feature_homography.html
    """
    import cv2
    cv2.setNumThreads(2)
    cv2.setRNGSeed(20261020)
    plan = load_plan(args.directory / "plan.json")
    reference = read_json(args.reference)
    image = cv2.imread(reference["frame"], cv2.IMREAD_GRAYSCALE)
    if image is None:
        raise ValueError("Cannot read the local reference frame")
    mask = np.zeros_like(image)
    cv2.fillPoly(mask, [np.array(reference["cardPolygon"], dtype=np.int32)], 255)
    sift = cv2.SIFT_create(nfeatures=5000)
    keypoints, descriptors = sift.detectAndCompute(image, mask)
    if descriptors is None or len(keypoints) < 16:
        raise ValueError("Reference needs at least sixteen distinct card features")
    matcher = cv2.BFMatcher()
    body = np.array(reference["body"], dtype=np.float32).reshape(-1, 1, 2)
    args.output.mkdir(parents=True, exist_ok=False)
    write_json(args.output / "plan.json", plan)
    write_json(args.output / "reference-local.json", reference)
    records, review, tracking = [], {}, []
    for segment in plan["segments"]:
        group = segment["group"]
        directory = args.output / group
        directory.mkdir()
        frames = (args.directory / group / "frames").resolve()
        (directory / "frames").symlink_to(frames, target_is_directory=True)
        manual = {}
        for frame in sorted(frames.glob("capture-*.jpg")):
            gray = cv2.imread(str(frame), cv2.IMREAD_GRAYSCALE)
            points, features = sift.detectAndCompute(gray, None)
            good = [] if features is None else [pair[0] for pair in matcher.knnMatch(
                descriptors, features, k=2) if len(pair) == 2 and pair[0].distance < .65 * pair[1].distance]
            result = {"key": f"{group}/{frame.stem}", "matches": len(good), "tracked": False}
            if len(good) >= 16:
                a = np.float32([keypoints[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
                b = np.float32([points[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
                matrix, inliers = cv2.findHomography(a, b, cv2.RANSAC, 2.5)
                if matrix is not None:
                    corners = cv2.perspectiveTransform(body, matrix).reshape(4, 2)
                    normalized = corners / [gray.shape[1], gray.shape[0]]
                    count = int(inliers.sum())
                    source_inliers = a[inliers.ravel().astype(bool), 0]
                    coverage = np.ptp(source_inliers, axis=0) / [image.shape[1], image.shape[0]]
                    convex = cv2.isContourConvex(corners.astype(np.float32))
                    area = cv2.contourArea(corners.astype(np.float32), oriented=True)
                    valid = (count >= 16 and count / len(good) >= .45 and min(coverage) > .10
                             and convex and area > 500 and np.isfinite(normalized).all()
                             and np.all((normalized >= 0) & (normalized <= 1)))
                    result.update(inliers=count, coverage=coverage.tolist(), tracked=bool(valid))
                    if valid:
                        manual[frame.stem] = normalized.tolist()
            tracking.append(result)
        geometry_path = directory / "manual-geometry-local.json"
        write_json(geometry_path, manual)
        subprocess.run([str(args.capture_binary.resolve()), str(frames), str(directory / "crops"),
                        str(geometry_path)], check=True)
        for record in map(json.loads, (directory / "crops" / "frames.jsonl").read_text().splitlines()):
            record.update(key=f"{group}/{record['frame']}", group=group, split=segment["split"],
                seconds=segment["start"] + int(record["frame"].split("-")[-1]) / segment["fps"])
            records.append(record)
            if record["outcome"] == "located":
                review[record["key"]] = {"aligned": False, "readableCells": [], "note": "unreviewed"}
                review_sheet(args.output, group, record, "".join(plan["truth"]))
        print(json.dumps({"group": group, "tracked": len(manual)}), flush=True)
    write_json(args.output / "records.json", records)
    write_json(args.output / "review.json", review)
    write_json(args.output / "tracking-local.json", tracking)
    write_json(args.output / "provenance.json", {"nativeProvenance": read_json(args.directory / "provenance.json"),
        "opencv": cv2.__version__, "referenceSha256": digest(args.reference),
        "captureBinarySha256": digest(args.capture_binary), "scriptSha256": digest(__file__), "localOnly": True})


def augment(image, rng):
    """Bounded imaging perturbations before Rust; labels still require review.

    No existing generator operates on captured raw cells while preserving their
    source grouping. Do not infer readability from blur width or erased area.
    """
    source = Image.fromarray(image)
    scale = rng.uniform(.93, 1.07)
    source = source.transform(source.size, Image.Transform.AFFINE,
        (scale, 0, rng.uniform(-1.5, 1.5), 0, scale, rng.uniform(-1.5, 1.5)),
        resample=Image.Resampling.BILINEAR, fillcolor=int(np.median(image)))
    source = source.filter(ImageFilter.GaussianBlur(rng.uniform(0, .65)))
    pixels = np.asarray(source, dtype=np.float32)
    if rng.random() < .35:
        pixels = sum(np.roll(pixels, shift, axis=rng.choice([0, 1]))
                     for shift in (-1, 0, 1)) / 3
    y, x = np.mgrid[:96, :96] / 95
    mode = rng.choice(["exposure", "shadow", "glare"])
    if mode == "exposure":
        pixels *= rng.uniform(.4, 1.1) + x * rng.uniform(-.12, .12)
    elif mode == "shadow":
        pixels *= .55 + .45 / (1 + np.exp(-(x + .3 * y - rng.uniform(.2, .8)) * 12))
    else:
        pixels += rng.uniform(20, 85) * np.exp(-((x-rng.random())/.35)**2 - ((y-rng.random())/.4)**2)
    pixels += np.random.default_rng(rng.getrandbits(32)).normal(0, rng.uniform(0, 1.8), pixels.shape)
    buffer = BytesIO()
    Image.fromarray(np.clip(pixels, 0, 255).astype(np.uint8)).save(buffer, format="JPEG", quality=rng.randint(65, 95))
    buffer.seek(0)
    return np.asarray(Image.open(buffer).convert("L"))


def sources(root):
    plan, review = load_plan(root / "plan.json"), read_json(root / "review.json")
    truth = "".join(plan["truth"])
    for record in read_json(root / "records.json"):
        if record["outcome"] != "located":
            continue
        checked = review[record["key"]]
        # Geometry failures/unreviewed grids remain in the frame manifest and
        # whole-camera replay, not mixed into a classifier-only benchmark.
        if checked["aligned"] is not True:
            continue
        readable = checked["readableCells"]
        if readable != "all" and (not isinstance(readable, list) or any(
                type(i) is not int or i not in range(70) for i in readable)):
            raise ValueError("Use readableCells: all or an explicit list of cell indices")
        for cell in record["cells"]:
            index = cell["cell"]
            known = checked["aligned"] is True and (readable == "all" or index in readable)
            yield record, cell, ord(truth[index])-65 if known else -1


def build(args):
    root = args.directory
    rng = random.Random(args.seed)
    # Build under local ignored storage. Only the explicitly selected NPZ may
    # leave this machine; the provenance sidecar is intentionally separate.
    args.output.mkdir(parents=True, exist_ok=False)
    buckets = {s: [] for s in SPLITS}
    lineage, seen, duplicate = [], {}, Counter()
    for record, cell, label in sources(root):
        raw = np.asarray(Image.open(root / record["group"] / "crops" / cell["file"]).convert("L"))
        split = record["split"]
        # Grouped within-split dedup only; holdouts retain every source for metrics.
        key = (record["group"], cell["cell"])
        thumbnail = np.asarray(Image.fromarray(raw).resize((24, 24)), dtype=np.float32)
        if split == "train":
            if label < 0:
                continue
            if any(np.abs(thumbnail - old).mean() < args.dedup_mae for old in seen.get(key, [])):
                duplicate[record["group"]] += 1
                continue
            seen.setdefault(key, []).append(thumbnail)
        source_id = f"{record['key']}/{cell['cell']}"
        buckets[split].append((raw, label, source_id, "original"))
        if split == "train":
            for index in range(args.augment):
                buckets[split].append((augment(raw, rng), -1, source_id, f"augment-{index}"))
    data, report = {}, {"deduplicated": dict(duplicate), "splits": {}}
    augmentation_review = {}
    for split, observations in buckets.items():
        if not observations:
            raise ValueError(f"No {split} sources; review readable training crops first")
        raw = np.stack([o[0] for o in observations])
        flags, original, corrected = prepare(raw, args.prepare_binary)
        labels = np.array([o[1] for o in observations], dtype=np.int64)
        # Augmented raw crops are not automatically readable even when the
        # source was. Export a local review sheet before admitting those labels.
        if split == "train" and args.augment:
            reviews = read_json(args.augmentation_review) if args.augmentation_review else {}
            approved = set(reviews.get("readable", []))
            for i, (_, _, source_id, variant) in enumerate(observations):
                if variant == "original":
                    continue
                token = hashlib.sha256((source_id + variant + str(args.seed)).encode() + raw[i].tobytes()).hexdigest()
                augmentation_review[token] = {"source": source_id, "variant": variant}
                Image.fromarray(raw[i]).save(args.output / f"{token}.png")
                if token in approved:
                    source_label = next(o[1] for o in observations if o[2] == source_id and o[3] == "original")
                    labels[i] = source_label
            if approved - set(augmentation_review):
                raise ValueError("Augmentation review contains stale IDs")
        order = list(range(len(observations)))
        rng.shuffle(order)
        if split == "train":
            valid = [i for i in order if flags[i] > 0 and labels[i] >= 0]
            dual = [i for i in valid if flags[i] == 2]
            data["train_images"] = np.concatenate((original[valid], corrected[dual]))
            data["train_labels"] = np.concatenate((labels[valid], labels[dual]))
            if not len(valid):
                raise ValueError("No supervised training views survived Rust preprocessing")
        else:
            data[split + "_images"] = original[order]
            data[split + "_corrected_images"] = corrected[order]
            data[split + "_flags"] = flags[order]
            data[split + "_labels"] = labels[order]
        for index in order:
            lineage.append({"split": split, "source": observations[index][2],
                            "variant": observations[index][3], "flag": int(flags[index]),
                            "label": int(labels[index])})
        counts = Counter(chr(65 + int(y)) for y in labels if y >= 0)
        report["splits"][split] = {"sources": len(observations), "unknown": int((labels < 0).sum()),
            "preprocessRejected": int((flags == 0).sum()), "dualViews": int((flags == 2).sum()),
            "supervisedSources": int(((flags > 0) & (labels >= 0)).sum()),
            "rejectedKnown": int(((flags == 0) & (labels >= 0)).sum()),
            "letters": dict(sorted(counts.items())), "missingLetters": [c for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ" if c not in counts]}
    # The remote training artifact excludes final-test pixels and labels too.
    np.savez_compressed(args.output / "training.npz", **{k:v for k,v in data.items() if not k.startswith("test_")})
    np.savez_compressed(args.output / "evaluation-local.npz", **{k:v for k,v in data.items() if not k.startswith("train_")})
    write_json(args.output / "augmentation-review-template.json", {"readable": [], "candidates": augmentation_review})
    write_json(args.output / "lineage-local.json", lineage)
    report.update(seed=args.seed, prepareSha256=digest(args.prepare_binary),
                  reviewSha256=digest(root / "review.json"), trainingSha256=digest(args.output / "training.npz"),
                  generatorSha256=digest(__file__))
    write_json(args.output / "report-local.json", report)
    print(json.dumps(report), flush=True)


def evaluate(args):
    import onnxruntime as ort
    from alphabet_calibration import fuse, infer_views, metrics
    ort.disable_telemetry_events()
    options = ort.SessionOptions()
    options.intra_op_num_threads = 2
    options.inter_op_num_threads = 1
    session = ort.InferenceSession(str(args.model), options, providers=["CPUExecutionProvider"])
    data = np.load(args.data, allow_pickle=False)
    lineage = read_json(args.lineage) if args.lineage else []
    report = {"modelSha256": digest(args.model), "dataSha256": digest(args.data), "splits": {}}
    for split in args.splits:
        start = time.perf_counter()
        raw, corrected, labels, flags = infer_views(session, data, split)
        prediction, confidence, conflicts = fuse(raw, corrected, flags)
        known = labels >= 0
        qualified = (prediction != 26) & (confidence >= .8)
        confusion = np.zeros((27, 27), dtype=np.int64)
        np.add.at(confusion, (labels[known], prediction[known]), 1)
        per_letter = {}
        for index in range(26):
            mask = labels == index
            per_letter[chr(65+index)] = {"total": int(mask.sum()),
                "correct": int((mask & (prediction == index)).sum()),
                "correctQualified": int((mask & qualified & (prediction == index)).sum()),
                "wrongQualified": int((mask & qualified & (prediction != index)).sum())}
        result = metrics(raw, corrected, labels, flags)
        result.update(modelBlank=int(((flags > 0) & (prediction == 26)).sum()),
                      lowConfidenceLetter=int(((flags > 0) & (prediction != 26) & (confidence < .8)).sum()),
                      preprocessingRejectedAll=int((flags == 0).sum()),
                      confusion=confusion.tolist(), perLetter=per_letter,
                      inferenceMs=(time.perf_counter()-start)*1000)
        if lineage:
            sources = [r for r in lineage if r["split"] == split]
            if len(sources) != len(labels) or any(r["label"] != int(y) or r["flag"] != int(f)
                                                for r, y, f in zip(sources, labels, flags)):
                raise ValueError("Lineage does not match evaluation observation order")
            frames = {}
            for index, source in enumerate(sources):
                frames.setdefault(source["source"].rsplit("/", 1)[0], []).append(index)
            complete, exact, qualified_exact = 0, 0, 0
            frame_reports = []
            for frame, indices in frames.items():
                wanted, actual = labels[indices], prediction[indices]
                readable = wanted >= 0
                correct = actual == wanted
                full = len(indices) == 70 and bool(readable.all())
                complete += int(full)
                exact += int(full and correct.all())
                qualified_exact += int(full and correct.all() and qualified[indices].all())
                frame_reports.append({"frame": frame, "readable": int(readable.sum()),
                    "correct": int((readable & correct).sum()),
                    "wrongQualified": int((readable & ~correct & qualified[indices]).sum())})
            result.update(fullyReadableCards=complete, exactCards=exact,
                          qualifiedExactCards=qualified_exact, frames=frame_reports)
        report["splits"][split] = result
    write_json(args.output, report)
    print(json.dumps({k: {f: v for f, v in r.items() if f not in ("confusion", "perLetter")}
                      for k, r in report["splits"].items()}), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("extract")
    command.add_argument("--plan", type=Path, required=True)
    command.add_argument("--capture-binary", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.set_defaults(run=extract)
    command = commands.add_parser("assist")
    command.add_argument("--directory", type=Path, required=True)
    command.add_argument("--reference", type=Path, required=True,
                         help="Local JSON: frame path, body corners and cardPolygon in reference pixels")
    command.add_argument("--capture-binary", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.set_defaults(run=assist)
    command = commands.add_parser("build")
    command.add_argument("--directory", type=Path, required=True)
    command.add_argument("--prepare-binary", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--augment", type=int, default=0)
    command.add_argument("--augmentation-review", type=Path)
    command.add_argument("--seed", type=int, default=20261020)
    command.add_argument("--dedup-mae", type=float, default=2)
    command.set_defaults(run=build)
    command = commands.add_parser("evaluate")
    command.add_argument("--data", type=Path, required=True)
    command.add_argument("--model", type=Path, required=True)
    command.add_argument("--lineage", type=Path, help="Local-only observation grouping for whole-card metrics")
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--splits", nargs="+", choices=["validation", "test"], default=["validation", "test"])
    command.set_defaults(run=evaluate)
    args = parser.parse_args()
    if getattr(args, "augment", 0) < 0 or getattr(args, "dedup_mae", 0) < 0:
        parser.error("Nonnegative augmentation count/dedup threshold required")
    args.run(args)


if __name__ == "__main__":
    main()
