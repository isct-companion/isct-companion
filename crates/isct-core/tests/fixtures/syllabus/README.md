# Public syllabus regression fixtures

Frozen public syllabus HTML excerpts, parsed course examples and provenance used
by Core parser tests and desktop acceptance tests. They were previously stored
under ignored local research output in `artifacts/syllabus-research/`, which made
a clean CI checkout unable to compile the tests. Only the fixtures required by
those tests are kept here. Local research output remains ignored.

`content-provenance.json` pins observed public URLs, capture dates and SHA-256
for the content excerpts. Navigation/scripts were removed during the original
capture. These files contain public catalog text, not authenticated school
records or distributed LMS files. Fixtures are test-only and are not embedded
in production Core/FFI libraries.
