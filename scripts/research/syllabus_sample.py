"""Sample 100 syllabus offerings (en + ja) and check which fields parse 100% reliably."""
import json, os, random, re, subprocess, sys, time
from collections import Counter, defaultdict
from bs4 import BeautifulSoup

ORIGIN = "https://syllabus.s.isct.ac.jp"
UA = "Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 Safari/605.1.15"
# Pages and the report are written next to this script; both are git-ignored.
HERE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "syllabus-sample-output")
CACHE = os.path.join(HERE, "pages")
os.makedirs(CACHE, exist_ok=True)  # also creates HERE

DEPARTMENTS = """1/0-901-311100-0-0 1/0-901-311200-0-0 1/0-901-311300-0-0 1/0-901-311400-0-0
2/0-902-321500-0-0 2/0-902-321600-0-0 2/0-902-321700-0-0 2/0-902-321800-0-0 2/0-902-321900-0-0
3/0-903-332000-0-0 3/0-903-332100-0-0 4/0-904-342200-0-0 4/0-904-342300-0-0 5/0-905-352400-0-0
6/0-906-362500-0-0 6/0-906-362600-0-0 6/0-906-362700-0-0""".split()
LIBERAL_CODES = ["LAH", "LAE", "LAS", "LAW", "LAL", "ENT", "XCO"]


def fetch(url, path):
    if os.path.exists(path) and os.path.getsize(path) > 1000:
        return open(path, encoding="utf-8").read()
    out = subprocess.run(["curl", "-sSL", "--max-time", "30", "-A", UA, url],
                         capture_output=True, text=True).stdout
    time.sleep(0.4)
    with open(path, "w", encoding="utf-8") as f:
        f.write(out)
    return out


def collect():
    ids = {}
    sources = [f"{ORIGIN}/courses/2026/{d}?hl=en" for d in DEPARTMENTS]
    sources += [f"{ORIGIN}/search?code={c}&year=2026&hl=en" for c in LIBERAL_CODES]
    for i, url in enumerate(sources):
        page = fetch(url, os.path.join(CACHE, f"list-{i}.html"))
        for a in BeautifulSoup(page, "html.parser").select('a[href*="/courses/2026/"]'):
            m = re.match(r"https://syllabus\.s\.isct\.ac\.jp(/courses/2026/[^?]+/(\d{9}))", a["href"])
            if m:
                ids[m.group(2)] = m.group(1)
    return ids


def ws(text):
    return re.sub(r"\s+", " ", text or "").strip()


def parse(html):
    soup = BeautifulSoup(html, "html.parser")
    h1 = soup.select_one("h1.c-h1")
    items = []
    for item in soup.select("dl.c-dl-2col > div.c-dl-2col__item"):
        dt, dd = item.select_one("dt"), item.select_one("dd")
        items.append((ws(dt.get_text(" ")) if dt else None, dd))
    return {
        "title": ws(h1.get_text()) if h1 else None,
        "items": items,
        "sections": [ws(h.get_text()) for h in soup.select("h3.c-h3")],
        "competencies": [(ws(li.get_text()), "is-check-list-checked" in (li.get("class") or []))
                         for li in soup.select("ul.c-check-list > li")],
        "plan_rows": len(soup.select("table#lecture_plans tbody tr")),
        "is_login_or_error": not h1,
    }


LABELS = {
    "en": ["Academic unit or major", "Instructor(s)", "Class Format", "Media-enhanced courses",
           "Day of week/Period (Classrooms)", "Class", "Course Code", "Number of credits",
           "Course offered", "Offered quarter", "Syllabus updated", "Language"],
    "ja": ["開講元", "担当教員", "授業形態", "メディア利用科目", "曜日・時限 (講義室)", "クラス",
           "科目コード", "単位数", "開講時期", "開講クォーター", "シラバス更新日", "使用言語"],
}
KEYS = ["unit", "instructors", "format", "media", "schedule", "class", "code", "credits",
        "year", "quarter", "updated", "language"]
DAY = {"Mon": 1, "Tue": 2, "Wed": 3, "Thu": 4, "Fri": 5, "Sat": 6, "Sun": 7,
       "月": 1, "火": 2, "水": 3, "木": 4, "金": 5, "土": 6, "日": 7}
