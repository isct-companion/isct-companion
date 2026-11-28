# Subscription fixtures

These files are a byte-preserving snapshot of the public Isct-Data index,
datasets, and schemas formerly bundled into `app-core`. They exist only for
validation, persistence, and UI acceptance tests. Production refreshes read the
remote subscription selected by the user; ordinary builds must not embed these
fixtures.

Keep `index.json` and its referenced data files together. When refreshing the
snapshot intentionally, preserve the remote bytes. Checks on the data itself
(counts, sources, cohorts) belong in Isct-Data, not here.
