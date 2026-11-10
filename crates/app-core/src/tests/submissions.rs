use super::*;
use crate::{
    data::{DataError, DataStore},
    downloads::Downloader,
    notes::NoteStore,
    plan::{Network, PlanEngine},
    school_auth::SchoolAuthService,
};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicI64, Ordering},
        Mutex, MutexGuard,
    },
};

const ASSIGNMENT: &str = "lms-assignment:77";

struct EmptyVault;

impl crate::ports::SecretVault for EmptyVault {
    fn store(&self, _: &str, _: &str) -> Result<(), crate::ports::SecretError> {
        Ok(())
    }
    fn load(&self, _: &str) -> Result<Option<String>, crate::ports::SecretError> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<(), crate::ports::SecretError> {
        Ok(())
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex_digest(bytes)
}

/// The LMS behind the port: one assignment and its submission, applied the
/// way Moodle applies writes (a save replaces the whole file area and moves
/// `time_modified`; a removal empties it and leaves the time), with every
/// call counted. No network: the transport is the only thing replaced, and
/// the service, ledger, snapshots, `PlanEngine` and workspaces under test
/// are the real ones.
#[derive(Clone)]
struct FakeLms(Arc<Lms>);

struct Lms {
    state: Mutex<LmsState>,
    /// With `hold_saves`, a save says it started and waits for release.
    save_started: tokio::sync::Notify,
    release_save: tokio::sync::Notify,
}

struct LmsState {
    identity: LmsIdentity,
    /// The account generation a permit is taken at.
    generation: u64,
    remote: RemoteState,
    rules: SubmissionRules,
    clock: i64,
    drafts: HashMap<u64, Vec<(String, Vec<u8>)>>,
    /// The bytes the LMS holds for the submission's files.
    submitted: Vec<(String, Vec<u8>)>,
    /// Whether a read asked for the files' content can download them.
    content_readable: bool,
    uploaded_from: Vec<PathBuf>,
    reads: usize,
    uploads: usize,
    saves: usize,
    removes: usize,
    save: SaveOutcome,
    hold_saves: bool,
    /// Turns the person's LMS switch off while the next upload runs.
    switch_off_on_upload: Option<DataStore>,
}

#[derive(Clone, Copy, PartialEq)]
enum SaveOutcome {
    Answered,
    /// The LMS applies the save and the answer is lost.
    LostAfterApplying,
    /// The request times out before the LMS applies anything.
    LostBeforeApplying,
}

impl SubmissionTransport for FakeLms {
    fn permit(&self) -> Result<WritePermit, TransportError> {
        Ok(WritePermit {
            generation: self.0.state.lock().unwrap().generation,
            inner: None,
        })
    }

    fn read(
        self: Arc<Self>,
        _: Arc<WritePermit>,
        module: LmsModule,
        content: bool,
    ) -> BoxFuture<'static, Result<RemoteSubmission, TransportError>> {
        Box::pin(async move {
            assert_eq!(module.cmid, 77);
            let mut lms = self.0.state.lock().unwrap();
            lms.reads += 1;
            let content = (content && lms.content_readable).then(|| {
                lms.submitted
                    .iter()
                    .map(|(name, bytes)| RemoteContent {
                        name: name.clone(),
                        sha256: sha256(bytes),
                    })
                    .collect()
            });
            Ok(RemoteSubmission {
                identity: lms.identity.clone(),
                assign_id: 501,
                cmid: 77,
                state: lms.remote.clone(),
                rules: lms.rules.clone(),
                content,
            })
        })
    }

