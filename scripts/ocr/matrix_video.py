"""Local card recordings -> production keyframe cells -> labelled OCR dataset.

The pipeline has three stages, each rerunnable on its own:

  export    Every recording runs through the Rust `matrix_dataset` example: the
            production tracker follows the video and every keyframe a live
            session would cut is cut by the production cell cut. No labels.
  build     Labels come from each card's human-checked truth by cell position.
            A keyframe is used only when Core finds it sharp
            (`matrix_frame_sharp`) and its indexing is verified (see
            `indexing`); a cell only when Core would admit it. Near-duplicate
            cells are dropped per recording and cell, the remaining cells go
            through Rust model preprocessing, and splits are written with
            lineage and a report.
  evaluate  A model on the validation/test split, per cell and per keyframe.

Splits are assigned per card in the manifest, so every recording of a card
lands in the same split: a held-out card measures generalisation to new cards,
which is what the app meets. Test cards are never used for model selection.
With too few cards for a validation card, `--validation-tail` selects
checkpoints on the end of each training recording instead.

Manifest (local JSON; paths relative to the manifest):

  {"cards": {"card-a": {"truth": ["ABCDEFGHIJ", ... 7 rows],
                        "truthSource": "human-checked", "split": "train"}},
   "recordings": [{"id": "a-desk", "card": "card-a", "video": "a-desk.mp4"}]}

Videos, cells, truth and every output here are card images or derived from
them: keep them local and out of commits.
"""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import random
import subprocess
import time

import numpy as np
from PIL import Image, ImageDraw

from alphabet_synthetic import prepare

SPLITS = ("train", "validation", "test")
ROOT = Path(__file__).resolve().parents[2]
PIXELS = 96 * 96
# Fields of each keyframe cell written by `matrix_dataset`.
CONTRAST, SHARPNESS, SIZE, CENTERING, GLARE, ADMISSIBLE, TEXT, CONFIDENCE, ACUITY = range(9)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text())


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def load_manifest(path, validation_tail=0.):
    path = Path(path).resolve()
    manifest = read_json(path)
    cards = manifest["cards"]
    for name, card in cards.items():
        rows = card["truth"]
        if len(rows) != 7 or any(len(r) != 10 or not r.isascii() or not r.isalpha()
                                 or r != r.upper() for r in rows):
            raise ValueError(f"{name}: truth must be seven rows of ten A-Z letters")
        if card.get("truthSource") != "human-checked":
            raise ValueError(f"{name}: OCR output cannot supply ground truth")
        if card.get("split") not in SPLITS:
            raise ValueError(f"{name}: split must be one of {SPLITS}")
    seen, identities = set(), {}
    for recording in manifest["recordings"]:
        rid = recording["id"]
        if not rid or not all(c.isalnum() or c in "-_" for c in rid) or rid in seen:
            raise ValueError("Recording ids must be unique and filesystem-safe")
        seen.add(rid)
        if recording["card"] not in cards:
            raise ValueError(f"{rid}: unknown card")
        recording["video"] = str((path.parent / recording["video"]).resolve())
        # Content identity: a renamed copy must not appear under two cards/splits.
        identity = digest(recording["video"])
        if identity in identities:
            raise ValueError(f"{rid} repeats the video of {identities[identity]}")
        identities[identity] = rid
        recording["sha256"] = identity
    splits = {c["split"] for c in cards.values()}
    if "train" not in splits or ("validation" not in splits and not validation_tail):
        raise ValueError("Assign a training card and a validation card or --validation-tail")
    return manifest


def binary(target, name):
    suffix = ".exe" if (Path(target) / "release/examples" / f"{name}.exe").exists() else ""
    return Path(target) / "release/examples" / f"{name}{suffix}"


def export(args):
    manifest = load_manifest(args.manifest)
    subprocess.run(["cargo", "build", "-p", "app-ocr", "--release", "--features",
                    "training-tools", "--example", "matrix_dataset", "--example",
                    "alphabet_prepare", "--target-dir", str(args.target)], cwd=ROOT, check=True)
    tool = binary(args.target, "matrix_dataset")
    tool_hash = digest(tool)
    for recording in manifest["recordings"]:
        out = args.output / recording["id"]
        stamp = out / "provenance.json"
        wanted = {"videoSha256": recording["sha256"], "exporterSha256": tool_hash}
        if stamp.exists() and read_json(stamp) == wanted and not args.force:
            print(f"{recording['id']}: up to date", flush=True)
            continue
        out.mkdir(parents=True, exist_ok=True)
        stamp.unlink(missing_ok=True)
        subprocess.run([str(tool), recording["video"], str(out)], check=True)
        write_json(stamp, wanted)


