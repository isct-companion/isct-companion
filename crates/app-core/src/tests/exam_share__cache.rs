use super::*;

struct TestCache(PathBuf);
impl Drop for TestCache {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

async fn put_copy(owner: &Path, id: &str, downloaded: bool, modified: SystemTime) -> PathBuf {
    let folder = owner.join(id);
    fs::create_dir_all(&folder).unwrap();
    let cached = Cached {
        document: ExamDocument {
            id: id.into(),
            subjects: vec![],
            teachers: vec![],
            academic_year: None,
            occasion: None,
            occasion_number: 0,
            variant: String::new(),
            role: None,
            note: None,
            file_name: format!("{id}.txt"),
            file_type: "text/plain".into(),
        },
        updated_at: 0,
        downloaded,
        file: uuid::Uuid::new_v4(),
    };
    let payload = file_path(&folder, &cached).unwrap();
    let mut file = fs::File::create(&payload).unwrap();
    std::io::Write::write_all(&mut file, b"Local text").unwrap();
    file.set_modified(modified).unwrap();
    save_metadata(&folder, &cached).await.unwrap();
    payload
}

#[tokio::test]
async fn stale_exam_copies_are_unavailable_and_maintenance_keeps_recent_downloads() {
    let root = TestCache(std::env::temp_dir().join(format!("exam-cache-{}", uuid::Uuid::new_v4())));
    let owner = root.0.join("account");
    let now = SystemTime::now();
    let stale = put_copy(
        &owner,
        "stale",
        true,
        now - Duration::from_secs(31 * 86_400),
    )
    .await;
    let current = put_copy(
        &owner,
        "current",
        true,
        now - Duration::from_secs(29 * 86_400),
    )
    .await;
    let preview = put_copy(
        &owner,
        "preview",
        false,
        now - Duration::from_secs(25 * 3_600),
    )
    .await;
    let orphan = owner.join("interrupted").join("partial.txt");
    fs::create_dir_all(orphan.parent().unwrap()).unwrap();
    let file = fs::File::create(&orphan).unwrap();
    file.set_modified(now - Duration::from_secs(25 * 3_600))
        .unwrap();
    drop(file);
    let upload = root.0.join("account.upload.json");
    fs::write(&upload, b"pending publication").unwrap();

    assert!(cache_file(stale.parent().unwrap()).await.unwrap().is_none());
    let available = downloads(&owner).await.unwrap();
    assert_eq!(
        available
            .iter()
            .map(|document| document.id.as_str())
            .collect::<Vec<_>>(),
        ["current"]
    );

    let folder = current.parent().unwrap();
    let cached = cache_file(folder).await.unwrap().unwrap();
    local(folder, cached, true, None).await.unwrap();
    clean_root(&root.0, now + Duration::from_secs(2 * 86_400)).unwrap();

    assert!(
        current.is_file(),
        "opening a surviving download renews its lifetime"
    );
    assert_eq!(
        fs::read(&upload).unwrap(),
        b"pending publication",
        "cleanup does not touch pending uploads"
    );
    assert!(!stale.exists() && !preview.exists() && !orphan.exists());
}
