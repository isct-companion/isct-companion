BEGIN IMMEDIATE;

CREATE TABLE catalog_search_items (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE manual_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE manual_memberships (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);

-- Search rows are intentionally incomplete and must never deserialize as a
-- full Offering entity.
INSERT INTO catalog_search_items(id, value, updated_at)
SELECT id, value, updated_at FROM offerings
WHERE json_type(value, '$.format') IS NULL
   OR json_type(value, '$.schedule') IS NULL
   OR json_type(value, '$.credits') IS NULL;
DELETE FROM offerings
WHERE json_type(value, '$.format') IS NULL
   OR json_type(value, '$.schedule') IS NULL
   OR json_type(value, '$.credits') IS NULL;

INSERT INTO manual_courses
SELECT id,value,hash,updated_at,updated_by,rev,source,deleted_at FROM timetable_courses
WHERE source NOT IN ('lms','kyomu','school');
DELETE FROM timetable_courses WHERE source NOT IN ('lms','kyomu','school');

INSERT INTO manual_memberships
SELECT id,value,hash,updated_at,updated_by,rev,source,deleted_at FROM memberships
WHERE COALESCE(source, json_extract(value,'$.evidence.provider'), '') NOT IN ('lms','kyomu','school');
DELETE FROM memberships
WHERE COALESCE(source, json_extract(value,'$.evidence.provider'), '') NOT IN ('lms','kyomu','school');

ALTER TABLE timetable_courses ADD COLUMN last_seen_at TEXT;
ALTER TABLE timetable_courses ADD COLUMN missing_since TEXT;
ALTER TABLE memberships ADD COLUMN last_seen_at TEXT;
ALTER TABLE memberships ADD COLUMN missing_since TEXT;
ALTER TABLE assignments ADD COLUMN last_seen_at TEXT;
ALTER TABLE assignments ADD COLUMN missing_since TEXT;
ALTER TABLE offerings ADD COLUMN last_seen_at TEXT;
ALTER TABLE offerings ADD COLUMN missing_since TEXT;
ALTER TABLE lms_courses ADD COLUMN last_seen_at TEXT;
ALTER TABLE lms_courses ADD COLUMN missing_since TEXT;
ALTER TABLE lms_items ADD COLUMN last_seen_at TEXT;
ALTER TABLE lms_items ADD COLUMN missing_since TEXT;
ALTER TABLE registrations ADD COLUMN last_seen_at TEXT;
ALTER TABLE registrations ADD COLUMN missing_since TEXT;
ALTER TABLE registration_states ADD COLUMN last_seen_at TEXT;
ALTER TABLE registration_states ADD COLUMN missing_since TEXT;
ALTER TABLE grades ADD COLUMN last_seen_at TEXT;
ALTER TABLE grades ADD COLUMN missing_since TEXT;
ALTER TABLE requirement_progress ADD COLUMN last_seen_at TEXT;
ALTER TABLE requirement_progress ADD COLUMN missing_since TEXT;
ALTER TABLE gpa_summary ADD COLUMN last_seen_at TEXT;
ALTER TABLE gpa_summary ADD COLUMN missing_since TEXT;
ALTER TABLE gpa_by_term ADD COLUMN last_seen_at TEXT;
ALTER TABLE gpa_by_term ADD COLUMN missing_since TEXT;
ALTER TABLE academic_calendar ADD COLUMN last_seen_at TEXT;
ALTER TABLE academic_calendar ADD COLUMN missing_since TEXT;
ALTER TABLE departments ADD COLUMN last_seen_at TEXT;
ALTER TABLE departments ADD COLUMN missing_since TEXT;
ALTER TABLE recommended_courses ADD COLUMN last_seen_at TEXT;
ALTER TABLE recommended_courses ADD COLUMN missing_since TEXT;

-- Early kyomu projections used the syllabus offering as courseId and omitted
-- the UI membership discriminator/term. Relink them to the migrated course.
UPDATE memberships
SET value = json_set(
  value,
  '$.kind', 'membership',
  '$.courseId', COALESCE(
    (SELECT id FROM timetable_courses c WHERE json_extract(c.value,'$.offeringId')=json_extract(memberships.value,'$.courseId') LIMIT 1),
    (SELECT id FROM manual_courses c WHERE json_extract(c.value,'$.offeringId')=json_extract(memberships.value,'$.courseId') LIMIT 1)
  ),
  '$.termId', COALESCE(
    (SELECT json_extract(c.value,'$.termId') FROM timetable_courses c WHERE json_extract(c.value,'$.offeringId')=json_extract(memberships.value,'$.courseId') LIMIT 1),
    (SELECT json_extract(c.value,'$.termId') FROM manual_courses c WHERE json_extract(c.value,'$.offeringId')=json_extract(memberships.value,'$.courseId') LIMIT 1)
  )
)
WHERE source='kyomu' AND json_type(value,'$.kind') IS NULL;

ALTER TABLE course_overrides ADD COLUMN source_key TEXT;
UPDATE course_overrides SET source_key = COALESCE(json_extract(value, '$.sourceKey'), course_id)
WHERE source_key IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS course_overrides_source_key ON course_overrides(source_key);

ALTER TABLE assignment_marks ADD COLUMN source_key TEXT;
UPDATE assignment_marks SET source_key = COALESCE(json_extract(value, '$.sourceKey'), assignment_id)
WHERE source_key IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS assignment_marks_source_key ON assignment_marks(source_key);

UPDATE timetable_courses SET last_seen_at = updated_at WHERE source IN ('lms','kyomu','school');
UPDATE memberships SET last_seen_at = updated_at WHERE source IN ('lms','kyomu','school');
UPDATE assignments SET last_seen_at = updated_at WHERE source IN ('lms','kyomu','school');
UPDATE offerings SET last_seen_at = updated_at WHERE source IN ('syllabus','school');
UPDATE lms_courses SET last_seen_at = updated_at;
UPDATE lms_items SET last_seen_at = updated_at;
UPDATE registrations SET last_seen_at = updated_at;
UPDATE registration_states SET last_seen_at = updated_at;
UPDATE grades SET last_seen_at = updated_at;
UPDATE requirement_progress SET last_seen_at = updated_at;
UPDATE gpa_summary SET last_seen_at = updated_at;
UPDATE gpa_by_term SET last_seen_at = updated_at;
UPDATE academic_calendar SET last_seen_at = updated_at;
UPDATE departments SET last_seen_at = updated_at;
UPDATE recommended_courses SET last_seen_at = updated_at;

INSERT OR REPLACE INTO settings(key,value,hash,updated_at,updated_by,rev,source,deleted_at)
SELECT 'userPreferences', json_object(
  'departmentId', json_extract(value,'$.departmentId'),
  'notifications', json_object(
    'classes', CASE json_extract(value,'$.notifications.classes')
      WHEN 1 THEN json('true') ELSE json('false') END,
    'assignments', CASE json_extract(value,'$.notifications.assignments')
      WHEN 1 THEN json('true') ELSE json('false') END
  ),
  'customization', json_extract(value,'$.customization')
), 'migrated-v4', updated_at, updated_by, rev, source, NULL
FROM settings WHERE key='settings';

INSERT OR REPLACE INTO settings(key,value,hash,updated_at,updated_by,rev,source,deleted_at)
SELECT 'devicePreferences', json_object(
  'lmsBilingualFilter', CASE
    WHEN json_type(value,'$.lmsBilingualFilter') IS NULL THEN NULL
    WHEN json_extract(value,'$.lmsBilingualFilter') = 1 THEN json('true')
    ELSE json('false') END,
  'autoDownloadLimitMb', json_extract(value,'$.autoDownloadLimitMb'),
  'autoCleanDays', json_extract(value,'$.autoCleanDays'),
  'agentAccess', json_extract(value,'$.agentAccess'),
  'appearance', json_extract(value,'$.appearance'),
  'density', json_extract(value,'$.density'),
  'textSize', json_extract(value,'$.textSize'),
  'locale', json_extract(value,'$.locale'),
  'notifications', json_object('localPermission', json_extract(value,'$.notifications.localPermission')),
  'restoreSchoolPage', json('false'),
  'schoolServiceWidth', 168,
  'schoolWebZoom', 1,
  'showGrades', json('false'),
  'siteConsents', json_object(
    'portal', json('false'),
    'lms', json('false'),
    'syllabus', json('false')
  )
), 'migrated-v4', updated_at, updated_by, rev, source, NULL
FROM settings WHERE key='settings';

INSERT OR REPLACE INTO settings(key,value,hash,updated_at,updated_by,rev,source,deleted_at)
SELECT 'cloud', json_extract(value,'$.cloud'), 'migrated-v4', updated_at, updated_by, rev, source, NULL
FROM settings WHERE key='settings';

DELETE FROM settings WHERE key='settings';

PRAGMA user_version = 4;
COMMIT;