    fn upload<'a>(
        &'a self,
        _: &'a WriteGrant,
        _: &'a WritePermit,
        files: &'a [UploadFile],
    ) -> BoxFuture<'a, Result<u64, TransportError>> {
        Box::pin(async move {
            let contents = files
                .iter()
                .map(|file| (file.name.clone(), std::fs::read(&file.path).unwrap()))
                .collect::<Vec<_>>();
            let switch_off = self.0.state.lock().unwrap().switch_off_on_upload.take();
            if let Some(store) = switch_off {
                lms_writes(&store, false);
            }
            let mut lms = self.0.state.lock().unwrap();
            lms.uploads += 1;
            lms.uploaded_from
                .extend(files.iter().map(|file| file.path.clone()));
            let draft = 9000 + lms.uploads as u64;
            lms.drafts.insert(draft, contents);
            Ok(draft)
        })
    }

    fn save<'a>(
        &'a self,
        _: &'a WriteGrant,
        _: &'a WritePermit,
        assign_id: u64,
        draft: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>> {
        Box::pin(async move {
            assert_eq!(assign_id, 501);
            let hold = self.0.state.lock().unwrap().hold_saves;
            if hold {
                self.0.save_started.notify_one();
                self.0.release_save.notified().await;
            }
            let mut lms = self.0.state.lock().unwrap();
            lms.saves += 1;
            let outcome = lms.save;
            if outcome != SaveOutcome::LostBeforeApplying {
                let files = lms.drafts.get(&draft).cloned().unwrap_or_default();
                lms.submit(files);
            }
            match outcome {
                SaveOutcome::Answered => Ok(()),
                _ => Err(TransportError::Uncertain("timed out".into())),
            }
        })
    }

    fn remove<'a>(
        &'a self,
        _: &'a WriteGrant,
        _: &'a WritePermit,
        assign_id: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>> {
        Box::pin(async move {
            assert_eq!(assign_id, 501);
            let mut lms = self.0.state.lock().unwrap();
            lms.removes += 1;
            lms.remote.status = RemoteStatus::New;
            lms.remote.files.clear();
            lms.submitted.clear();
            Ok(())
        })
    }
}

impl LmsState {
    /// The submission as a save (Core's or the person's in a browser)
    /// leaves it.
    fn submit(&mut self, files: Vec<(String, Vec<u8>)>) {
        self.clock += 1;
        self.remote.time_modified = self.clock;
        self.remote.status = RemoteStatus::Submitted;
        self.remote.files = files
            .iter()
            .map(|(name, bytes)| RemoteFile {
                name: name.clone(),
                bytes: bytes.len() as u64,
            })
            .collect();
        self.submitted = files;
    }
}

/// Agent access on, and "Allow AI agents to write to LMS" as `allowed`.
fn lms_writes(store: &DataStore, allowed: bool) {
    store
        .put_workspace_setting(
            "devicePreferences",
            json!({"agentAccess": {"enabled": true, "lmsWrites": allowed}}),
        )
        .unwrap();
}

struct Fixture {
    root: PathBuf,
    store: DataStore,
    workspaces: AssignmentWorkspaces,
    lms: FakeLms,
    /// The engine every read goes through, without school sessions: the
    /// fake transport decides what it reaches.
    plan: PlanEngine,
    tasks: RuntimeTasks,
    now: Arc<AtomicI64>,
}

