# 教務Web maintenance

The Core browser reads the current document before accepting a 教務Web load
as a successful login or starting extraction. The same notice reader is used
for fetched course-search responses. Maintenance dates and recurring periods
come from the live notice; there is no built-in maintenance calendar.

Maintenance has its own ingest state and typed browser/service/FFI error.
It ends the matching hidden navigation or sign-in promptly, without immediate
coordinator retries, credential rejection, or applying an empty collection.
Existing courses, grades, settings, credentials and successful-import timestamps
are retained. A subsequent healthy document clears the service notice, and a
successful data extraction can restore a session interrupted by maintenance.

The current notice is included in the browser snapshot, independently of saved
credentials. The desktop status bar shows maintenance and the live notice;
hovering the message reveals its full text. The account panel also distinguishes
an interrupted sign-in from invalid credentials.

Regression coverage lives in the existing school extractor, browser, service,
coordinator and app-shell suites. It covers changed notice dates, future notices,
foreign-host rejection, maintenance redirects during search POSTs, preservation
of stored rows and import metadata, prompt waiter completion, retry suppression,
service recovery and the footer's maintenance/progress display.

No signed application was packaged or replaced for this change. Native rendering
against a real maintenance page remains unverified; automated browser-port tests
exercise the relay and navigation lifecycle using scripted host callbacks.
