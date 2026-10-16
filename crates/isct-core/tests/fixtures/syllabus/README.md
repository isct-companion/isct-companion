# Public syllabus regression fixtures

Frozen public syllabus HTML excerpts, parsed course examples and provenance used
by Core parser tests and desktop acceptance tests. They were previously stored
under ignored local research output in `artifacts/syllabus-research/`, which made
a clean CI checkout unable to compile the tests. Only the fixtures required by
those tests are kept here. Local research output remains ignored.

`content-provenance.json` pins the public URL and SHA-256 of every excerpt. These
files contain public catalog text, not authenticated school records or
distributed LMS files. Fixtures are test-only and are not embedded in production
Core/FFI libraries.

## How an excerpt is taken

One rule, applied to every file, so that a refresh is reproducible:

> fetch the pinned URL, then keep the bytes from the first `<h1 class="c-h1">`
> to the first `<aside`, with CRLF normalised to LF.

`https://syllabus.s.isct.ac.jp/courses/...?hl=ja` redirects to the same path
without the query; follow redirects when fetching. Navigation, scripts and page
chrome fall outside that range by construction.

## Refresh history

| Date | Change |
| --- | --- |
| 2026-09-10 | First capture of the excerpts below. |
| 2026-10-16 | Every excerpt re-fetched and byte-verified against its URL. `social-ja`, `object-en`, `assembly-en`, `programming-one-en` and `logic-en` had changed: the site had filled in the in-term timetable and rooms (`Day of week/Period` moved from `unknown` to real periods and classrooms), added a delivery mode to `Class Format` (`Lecture` became `Lecture (Face-to-face)`), and populated `Media-enhanced courses`. The Japanese page also switched its line breaks from `<br>` to `<br />` and dropped a wrapper element before the heading, which is why the capture rule above is stated explicitly. |

The pre-term shape that the earlier capture recorded — `Day of week/Period` of
`unknown`, an empty `Media-enhanced courses`, and a `Class Format` without a
delivery suffix — is a state the site returns every year before a quarter
starts, and Core must keep importing such a page rather than failing it. That
shape is now covered by an inline synthetic fixture in the Core syllabus parser
tests instead of a whole captured page, so refreshing these files cannot lose
the contract.