impl Fixture {
    /// One assignment of a course on the timetable, its LMS submission
    /// empty and open: an individual file assignment with drafts off.
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("isct-submissions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.join("data")).unwrap().0;
        let course = json!({
            "id":"2026-c1", "kind":"course", "termId":"2026-3Q", "code":"CSC.T251",
            "title":"Algorithms", "instructor":"", "credits":2,
            "creditSplit":{"lecture":2,"exercise":0,"lab":0}, "instructionLanguage":"en",
            "color":"#526e91", "meetings":[], "source":{"provider":"kyomu"}
        });
        let assignment = json!({
            "id":ASSIGNMENT, "kind":"assignment", "courseId":"2026-c1", "lmsCmid":77,
            "lmsCourseId":5, "title":"Report 3", "details":"", "source":{"provider":"lms"}
        });
        store
            .write(None, |transaction| {
                for (table, id, value) in [
                    ("timetable_courses", "2026-c1", &course),
                    ("assignments", ASSIGNMENT, &assignment),
                ] {
                    transaction
                        .execute(
                            &format!(
                                "INSERT INTO {table} (id,value,hash,updated_at,updated_by,rev)
                                 VALUES (?1,?2,'seed','2026-10-01T00:00:00Z','seed',1)"
                            ),
                            rusqlite::params![id, value.to_string()],
                        )
                        .map_err(DataError::from)?;
                }
                Ok::<_, DataError>(())
            })
            .unwrap();
        lms_writes(&store, true);
        let notes = NoteStore::default();
        notes.at(root.join("files"));
        let workspaces = AssignmentWorkspaces::new(store.clone(), notes, Downloader::default());
        let lms = FakeLms(Arc::new(Lms {
            state: Mutex::new(LmsState {
                identity: LmsIdentity {
                    site: "https://lms.s.isct.ac.jp/2025".into(),
                    user_id: 9001,
                },
                generation: 1,
                remote: RemoteState {
                    attempt: 0,
                    status: RemoteStatus::New,
                    files: Vec::new(),
                    time_modified: 0,
                },
                rules: SubmissionRules {
                    can_edit: true,
                    locked: false,
                    drafts: false,
                    statement: false,
                    team: false,
                    time_limit: false,
                    files_enabled: true,
                    online_text: false,
                    max_files: 20,
                    max_file_bytes: 250 * 1024 * 1024,
                    file_types: Vec::new(),
                    cutoff: None,
                },
                clock: 1_000,
                drafts: HashMap::new(),
                submitted: Vec::new(),
                content_readable: true,
                uploaded_from: Vec::new(),
                reads: 0,
                uploads: 0,
                saves: 0,
                removes: 0,
                save: SaveOutcome::Answered,
                hold_saves: false,
                switch_off_on_upload: None,
            }),
            save_started: tokio::sync::Notify::new(),
            release_save: tokio::sync::Notify::new(),
        }));
        let plan = PlanEngine::new(
            store.clone(),
            Arc::new(SchoolAuthService::new(EmptyVault)),
            Network::Wifi,
        );
        Self {
            root,
            store,
            workspaces,
            lms,
            plan,
            tasks: RuntimeTasks::default(),
            now: Arc::new(AtomicI64::new(1_790_000_000)),
        }
    }

    /// A service over the fixture's data folder: a second one is the app
    /// after a restart.
    fn service(&self) -> SubmissionService {
        self.open().unwrap()
    }

    fn open(&self) -> Result<SubmissionService, SubmissionError> {
        let now = self.now.clone();
        SubmissionService::with_clock(
            &self.store,
            self.workspaces.clone(),
            Arc::new(self.lms.clone()),
            self.plan.hints(),
            self.tasks.clone(),
            Arc::new(move || now.load(Ordering::SeqCst)),
        )
    }

    /// The active ledger generation's folder.
    fn generation(&self) -> PathBuf {
        let active = self
            .store
            .root()
            .active(StoreKind::Submissions)
            .unwrap()
            .unwrap();
        self.store
            .paths()
            .generation(StoreKind::Submissions, &active.id)
    }

    fn object(&self, sha256: &str) -> PathBuf {
        self.generation().join("objects").join(sha256)
    }

