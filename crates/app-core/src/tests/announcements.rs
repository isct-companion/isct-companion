use super::*;
use crate::{
    data::DataStore,
    workspace::{WorkspaceIntent, WorkspaceMutation, WorkspaceService},
};
use chrono::Duration;

#[test]
fn answering_preserves_the_banner_and_survives_reopen_until_the_feed_or_time_removes_it() {
    let directory =
        std::env::temp_dir().join(format!("isct-announcements-{}", uuid::Uuid::new_v4()));
    let now = crate::clock::now();
    let until = now + Duration::minutes(5);
    // Exercise the domain with records in storage. Publication schema checks
    // belong to the Isct-Data feed.
    let notice = |id: &str, minutes_ago| Announcement {
        id: id.into(),
        title: Localized {
            en: Some(id.into()),
            ja: None,
        },
        body: Localized {
            en: Some(id.into()),
            ja: None,
        },
        level: AnnouncementLevel::Info,
        starts_at: (now - Duration::minutes(minutes_ago)).fixed_offset(),
        expires_at: until.fixed_offset(),
        platforms: None,
        min_app_version: None,
        max_app_version: None,
        link: None,
    };
    let view_at = |data: &DataStore, at| {
        let fields = crate::workspace_changes::loaded_fields([
            crate::workspace_changes::WorkspaceChange::Announcements,
        ]);
        let (workspace, school, _) = data.view(&fields).unwrap();
        view(
            &school.announcements,
            &workspace,
            &school.answered_announcements,
            at,
        )
    };
    {
        let data = DataStore::initialize_dir(directory.clone()).unwrap().0;
        data.write(None, |transaction| {
            store(transaction, &[notice("older", 2), notice("newer", 1)])
        })
        .unwrap();
        let workspace = WorkspaceService::new(data.clone());
        assert_eq!(view_at(&data, now).pending[0].id, "newer");
        workspace
            .mutate_commit(WorkspaceMutation {
                expected_revision: workspace.revision().unwrap(),
                intent: WorkspaceIntent::AnswerAnnouncement { id: "newer".into() },
            })
            .unwrap();
    }
    let data = DataStore::initialize_dir(directory.clone()).unwrap().0;
    let reopened = view_at(&data, now);
    assert_eq!(
        reopened
            .active
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        ["newer", "older"]
    );
    assert_eq!(reopened.pending[0].id, "older");
    assert!(view_at(&data, until).active.is_empty());
    assert!(view_at(&data, until).pending.is_empty());
    data.write(None, |transaction| store(transaction, &[]))
        .unwrap();
    assert!(view_at(&data, now).active.is_empty());
    assert!(view_at(&data, now).pending.is_empty());
    drop(data);
    let _ = std::fs::remove_dir_all(directory);
}
