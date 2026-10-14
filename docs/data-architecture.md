# School data architecture

## Boundary

The school does not provide one stable JSON API. HTML is an input format, not the application model. UI components never parse HTML and parsers never write directly to the workspace.

```text
official page
  -> bounded acquisition
  -> provider parser
  -> normalized import envelope
  -> schema and identity validation
  -> reconciliation
  -> atomic cache publication
  -> read model for the UI
```

## Acquisition strategies

### Authenticated pages

Portal and LMS pages load in a persistent native WebView. A fixed, app-owned extractor runs against the already parsed DOM and returns a bounded data-transfer object. This is preferable to copying authenticated HTML because the DOM has already handled malformed markup and the returned object can exclude navigation chrome, CSRF values and personal account data.

Authenticated HTML is not persisted by default. Sanitized fixtures are created deliberately for parser tests.

### Public syllabus

The native layer performs bounded HTTPS GET requests to an allowlisted host. Responses are checked for status, content type, size and final URL, then parsed with structural HTML selectors. Regular expressions remain appropriate only for leaf values such as a quarter or period string, never for discovering the document hierarchy.

## Provider adapters

Each source owns an independent adapter:

- `syllabus`: search results and offering details
- `t2schola`: memberships, sections, assignments and materials
- `portal`: notices and service links
- future services: separate adapters rather than conditions in one generic scraper

Selectors are local to that source's parser. A site change can therefore fail one source without invalidating the rest of the workspace.

### Rust module boundary

`crates/isct-core/src/school_parser/` is the single parsing boundary. Its provider
modules accept bounded HTML or extractor JSON and return typed, validated
values. They do not open network connections, access Tauri, write files, or
write SQLite.

- `school_parser::syllabus` owns public syllabus HTML selectors.
- `school_parser::lms` owns LMS payload validation and module-page HTML selectors.
- `school_parser::kyomu` owns the trust boundary for captured 教務 page data.

The WebView scripts are acquisition adapters for pages that require a live
browser DOM or Moodle session values; their bounded output is not trusted
until the Rust parser accepts it. `lms_downloader` remains a separate service:
it owns cookies, HTTP, concurrency, temporary files and cache writes, then
passes response HTML to `school_parser::lms`. Ingest modules own only
reconciliation and persistence.

## Import envelope

Every parser returns the same outer contract:

- provider and sanitized source URL
- acquisition time
- state: `fresh`, `partial`, `cached`, `reconnect-required`, `unsupported` or `error`
- normalized records
- non-sensitive warnings
- completeness scope, for example `all-courses` or `single-course`

An empty record list is never interpreted as deletion unless the parser positively identifies a complete, authenticated empty collection.

## Identity and provenance

- A syllabus offering is identified by its full offering path/ID plus academic year, not by title or course code.
- An LMS membership is separate from the academic course offering and may be linked provisionally by code and term.
- Assignments and materials retain their source item identity and parent membership.
- Every normalized record stores provider, external identity, sanitized source URL and fetched time.
- Local title, colour and planning changes are overlays. Refreshing school data does not erase them.

## Validation and reconciliation

1. Reject untrusted hosts, credential-bearing URLs, oversized documents and unexpected redirects.
2. Validate field lengths, dates, terms and parent relationships.
3. Publish fresh records only after the complete envelope passes validation.
4. Preserve the last valid cache on login, network or parser failure.
5. Tombstone missing records only after a successful full-scope scan. Partial scans only upsert.
6. Redact URLs, HTML, cookies, tokens and page text from normal diagnostics.

## Parser testing

Each adapter has:

- small sanitized HTML fixtures
- expected normalized-record fixtures
- tests for Japanese and English labels
- tests for missing fields and reordered markup
- login-page and partial-page fixtures
- size and identity limits

Production failures record the provider, category and timestamp. They do not record raw authenticated content.

## Extension boundary

Site JavaScript executes only inside its allowlisted site WebView and cannot access the application store. App hooks execute against a narrow API with copied read models and explicit actions; they cannot access WebView DOM, cookies or session storage. No generic bridge connects the two environments.

Authentication secrets are outside both the workspace and extension boundary. A small native authentication coordinator may request exact secret fields from the operating-system credential vault and fill only audited official login forms. It returns state such as `signed-in` or `reconnect-required`, never secret values.

## Development reset policy

The current pre-release build does not preserve compatibility with the previous demo. Changes replace the local model and UI directly. Existing local demo data may be cleared and reseeded rather than migrated; compatibility work starts only after the product and source contracts stabilize.