    fn lms(&self) -> MutexGuard<'_, LmsState> {
        self.lms.0.state.lock().unwrap()
    }

    /// Uploads, saves and removals the LMS received.
    fn calls(&self) -> (usize, usize, usize) {
        let lms = self.lms();
        (lms.uploads, lms.saves, lms.removes)
    }

    /// Reads of the LMS so far.
    fn reads(&self) -> usize {
        self.lms().reads
    }

    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }

    fn advance(&self, seconds: i64) {
        self.now.fetch_add(seconds, Ordering::SeqCst);
    }

    /// Writes `name` into `submit/`, creating the workspace on first use.
    fn put(&self, name: &str, contents: &[u8]) {
        if let Some(workspace) = self.workspaces.find(ASSIGNMENT).unwrap() {
            std::fs::write(
                PathBuf::from(workspace.path).join("submit").join(name),
                contents,
            )
            .unwrap();
            return;
        }
        let inbox = self.root.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let source = inbox.join(name);
        std::fs::write(&source, contents).unwrap();
        self.workspaces
            .add_submit_files(ASSIGNMENT, vec![(source, name.into())])
            .unwrap();
    }

    /// What the client sends back after showing the summary.
    fn confirm(&self, preparation: &Preparation) -> ExecuteRequest {
        ExecuteRequest {
            preparation_id: preparation.id.clone(),
            summary: preparation.summary(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}


// The snapshot is what the person confirmed. Once `submit/` changes, the
// confirmed files are no longer what they want handed in, and a damaged
// snapshot is no longer what they confirmed: nothing is sent until they
// prepare again. A confirmation that does not echo the summary is refused
// too. What is uploaded comes from the snapshot objects, never `submit/`.
#[tokio::test]
async fn files_changed_after_prepare_are_never_sent() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 first");
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert_eq!(prepared.effect.added, ["report.pdf"]);
    assert_eq!(prepared.files[0].bytes, 12);
    assert_eq!(
        prepared.summary().files[0].sha256_prefix.len(),
        DIGEST_PREFIX
    );

    fixture.put("report.pdf", b"%PDF-1 second");
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::FilesChanged)
    ));
    let mut echo = prepared.summary();
    echo.files[0].sha256_prefix = "000000000000".into();
    assert!(matches!(
        service
            .execute(ExecuteRequest {
                preparation_id: prepared.id.clone(),
                summary: echo,
            })
            .await,
        Err(SubmissionError::ConfirmationMismatch("files"))
    ));

    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    std::fs::write(fixture.object(&prepared.files[0].sha256), b"tampered").unwrap();
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::FilesChanged)
    ));
    assert_eq!(fixture.calls(), (0, 0, 0));

    // Preparing again replaces the damaged object, whole, under the same
    // preparation.
    let again = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert_eq!(again.id, prepared.id);
    let done = service.execute(fixture.confirm(&again)).await.unwrap();
    assert_eq!(done.state, OperationState::Succeeded);
    let lms = fixture.lms();
    assert_eq!(
        lms.submitted,
        [("report.pdf".to_string(), b"%PDF-1 second".to_vec())]
    );
    assert!(lms
        .uploaded_from
        .iter()
        .all(|path| path.starts_with(fixture.generation().join("objects"))));
}

// A snapshot is published with the preparation that names it, and
// collected only in maintenance once nothing names it: a preparation keeps
// its objects until it expires, and a claimed write keeps them past its
// preparation's expiry. Staging another run left goes too.
#[tokio::test]
async fn maintenance_collects_only_what_nothing_names() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    let object = fixture.object(&prepared.files[0].sha256);
    let orphan = fixture.generation().join("staging/earlier-run/capture");
    std::fs::create_dir_all(&orphan).unwrap();
    service.maintain(fixture.now()).unwrap();
    assert!(object.is_file(), "the preparation names it");
    assert!(!orphan.exists());

    let now = fixture.now();
    assert!(matches!(
        service
            .ledger
            .claim(&prepared.id, now, "earlier-run", |_| Ok(()), |preparation| {
                Operation::new(preparation, now)
            })
            .unwrap(),
        Claim::Claimed(..)
    ));
    fixture.advance(PREPARATION_TTL + 1);
    service.maintain(fixture.now()).unwrap();
    assert!(service.ledger.preparation(&prepared.id).unwrap().is_none());
    assert!(object.is_file(), "the claimed write names it");

    fixture.put("notes.txt", b"other");
    let other = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap_err();
    assert!(matches!(other, SubmissionError::InProgress));
}

