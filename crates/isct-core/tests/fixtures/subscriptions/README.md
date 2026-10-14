# Subscription fixtures

These files are a byte-preserving snapshot of the public Isct-Data index,
datasets, and schemas formerly bundled into `isct-core`. They exist only for
validation, persistence, and UI acceptance tests. Production refreshes read the
remote subscription selected by the user; ordinary builds must not embed these
fixtures.

Keep `index.json`, its referenced data files, and `index.sha256` together. When
refreshing the snapshot intentionally, preserve the remote bytes and update the
hash file plus every acceptance assertion affected by the new source data.