def keyframes(directory):
    """(row, 70 × 96 × 96 cells) for every exported keyframe of a recording."""
    glyphs = np.memmap(directory / "glyphs.u8", dtype=np.uint8, mode="r").reshape(-1, 70, 96, 96)
    for line in (directory / "frames.jsonl").read_text().splitlines():
        row = json.loads(line)
        if "keyframe" in row:
            yield row, glyphs[row["keyframe"]]


SHIFTS = [(dr, dc) for dr in (-1, 0, 1) for dc in (-1, 0, 1) if (dr, dc) != (0, 0)]


def indexing(cells, truth, minimum=8, margin=3.):
    """Is this keyframe's grid on the printed cells, not one row or column off?

    The tracker gates already reject most misindexed fits; this independent
    check counts confident model readings that match the truth at the fitted
    positions and at every one-cell shift. Letters differ between neighbours,
    so a shifted grid matches its shift, not the fitted positions. The reading
    only accepts or rejects the whole keyframe; every label still comes from
    the truth, including cells the model reads wrongly. A keyframe with fewer
    than `minimum` confident matches (heavy motion blur, mostly) cannot show
    its indexing and is left out.
    """
    def agree(dr, dc):
        count = 0
        for i, cell in enumerate(cells):
            r, c = i // 10 + dr, i % 10 + dc
            if 0 <= r < 7 and 0 <= c < 10 and cell[CONFIDENCE] >= .8 and cell[TEXT] == truth[r * 10 + c]:
                count += 1
        return count
    own = agree(0, 0)
    shifted = max(agree(dr, dc) for dr, dc in SHIFTS)
    return own >= minimum and own >= margin * shifted + 3, own, shifted


def sheet(path, cells, truth, keep):
    """A 7×10 review mosaic: truth under each cell, red for excluded cells."""
    image = Image.new("RGB", (10 * 96, 7 * 112), "white")
    draw = ImageDraw.Draw(image)
    for i in range(70):
        x, y = i % 10 * 96, i // 10 * 112
        image.paste(Image.fromarray(np.asarray(cells[i])).convert("RGB"), (x, y))
        draw.text((x + 3, y + 97), f"{truth[i]}", fill="black" if keep[i] else "red")
    image.save(path, quality=88)