// The answer to a save is lost after the LMS applied it. Core reads the
// LMS at once, downloads what it holds and finds its own content there:
// succeeded, sent once, with the assignment queued to read again past any
// refresh from before the write. Asking again, with the same preparation or
// a fresh one of the same content, answers with that write instead of
// sending another.
#[tokio::test]
async fn a_lost_answer_after_the_lms_applied_settles_by_reading() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    fixture.lms().save = SaveOutcome::LostAfterApplying;
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    let operation = service.execute(fixture.confirm(&prepared)).await.unwrap();
    assert_eq!(operation.state, OperationState::Succeeded);
    assert!(operation.sent);
    let refreshes = service.ledger.refreshes_due(fixture.now()).unwrap();
    assert_eq!(refreshes.len(), 1);
    assert_eq!(refreshes[0].lms_course_id, 5);
    assert_eq!(refreshes[0].revision, 1, "the write raised the resource");

    let repeated = service.execute(fixture.confirm(&prepared)).await.unwrap();
    assert_eq!(repeated.id, operation.id);
    let fresh = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert!(fresh.effect.unchanged);
    assert_eq!(
        service.execute(fixture.confirm(&fresh)).await.unwrap().id,
        operation.id
    );
    assert_eq!(fixture.calls(), (1, 1, 0));
}

// Names and sizes are not content. A lost answer after which the LMS holds
// files of the right names and sizes stays unknown while their content
// cannot be read, and fails once it can be and is not what was sent.
#[tokio::test]
async fn content_decides_whether_a_write_applied() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    {
        let mut lms = fixture.lms();
        lms.save = SaveOutcome::LostBeforeApplying;
        lms.content_readable = false;
    }
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    let operation = service.execute(fixture.confirm(&prepared)).await.unwrap();
    assert_eq!(operation.state, OperationState::OutcomeUnknown);
    // Someone else's file of the same name and size lands meanwhile.
    fixture
        .lms()
        .submit(vec![("report.pdf".into(), b"%PDF-1 REPORT".to_vec())]);
    fixture.advance(CHECK_FIRST);
    service.run_due().await.unwrap();
    assert_eq!(
        service.status(&operation.id).unwrap().state,
        OperationState::OutcomeUnknown
    );

    fixture.lms().content_readable = true;
    fixture.advance(CHECK_FIRST * 3);
    service.run_due().await.unwrap();
    let settled = service.status(&operation.id).unwrap();
    assert_eq!(settled.state, OperationState::Failed);
    assert_eq!(settled.failure, Some(ErrorCode::SubmissionMismatch));
    assert_eq!(fixture.calls(), (1, 1, 0));
}

// A save that times out before the LMS applied it is never failed by the
// clock: however long the LMS has had, it stays unknown and refuses other
// writes until a read shows what the LMS did. Asking for its state answers
// from the ledger without reading the LMS, and checks come less often the
// longer it stays unknown. A save made in a browser meanwhile is evidence:
// the LMS holds something else, the write failed and the assignment is
// free again. Nothing sends it again by itself.
#[tokio::test]
async fn a_lost_save_stays_unknown_until_a_read_shows_what_happened() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    fixture.lms().save = SaveOutcome::LostBeforeApplying;
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    let operation = service.execute(fixture.confirm(&prepared)).await.unwrap();
    assert_eq!(operation.state, OperationState::OutcomeUnknown);
    assert_eq!(
        service
            .execute(fixture.confirm(&prepared))
            .await
            .unwrap()
            .id,
        operation.id
    );
    assert!(matches!(
        service.prepare(ASSIGNMENT, SubmissionAction::Save).await,
        Err(SubmissionError::OutcomeUnknown)
    ));

    let reads = fixture.reads();
    assert_eq!(
        service.status(&operation.id).unwrap().state,
        OperationState::OutcomeUnknown
    );
    assert_eq!(fixture.reads(), reads, "status never reads the LMS");
    let first = service.next_due().unwrap().timestamp();
    fixture.advance(CHECK_MAX * 4);
    service.run_due().await.unwrap();
    assert_eq!(
        service.status(&operation.id).unwrap().state,
        OperationState::OutcomeUnknown,
        "time alone proves nothing"
    );
    let later = service.next_due().unwrap().timestamp() - fixture.now();
    assert!(later >= CHECK_FIRST, "{first} then {later}");
    fixture.advance(later);
    service.run_due().await.unwrap();
    assert!(service.next_due().unwrap().timestamp() - fixture.now() > later);

    fixture
        .lms()
        .submit(vec![("other.pdf".into(), b"from the browser".to_vec())]);
    fixture.advance(CHECK_MAX);
    service.run_due().await.unwrap();
    let settled = service.status(&operation.id).unwrap();
    assert_eq!(settled.state, OperationState::Failed);
    assert_eq!(settled.failure, Some(ErrorCode::SubmissionMismatch));
    assert_eq!(fixture.calls(), (1, 1, 0));
    assert!(matches!(
        service.prepare(ASSIGNMENT, SubmissionAction::Save).await,
        Err(SubmissionError::RemoteNotEmpty { .. })
    ));
}

