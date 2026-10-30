# Data ownership and local storage

This document is the authority for persistence boundaries. Cloud transport and
merge behavior are deliberately out of scope.

| Data                                                                                            | Owner                 | Storage                                                         | Deletion / lifetime                                             |
| ----------------------------------------------------------------------------------------------- | --------------------- | --------------------------------------------------------------- | --------------------------------------------------------------- |
| LMS courses, memberships, assignments and material metadata                                     | School                | SQLite school-cache tables                                      | `lastSeenAt` / `missingSince`; never a user tombstone           |
| Kyomu registrations, grades, GPA and requirements                                               | School                | SQLite school-cache tables                                      | `lastSeenAt` / `missingSince`; refreshed from the school site   |
| Syllabus offerings and detail content                                                           | Official source       | SQLite reference cache                                          | Refresh TTL; may be rebuilt                                     |
| Academic calendar, departments and recommendations                                              | Official subscription | SQLite reference cache                                          | `lastSeenAt` / `missingSince`; may be rebuilt                   |
| Assignment marks and course overrides                                                           | User                  | Dedicated SQLite overlay tables                                 | Explicit soft delete only; overlays survive missing school rows |
| Events, plans, plan items and manual courses                                                    | User                  | SQLite user tables                                              | Explicit soft delete only                                       |
| Markdown course notes                                                                           | Local user            | Course notes directory, local export and workspace cloud backup | Never removed by cache cleanup                                  |
| Downloaded school attachments and thumbnails                                                    | Cache                 | Course materials directory / memory                             | May be evicted by cache policy                                  |
| Search indexes and parsed projections                                                           | Derived cache         | SQLite FTS / memory                                             | May be rebuilt                                                  |
| Appearance, locale, school content language, active view, download path and refresh preferences | Device                | Device settings                                                 | Local to one device                                             |
| Import runs, refresh timestamps, logs and connection state                                      | Runtime               | SQLite runtime tables                                           | Local diagnostic/refresh state                                  |
| School credentials and OTP material                                                             | Secret                | OS credential vault; optional AES-256-GCM cloud file            | Cloud sync is opt-in and never uploads plaintext                |
| OAuth tokens and cloud credential-encryption password                                           | Secret                | OS credential vault                                             | Device-local; never exported or returned to the WebView         |
| School WebView cookies and restore location                                                     | Session/device        | WebView store / app data                                        | Session or explicit restore preference                          |

## Reset

Settings offers two resets, both decided in Core (`crates/app-core/src/reset.rs`,
`data/reset_store.rs`) and affecting only this device; neither writes a tombstone.

- **Clear school caches and resync** deletes the school, reference and derived
  caches (every school-cache table, `offerings`, `offerings_fts`),
  `dataset_state`, `sync_runs`,
  the cache settings rows (`kyomuAcademicIdentity`, `kyomuSearchOptions:*`),
  the refresh bookkeeping inside subscription rows and the LMS token. Options
  also remove the school WebView cookies and downloaded attachments. User
  tables, overlays, subscriptions (kind, URL, on/off), scheduled tasks, settings,
  saved credentials, notes and `course_identity_map` (the course ids overlays and
  plans point at) stay.
- **Full reset** deletes every table and settings row, the saved school
  credentials and the WebView cookies, then writes first-launch defaults with a
  new device id. Cloud sync is disconnected first with a local recovery point.
  The course folder (notes and downloads), cloud copies, cloud authorization
  and logs stay.

## Stable source keys

School entities are addressed by an opaque `sourceKey` generated by the Rust
backend. The frontend must never construct one.

```
lms:{accountScope}:course:{lmsCourseId}
lms:{accountScope}:assignment:{cmid}
lms:{accountScope}:material:{cmid}
kyomu:{accountScope}:{recordKind}:{externalId}
syllabus:{academicYear}:offering:{offeringCode}
reference:{dataset}:{version}:{recordId}
```

`accountScope` is a stable, non-secret account identifier. Until multiple
accounts are supported it is `default`; it must not contain a username or
credential. User-created records use backend-generated UUIDv7 IDs.

Overlays reference `sourceKey`, not a device-local database ID. A missing
school row hides its overlay from composed UI views but does not delete it.

## Time policies

- A refresh TTL controls when a source should be fetched again.
- An eviction TTL controls when disposable bytes may be removed.
- Session and credential expiry are lifetime policies, not refresh TTLs.
- User data has no eviction TTL.

The backend contract is the single source of policy values.

## Cloud files

Rust owns cloud authorization, provider requests, conflict detection, export
and restore. The WebView can start an operation and display its result, but it
cannot read tokens, workspace export bytes, school credentials or encryption
keys.

- `IsctCompanion.json` is the readable versioned workspace export in the
  provider's private app folder.
- `IsctCompanion.credentials.json` is optional and contains only an authenticated
  encrypted school-credential bundle.
- A new credential encryption password is 12–128 characters (Core's
  `cloud_credentials::PASSWORD_LENGTH`; a passphrase fits) and stays in a
  separate device-local credential-vault entry, saved only after the backup it
  opened or wrote succeeded. Another device must be given the same password
  explicitly.