def build(args):
    manifest = load_manifest(args.manifest, args.validation_tail)
    rng = random.Random(args.seed)
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "sheets").mkdir()
    buckets = {s: [] for s in SPLITS}
    report = {"recordings": {}, "splits": {}}
    for recording in manifest["recordings"]:
        rid, card = recording["id"], manifest["cards"][recording["card"]]
        truth, card_split = "".join(card["truth"]), card["split"]
        directory = args.exports / rid
        # With every card but the test card in training, the end of each
        # training recording selects checkpoints. Same card: optimistic, but
        # it never touches the test card; a gap keeps neighbouring frames out.
        boundary = np.inf
        if card_split == "train" and args.validation_tail:
            duration = max(json.loads(line)["t"] for line in (directory / "frames.jsonl").read_text().splitlines())
            boundary = duration * (1 - args.validation_tail)
        if read_json(directory / "provenance.json")["videoSha256"] != recording["sha256"]:
            raise ValueError(f"{rid}: export is stale; rerun export")
        stats = Counter()
        kept = {i: [] for i in range(70)}
        aligned_keys = []
        for row, cells in keyframes(directory):
            stats["keyframes"] += 1
            split = "validation" if row["t"] >= boundary else card_split
            if boundary - args.validation_gap <= row["t"] < boundary:
                stats["gap"] += 1
                continue
            # Core rejects a blurred keyframe whole in the app; every split
            # leaves it out the same way.
            if not row["sharp"]:
                stats["blurred"] += 1
                continue
            ok, own, shifted = indexing(row["cells"], truth)
            if not ok:
                stats["misindexed"] += 1
                if stats["misindexed"] <= args.sheets:
                    sheet(args.output / "sheets" / f"{rid}-rejected-{row['frame']:05}.jpg",
                          cells, truth, [False] * 70)
                continue
            stats["aligned"] += 1
            aligned_keys.append(row["frame"])
            admitted = [bool(c[ADMISSIBLE]) for c in row["cells"]]
            for i in range(70):
                if not admitted[i]:
                    stats["notAdmitted"] += 1
                    continue
                pixels = np.array(cells[i])
                # Dedup within one recording and cell: a card held still
                # gives near-identical cells that only reweight the data.
                thumb = np.asarray(Image.fromarray(pixels).resize((24, 24)), dtype=np.float32)
                if split == "train" and any(np.abs(thumb - old).mean() < args.dedup_mae
                                            for old in kept[i]):
                    stats["duplicates"] += 1
                    continue
                kept[i].append(thumb)
                reading = row["cells"][i]
                stats["modelCorrect"] += int(reading[TEXT] == truth[i])
                stats["modelWrongQualified"] += int(reading[TEXT] not in ("", truth[i])
                                                    and reading[CONFIDENCE] >= .8)
                buckets[split].append((pixels, ord(truth[i]) - 65, f"{rid}/{row['frame']}/{i}"))
                stats["cells"] += 1
        for frame in rng.sample(aligned_keys, min(args.sheets, len(aligned_keys))):
            row, cells = next((r, c) for r, c in keyframes(directory) if r["frame"] == frame)
            sheet(args.output / "sheets" / f"{rid}-aligned-{frame:05}.jpg", cells, truth,
                  [bool(c[ADMISSIBLE]) for c in row["cells"]])
        report["recordings"][rid] = {"card": recording["card"], "split": card_split,
                                     "validationFrom": None if np.isinf(boundary) else boundary, **stats}
        print(json.dumps({rid: report["recordings"][rid]}), flush=True)
    data, lineage = {}, []
    prepare_binary = binary(args.target, "alphabet_prepare")
    for split, observations in buckets.items():
        if not observations:
            continue
        order = list(range(len(observations)))
        rng.shuffle(order)
        raw = np.stack([observations[i][0] for i in order])
        labels = np.array([observations[i][1] for i in order], dtype=np.int64)
        flags, original, corrected = prepare(raw, prepare_binary)
        if split == "train":
            # Rust-rejected crops never reach the model; dual views both train.
            valid = np.flatnonzero(flags > 0)
            dual = np.flatnonzero(flags == 2)
            data["train_images"] = np.concatenate((original[valid], corrected[dual]))
            data["train_labels"] = np.concatenate((labels[valid], labels[dual]))
        else:
            data[f"{split}_images"] = original
            data[f"{split}_corrected_images"] = corrected
            data[f"{split}_flags"] = flags
            data[f"{split}_labels"] = labels
        for position, index in enumerate(order):
            lineage.append({"split": split, "source": observations[index][2],
                            "flag": int(flags[position]), "label": int(labels[position])})
        letters = Counter(chr(65 + int(y)) for y in labels)
        report["splits"][split] = {
            "cells": len(observations), "preprocessRejected": int((flags == 0).sum()),
            "dualViews": int((flags == 2).sum()), "letters": dict(sorted(letters.items())),
            "missingLetters": [c for c in map(chr, range(65, 91)) if c not in letters]}
    if "train_images" not in data or "validation_images" not in data:
        raise ValueError("Both a training and a validation card need verified keyframes")
    np.savez_compressed(args.output / "training.npz",
                        **{k: v for k, v in data.items() if not k.startswith("test_")})
    np.savez_compressed(args.output / "evaluation-local.npz",
                        **{k: v for k, v in data.items() if not k.startswith("train_")})
    write_json(args.output / "lineage-local.json", lineage)
    report.update(seed=args.seed, dedupMae=args.dedup_mae, validationTail=args.validation_tail,  prepareSha256=digest(prepare_binary),
                  manifestSha256=digest(args.manifest), generatorSha256=digest(__file__),
                  trainingSha256=digest(args.output / "training.npz"))
    write_json(args.output / "report-local.json", report)
    print(json.dumps(report["splits"]), flush=True)


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
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("export")
    command.add_argument("--manifest", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--target", type=Path, default=ROOT / "target")
    command.add_argument("--force", action="store_true")
    command.set_defaults(run=export)
    command = commands.add_parser("build")
    command.add_argument("--manifest", type=Path, required=True)
    command.add_argument("--exports", type=Path, required=True)
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--target", type=Path, default=ROOT / "target")
    command.add_argument("--seed", type=int, default=20261024)
    command.add_argument("--dedup-mae", type=float, default=2)
    command.add_argument("--sheets", type=int, default=6, help="Review mosaics per recording")
    command.add_argument("--validation-tail", type=float, default=0.,
                         help="Share at the end of each training recording used for validation")
    command.add_argument("--validation-gap", type=float, default=2.,
                         help="Seconds dropped before that validation tail")
    command.set_defaults(run=build)
    command = commands.add_parser("evaluate")
    command.add_argument("--data", type=Path, required=True)
    command.add_argument("--model", type=Path, required=True)
    command.add_argument("--lineage", type=Path, help="Local-only observation grouping for whole-card metrics")
    command.add_argument("--output", type=Path, required=True)
    command.add_argument("--splits", nargs="+", choices=["validation", "test"], default=["validation", "test"])
    command.set_defaults(run=evaluate)
    args = parser.parse_args()
    if getattr(args, "dedup_mae", 0) < 0 or not 0 <= getattr(args, "validation_tail", 0) < 1:
        parser.error("Nonnegative dedup threshold and a validation tail in [0, 1) required")
    args.run(args)


if __name__ == "__main__":
    main()