// The same content prepared twice is one preparation, and executed twice is
// one write. Core's receipt then says the LMS holds it, until the LMS
// changes behind Core's back: a save would then replace a submission Core
// did not make, so it is refused with what it would replace. A removal is
// prepared the same way and says it cannot be undone.
#[tokio::test]
async fn the_same_content_prepared_and_executed_twice_is_written_once() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    fixture.put("data.csv", b"a,b\n1,2\n");
    let service = fixture.service();
    let first = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    let second = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.effect.added, ["data.csv", "report.pdf"]);
    assert_eq!(first.reversibility, Reversibility::WhileOpen);
    let operation = service.execute(fixture.confirm(&first)).await.unwrap();
    assert_eq!(operation.state, OperationState::Succeeded);
    assert_eq!(
        service.execute(fixture.confirm(&second)).await.unwrap().id,
        operation.id
    );
    assert_eq!(fixture.calls(), (1, 1, 0));

    {
        let mut lms = fixture.lms();
        lms.clock += 1;
        lms.remote.time_modified = lms.clock;
        lms.remote.files = vec![RemoteFile {
            name: "report.pdf".into(),
            bytes: 99,
        }];
    }
    match service.prepare(ASSIGNMENT, SubmissionAction::Save).await {
        Err(SubmissionError::RemoteNotEmpty {
            added,
            replaced,
            removed,
        }) => {
            assert_eq!(added, ["data.csv"]);
            assert_eq!(replaced, ["report.pdf"]);
            assert!(removed.is_empty());
        }
        other => panic!("{other:?}"),
    }

    let removal = service
        .prepare(ASSIGNMENT, SubmissionAction::Remove)
        .await
        .unwrap();
    assert_eq!(removal.effect.removed, ["report.pdf"]);
    assert_eq!(removal.reversibility, Reversibility::Irreversible);
    let removed = service.execute(fixture.confirm(&removal)).await.unwrap();
    assert_eq!(removed.state, OperationState::Succeeded);
    assert!(fixture.lms().remote.files.is_empty());
    assert_eq!(fixture.calls(), (1, 1, 1));
}

// Between prepare and execute the submission changes on the LMS page, or a
// new attempt opens, or the account is another one or signed in anew: the
// write was confirmed against what is no longer there, so it is refused
// before anything is sent, and the refusal leaves nothing that blocks
// preparing again. A preparation also expires.
#[tokio::test]
async fn a_remote_change_after_prepare_is_a_conflict() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    fixture.lms().generation = 2;
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::IdentityChanged)
    ));
    fixture.lms().generation = 1;
    fixture.lms().identity.user_id = 9002;
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::IdentityChanged)
    ));
    fixture.lms().identity.user_id = 9001;
    {
        let mut lms = fixture.lms();
        lms.clock += 1;
        lms.remote = RemoteState {
            attempt: 0,
            status: RemoteStatus::Submitted,
            files: vec![RemoteFile {
                name: "draft.pdf".into(),
                bytes: 5,
            }],
            time_modified: lms.clock,
        };
    }
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::RemoteConflict)
    ));
    fixture.lms().remote = RemoteState {
        attempt: 1,
        status: RemoteStatus::Reopened,
        files: Vec::new(),
        time_modified: 0,
    };
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::RemoteConflict)
    ));
    assert_eq!(fixture.calls(), (0, 0, 0));
    let reprepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert_eq!(reprepared.baseline.attempt, 1);

    fixture.advance(PREPARATION_TTL);
    assert!(matches!(
        service.execute(fixture.confirm(&reprepared)).await,
        Err(SubmissionError::Expired)
    ));
    assert_eq!(fixture.calls(), (0, 0, 0));
}