MONTHS = {m: i + 1 for i, m in enumerate("Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split())}


def leaf(key, dd, lang):
    """Returns (parsed_value or None, raw). None means it did not parse."""
    raw = ws(dd.get_text(" ")) if dd is not None else ""
    if key == "credits":
        spans = [ws(s.get_text()) for s in dd.select("span")] if dd is not None else []
        ok = "is-dl-number-of-credits" in (dd.get("class") or []) and len(spans) == 3 and all(s.isdigit() for s in spans)
        return (tuple(map(int, spans)) if ok else None), "|".join(spans) or raw
    if key == "code":
        return (raw if re.fullmatch(r"[A-Z]{3}\.[A-Z]\d{3}(-\d+)?", raw) else None), raw
    if key == "year":
        m = re.fullmatch(r"(\d{4})(年度)?", raw)
        return (int(m.group(1)) if m else None), raw
    if key == "quarter":
        m = re.fullmatch(r"([1-4])(?:\s*[-～~]\s*([1-4]))?Q", raw)
        if not m:
            return None, raw
        a, b = int(m.group(1)), int(m.group(2) or m.group(1))
        return tuple(range(a, b + 1)), raw
    if key == "updated":
        if raw in ("-", ""):
            return "none", raw
        m = re.fullmatch(r"([A-Z][a-z]{2}) (\d{1,2}), (\d{4})", raw)
        if m and m.group(1) in MONTHS:
            return f"{m.group(3)}-{MONTHS[m.group(1)]:02d}-{int(m.group(2)):02d}", raw
        m = re.fullmatch(r"(\d{4})年(\d{1,2})月(\d{1,2})日", raw)
        if m:
            return f"{m.group(1)}-{int(m.group(2)):02d}-{int(m.group(3)):02d}", raw
        return None, raw
    if key == "schedule":
        if raw in ("-", ""):
            return "none", raw
        meetings = []
        for part in [p.strip() for p in raw.split(" / ")]:
            if lang == "en":
                m = re.fullmatch(r"(\d{1,2})-(\d{1,2}) (Mon|Tue|Wed|Thu|Fri|Sat|Sun)(?: \(([^()]*)\))?", part)
                if m:
                    meetings.append((DAY[m.group(3)], int(m.group(1)), int(m.group(2)), m.group(4) or ""))
                    continue
            else:
                m = re.fullmatch(r"([月火水木金土日])(\d{1,2})-(\d{1,2})(?: \(([^()]*)\))?", part)
                if m:
                    meetings.append((DAY[m.group(1)], int(m.group(2)), int(m.group(3)), m.group(4) or ""))
                    continue
            return None, raw
        return tuple(meetings), raw
    return raw, raw  # free/enum text: always "parses"; distinct values reported


def main():
    ids = collect()
    print(f"candidates: {len(ids)}")
    random.seed(20260929)
    sample = random.sample(sorted(ids), min(100, len(ids)))
    label_sets = {"en": Counter(), "ja": Counter()}
    failures = defaultdict(list)
    distinct = defaultdict(Counter)
    sections = {"en": Counter(), "ja": Counter()}
    mismatches = defaultdict(list)
    comp_counts, plan_counts, errors = Counter(), Counter(), []
    for n, oid in enumerate(sample, 1):
        path = ids[oid]
        parsed = {}
        for lang, url in (("en", f"{ORIGIN}{path}?hl=en"), ("ja", f"{ORIGIN}{path}")):
            page = parse(fetch(url, os.path.join(CACHE, f"{oid}-{lang}.html")))
            if page["is_login_or_error"]:
                errors.append((oid, lang)); continue
            labels = tuple(l for l, _ in page["items"])
            label_sets[lang][labels == tuple(LABELS[lang])] += 1
            if labels != tuple(LABELS[lang]):
                failures[f"labels-{lang}"].append((oid, labels))
            by_label = dict(page["items"])
            values = {}
            for key, label in zip(KEYS, LABELS[lang]):
                value, raw = leaf(key, by_label.get(label), lang)
                values[key] = value
                if value is None:
                    failures[f"{key}-{lang}"].append((oid, raw))
                if key in ("format", "media", "class", "language", "unit"):
                    distinct[f"{key}-{lang}"][raw] += 1
            for h in page["sections"]:
                sections[lang][h] += 1
            comp_counts[(lang, len(page["competencies"]))] += 1
            plan_counts[(lang, page["plan_rows"] > 0)] += 1
            parsed[lang] = values
        if len(parsed) == 2:
            for key in ("code", "year", "quarter", "credits", "schedule", "updated"):
                a, b = parsed["en"][key], parsed["ja"][key]
                if a is not None and b is not None and a != b:
                    mismatches[key].append((oid, a, b))
        print(f"\r{n}/{len(sample)}", end="", file=sys.stderr)
    print(file=sys.stderr)
    report = {
        "sampled": len(sample), "error_pages": errors,
        "label_sets_exact": {k: dict(v) for k, v in label_sets.items()},
        "failures": {k: {"count": len(v), "examples": v[:6]} for k, v in failures.items()},
        "en_ja_mismatch": {k: {"count": len(v), "examples": v[:6]} for k, v in mismatches.items()},
        "distinct": {k: v.most_common(15) for k, v in distinct.items()},
        "sections": {k: v.most_common(30) for k, v in sections.items()},
        "competency_list_sizes": {f"{l}:{c}": n for (l, c), n in comp_counts.items()},
        "has_lecture_plan": {f"{l}:{h}": n for (l, h), n in plan_counts.items()},
    }
    json.dump(report, open(os.path.join(HERE, "syllabus_sample_report.json"), "w"), ensure_ascii=False, indent=1, default=str)
    print(json.dumps(report, ensure_ascii=False, indent=1, default=str)[:12000])


if __name__ == "__main__":
    main()
