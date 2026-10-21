# Task 2 — Syllabus catalog, both languages

Superseded (2026-10-21). The app no longer fetches, parses or stores シラバス
pages, and the Isct-Data catalog pack is gone.

- The catalog is the 教務Web course search, one `catalog.official/{year}/{prefix}`
  sync key per department (`crates/isct-core/src/catalog.rs`). In English
  content, `{year}/{prefix}/en` reads the English search page and adds only the
  English title and instructor names; it is planned only while the person is
  not viewing 教務Web, because the language is a property of the shared session.
- "Syllabus" opens the school's own page inside the app. Core builds the one
  link, `https://syllabus.s.isct.ac.jp/courses/-/-/-/{offering id}` with `?hl=en`
  in English (`catalog::syllabus_url`); shells show it as given.

The earlier design (bilingual detail pages, stored syllabus content, a yearly
catalog pack) is in git history.