// While one write is on its way, a write of other content to the same
// assignment is refused. Cancelling the one on its way does not claim to
// undo it: its save was sent, and it runs to its end, owned by Core past
// the caller that stopped waiting.
#[tokio::test]
async fn one_write_per_assignment_at_a_time() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 second");
    let service = fixture.service();
    let second = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    fixture.put("report.pdf", b"%PDF-1 first");
    let first = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);

    fixture.lms().hold_saves = true;
    let running = tokio::spawn({
        let service = service.clone();
        let request = fixture.confirm(&first);
        async move { service.execute(request).await }
    });
    fixture.lms.0.save_started.notified().await;
    let pending = service.ledger.latest_for(&first.id).unwrap().unwrap();
    assert_eq!(pending.state, OperationState::Saving);
    // The caller stops waiting; the write does not stop.
    running.abort();

    fixture.put("report.pdf", b"%PDF-1 second");
    assert!(matches!(
        service.execute(fixture.confirm(&second)).await,
        Err(SubmissionError::InProgress)
    ));
    let cancel = service.cancel(&pending.id).unwrap();
    assert!(!cancel.cancelled);
    assert!(cancel.operation.sent);
    assert_eq!(cancel.operation.state, OperationState::Saving);

    fixture.lms.0.release_save.notify_one();
    fixture.tasks.close();
    fixture.tasks.wait().await;
    assert_eq!(
        service.status(&pending.id).unwrap().state,
        OperationState::Succeeded
    );
    assert_eq!(fixture.calls(), (1, 1, 0));
}

// The app stops mid-write. On the next start, writes another run owned
// lost their executor (one Core holds the folder): one that never sent its
// save failed; one that may have is read until the LMS shows what it did,
// and is never sent again.
#[tokio::test]
async fn restart_settles_interrupted_writes_by_reading_only() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    // Stored as an earlier run leaves it: claimed, then moved to `to` with
    // the save sent when `sent`.
    let interrupted = |to: OperationState, sent: bool| {
        let now = fixture.now();
        // A write that succeeded released its preparation; publish it again.
        service.ledger.publish_preparation(&prepared, now).unwrap();
        let Claim::Claimed(_, operation) = service
            .ledger
            .claim(&prepared.id, now, "earlier-run", |_| Ok(()), |preparation| {
                Operation::new(preparation, now)
            })
            .unwrap()
        else {
            panic!("claimed");
        };
        let mut operation = *operation;
        operation.state = to;
        operation.sent = sent;
        operation.sent_at = sent.then_some(now);
        assert!(service
            .ledger
            .transition(&operation, OperationState::Prepared, "earlier-run", None, now)
            .unwrap());
        operation
    };

    // Stopped after the save left and the LMS applied it.
    let applied = interrupted(OperationState::Saving, true);
    fixture
        .lms()
        .submit(vec![("report.pdf".into(), b"%PDF-1 report".to_vec())]);
    let restarted = fixture.service();
    assert_eq!(
        restarted.status(&applied.id).unwrap().state,
        OperationState::OutcomeUnknown
    );
    fixture.advance(CHECK_FIRST);
    restarted.run_due().await.unwrap();
    assert_eq!(
        restarted.status(&applied.id).unwrap().state,
        OperationState::Succeeded
    );

    // Stopped while uploading: the save never left.
    fixture.lms().remote = prepared.baseline.clone();
    fixture.lms().submitted.clear();
    let uploading = interrupted(OperationState::Uploading, false);
    let restarted = fixture.service();
    let stopped = restarted.status(&uploading.id).unwrap();
    assert_eq!(stopped.state, OperationState::Failed);
    assert_eq!(stopped.failure, Some(ErrorCode::Stopped));
    assert!(!stopped.sent);

    // Stopped after the save left, which the LMS never applied: unknown
    // for as long as the LMS shows nothing.
    let lost = interrupted(OperationState::Saving, true);
    let restarted = fixture.service();
    fixture.advance(CHECK_MAX * 4);
    restarted.run_due().await.unwrap();
    assert_eq!(
        restarted.status(&lost.id).unwrap().state,
        OperationState::OutcomeUnknown
    );
    assert_eq!(fixture.calls(), (0, 0, 0));
}

