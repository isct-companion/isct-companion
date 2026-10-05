# Official public data agent: extraction and review contract

This is prompt material for a future GitHub agentic workflow. No workflow is enabled by this document. Use `Gliese129/Isct-Data` as the published dataset; the app defaults to its raw `index.json`.

## Scope and layout

Read the university's official sources for the selected academic/admission years. Publish datasets under category folders with year filenames: `academic-calendar/{year}.json`, `important-dates/{year}.json`, `exam/{level}/{year}-{quarter-lowercase}.json`, and `departments/{admissionYear}.json`. Create files only when official data is available and verified. Do not create empty placeholder files for unavailable examination years/quarters. List only existing published files in the index and explain gaps in the README. Use the repository JSON schemas, update the index, and preserve verified historical datasets. Academic years include dates through March of the next civil year; admission cohorts identify their own guide PDFs.

## Understand the document before extracting

Determine the meaning of each table's headers and symbols for each department independently. Read the guide introduction, course appendices, 科目体系図, standard study plans, and required/recommended markers together. The `recommended` collection is the app's department curriculum reference, including courses on curriculum maps and standard plans; it is not limited to sections literally titled "recommended". A parseable table is not the only valid evidence.

Preserve source page numbers, source kind, course context, and original published codes. Match map course titles to official course-table codes when available. Normalize a published code suffix only when the app's syllabus identifier convention confirms it; preserve the full printed identifier in the notes. Merge repeated evidence for the same course without losing pages or context.

Check text extraction against rendered pages before bulk parsing. Some guide PDFs have broken font mappings: apparently readable Japanese can accompany shifted Latin course codes, and other PDFs require OCR for both scripts. Repair or OCR the source, then compare the complete course-code set with the extracted records. A list of newly found omissions is not a replacement for the complete department dataset. Read columns and continuation lines: a code in a parenthesized equivalence or offering-department note is not automatically an additional course row.

Set `mark` only when the document's symbol legend supports required or recommended. Preserve conditional requirements and selection rules in notes; do not invent a compulsory/recommended mark for every course appearing on a map. Learning-content letters A/B/C belong in `content`; they are not automatically A群/B群. A missing group or credit allocation may be omitted rather than invented. An empty curriculum array requires an explanation backed by a completed document review, not a failed parser.

## Calendar semantics

Separate overall quarter bounds, fragmented regular teaching intervals, and exam/makeup intervals. Do not join the first and last teaching dates across intervening exam days. Add verified Japanese national and substitute holidays using Cabinet Office data. Where the university explicitly schedules replacement teaching on a holiday, its instruction takes priority. Keep campus festivals, ceremonies, orientations and entrance examinations as important events. Only an explicit teaching exception suppresses classes. Include festival preparation/cleanup days where the university states classes are cancelled. Intensive-course dates may occur on days when ordinary classes are cancelled.

## Exams and languages

Check both undergraduate and graduate official entries and PDFs. A source URL may be overwritten with the newest quarter; verify the year and quarter inside the downloaded document. Distinguish final examinations from makeup teaching rows. Check struck-through, deleted and replacement rows visually. An allocated period pair is not evidence of exact examination duration. Omit both clock fields if exact times are unpublished; when exact times are published, supply both. Missing historical feeds must be documented as unavailable, not as evidence that no examinations occurred.

Extract official Japanese and English text whenever published. Mark generated English translations separately from official English wording. Preserve stable IDs, original source URLs, source page numbers, publication date where available, actual retrieval timestamp and PDF SHA-256. Do not invent a retrieval time or an unavailable historical source.

## Acceptance review

1. Validate every JSON file against the matching schema and check cross-field relations, references, unique identities, ordered intervals and academic/admission year consistency.
2. Compare each department's extracted course-code set against the complete PDF; review unmatched codes, continuation rows, multi-column diagrams and unexplained empty arrays.
3. Check every department and every year. Do not let a working CS parser define the scope of all other departments.
4. Visually review diagrams and uncertain table rows; retain the reasoning and page evidence for unresolved cases.
   Check wrapped titles and mixed table columns, including unbalanced parentheses, category labels in course names and missing official English text. Similar codes across years do not prove identical published titles.
5. Verify important-date coverage, national holidays, explicit school overrides, fragmented teaching intervals, and replacement/cancelled examination rows.
6. Report actual coverage and unresolved evidence. JSON syntax validation alone is not extraction completion.

## Workflow CLI tools

Keep scripts only when the workflow can actually invoke them. Accept dataset/source locations and target years as CLI arguments instead of hard-coding a local temporary directory. Provide machine-readable review results, documented dependencies and meaningful exit codes for success, review findings and input failures. Missing source files must be reported rather than silently skipped. Extraction helpers must state when they used OCR or repaired font mappings; a future agent remains responsible for resolving semantic review findings against the official pages.
