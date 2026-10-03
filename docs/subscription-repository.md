# Subscription data repository

The future public repository supplies reviewed reference data to Campus Life. `index.json` has two independent maps: `academicYears` selects academic calendars, while `admissionYears` selects cohort-specific undergraduate guides. The app must never use the current academic year as a substitute for a missing admission year.

## Yearly update workflow

One GitHub Actions workflow follows the official calendar for each academic year. A separate workflow follows `https://www.titech.ac.jp/guide/guide_<admissionYear>/gakubu1/` and its 17 curriculum PDFs for each admission cohort. The agents read only cited official documents and must not infer missing dates, marks, credits, or course metadata.

Each workflow opens a pull request rather than publishing directly. A human reviewer checks every cited source, calendar exception, department scope, and recommended-course row against the official page/PDF. Merge is the publication gate. The app fetches only merged HTTPS JSON and validates each selected file again. A failure keeps the current data for that axis.

Recommended yearly checks:

1. Confirm the guide root contains the selected `admissionYear`, then confirm all 17 departments, syllabus scopes, unit codes, and guide-PDF URLs use that same guide version.
2. Compare quarter dates and every holiday, no-class day, and replacement day with the official calendar.
3. Compare each guide-table course code, group suffix, mark, credit split, competencies, content, and note.
4. Run schema validation and reject duplicate department IDs, calendar dates, and `(admissionYear, department, code, group)` rows.
5. Review the generated diff and merge only after a second person signs off.