// The ledger has its own store generation. One of an older format starts
// empty in a new generation (hand-ins are confirmed before upgrading), and
// nothing of it is read; one of a newer format is refused and kept as it
// is, with LMS writes off.
#[tokio::test]
async fn an_older_ledger_is_replaced_and_a_newer_one_refused() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let first = fixture.generation();
    drop(service);
    let root = fixture.store.root();
    let republish = |format: i64| {
        let (id, directory) = root.new_generation(StoreKind::Submissions).unwrap();
        std::fs::write(directory.join("marker"), b"kept").unwrap();
        root.publish(
            &[crate::data::root::Candidate {
                kind: StoreKind::Submissions,
                generation: crate::data::root::Generation { id, format },
            }],
            false,
        )
        .unwrap();
        directory
    };

    let older = republish(ledger::FORMAT - 1);
    let service = fixture.service();
    assert_ne!(fixture.generation(), older);
    assert_ne!(fixture.generation(), first);
    fixture.put("report.pdf", b"%PDF-1 report");
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    assert!(service.snapshots.intact(&prepared.files));
    drop(service);

    let newer = republish(ledger::FORMAT + 1);
    assert!(matches!(
        fixture.open(),
        Err(SubmissionError::UnsupportedLedger { .. })
    ));
    assert_eq!(fixture.generation(), newer);
    assert_eq!(std::fs::read(newer.join("marker")).unwrap(), b"kept");
}

// Before generations the ledger lay flat in the data folder. One of a
// format no release wrote is refused rather than replaced by an empty one.
#[tokio::test]
async fn a_flat_ledger_of_an_unknown_format_is_refused() {
    let fixture = Fixture::new();
    let flat = fixture.store.paths().root().join(ledger::FILE);
    rusqlite::Connection::open(&flat)
        .unwrap()
        .execute_batch("PRAGMA user_version=9; CREATE TABLE kept(id TEXT);")
        .unwrap();
    assert!(matches!(
        fixture.open(),
        Err(SubmissionError::UnsupportedLedger { found: 9, .. })
    ));
    assert!(fixture
        .store
        .root()
        .active(StoreKind::Submissions)
        .unwrap()
        .is_none());
    rusqlite::Connection::open(&flat)
        .unwrap()
        .execute_batch("PRAGMA user_version=2;")
        .unwrap();
    fixture.service();
    assert!(fixture.generation().join(ledger::FILE).is_file());
}

// The person's switch gates the write path itself: off, nothing is sent;
// turned off during the upload, the save never leaves, and the write is
// recorded as failed without having been sent.
#[tokio::test]
async fn lms_writes_need_the_persons_switch() {
    let fixture = Fixture::new();
    fixture.put("report.pdf", b"%PDF-1 report");
    let service = fixture.service();
    let prepared = service
        .prepare(ASSIGNMENT, SubmissionAction::Save)
        .await
        .unwrap();
    lms_writes(&fixture.store, false);
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::WritesDisabled)
    ));
    assert_eq!(fixture.calls(), (0, 0, 0));

    lms_writes(&fixture.store, true);
    fixture.lms().switch_off_on_upload = Some(fixture.store.clone());
    assert!(matches!(
        service.execute(fixture.confirm(&prepared)).await,
        Err(SubmissionError::WritesDisabled)
    ));
    let stopped = service.ledger.latest_for(&prepared.id).unwrap().unwrap();
    assert_eq!(stopped.state, OperationState::Failed);
    assert_eq!(stopped.failure, Some(ErrorCode::LmsWritesDisabled));
    assert!(!stopped.sent);
    assert_eq!(fixture.calls(), (1, 0, 0));

    lms_writes(&fixture.store, true);
    let done = service.execute(fixture.confirm(&prepared)).await.unwrap();
    assert_eq!(done.state, OperationState::Succeeded);
    assert_eq!(fixture.calls(), (2, 1, 0));
}
