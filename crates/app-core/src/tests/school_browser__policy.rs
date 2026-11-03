use super::*;

#[test]
fn sign_in_pages_are_recognized_for_identity_providers_and_services() {
    for raw in [
        "https://isct.ex-tic.com/auth/session",
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=userpass_key",
        "https://lms.s.isct.ac.jp/2025/login/index.php",
        "https://lms.s.isct.ac.jp/2025/auth/saml2/login.php",
    ] {
        assert!(is_login_url(&Url::parse(raw).unwrap()), "{raw}");
    }
    for raw in [
        "https://isct.ex-tic.com/idm/user/portal/",
        "https://lms.s.isct.ac.jp/2025/my/",
        "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_resource/content/1/a.pdf",
    ] {
        assert!(!is_login_url(&Url::parse(raw).unwrap()), "{raw}");
    }
    assert!(!is_identity_login(
        &Url::parse("https://lms.s.isct.ac.jp/2025/login/index.php").unwrap()
    ));
}

#[test]
fn automation_reaches_adapter_sites_and_identity_providers_only() {
    for raw in [
        "https://lms.s.isct.ac.jp/2025/my/",
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
        "https://isct.ex-tic.com/auth/session",
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login",
    ] {
        assert!(automation_target(raw).is_some(), "{raw}");
    }
    for raw in [
        // Linked from the portal resource menu, browsed only.
        "https://vlmgr-42.edu2024.gsic.titech.ac.jp/idm/upc/",
        "https://yushima2.tmd.ac.jp/",
        "http://lms.s.isct.ac.jp/2025/my/",
        "https://user:secret@lms.s.isct.ac.jp/2025/",
        "https://example.com/",
    ] {
        assert!(automation_target(raw).is_none(), "{raw}");
    }
    // The adapter sites behind that authority, host by host.
    for (raw, site) in [
        ("https://lms.s.isct.ac.jp/", SchoolSite::ScienceLms),
        ("https://t2schola.titech.ac.jp/", SchoolSite::ScienceLms),
        ("https://syllabus.s.isct.ac.jp/", SchoolSite::Syllabus),
        ("https://isct.ex-tic.com/", SchoolSite::SciencePortal),
        ("https://portal.isct.ac.jp/", SchoolSite::SciencePortal),
        ("https://portal.titech.ac.jp/", SchoolSite::TokyoPortal),
        (
            "https://portal.nap.gsic.titech.ac.jp/",
            SchoolSite::TokyoPortal,
        ),
        ("https://rp.nap.gsic.titech.ac.jp/", SchoolSite::TokyoPortal),
        ("https://mailv4.m.titech.ac.jp/", SchoolSite::TokyoMail),
        ("https://kyomu0.gakumu.titech.ac.jp/", SchoolSite::Kyomu),
        ("https://request.libra.titech.ac.jp/", SchoolSite::Library),
        ("https://portal.t4.gsic.titech.ac.jp/", SchoolSite::Tsubame),
        ("https://www.titech.ac.jp/", SchoolSite::StudyGuide),
    ] {
        assert_eq!(SchoolSite::for_url(raw), Some(site), "{raw}");
        assert!(automation_target(raw).is_some(), "{raw}");
    }
    // A view the person looks at may save anything they can browse, and
    // files the page builds itself; never one with embedded credentials.
    let campus = "https://vlmgr-42.edu2024.gsic.titech.ac.jp/file.pdf";
    assert!(download_permitted(campus));
    assert!(download_permitted(
        "blob:https://kyomu0.gakumu.titech.ac.jp/uuid"
    ));
    assert!(!download_permitted("https://user:secret@example.com/a.pdf"));
}

#[test]
fn only_scriptless_attachments_are_shown_in_place() {
    for mime in [
        "application/pdf",
        "image/png",
        "IMAGE/JPEG",
        "video/mp4",
        "audio/mpeg",
    ] {
        assert!(attachment_shown_in_place(mime), "{mime}");
    }
    // Script would run as the school's origin, or the file reads badly as a
    // page (a notebook as raw JSON): these are saved.
    for mime in [
        "text/html",
        "text/html; charset=utf-8",
        "application/xhtml+xml",
        "image/svg+xml",
        "application/xml",
        "text/xml",
        "application/json",
        "application/x-ipynb+json",
        "text/plain",
        "",
    ] {
        assert!(!attachment_shown_in_place(mime), "{mime}");
    }
}

#[test]
fn restore_records_keep_only_bounded_domain_parameters() {
    assert_eq!(
        restore_record("https://syllabus.s.isct.ac.jp/courses?jwc=202602431&x=1#a").as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
    );
    assert!(restore_record("https://example.com/?id=1").is_none());
}

// Reload reopens the page the person was on: a forum discussion keeps its
// `d`, which the restore record on disk drops, while the fragment and what
// would act or leak (`sesskey`, `confirm`, tokens) go.
#[test]
fn a_reload_keeps_the_page_and_drops_what_acts_or_leaks() {
    let discuss = "https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=77#p5";
    assert_eq!(
        reopen_url(discuss).unwrap().as_str(),
        "https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=77"
    );
    assert_eq!(
        restore_record(discuss).as_deref(),
        Some("https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php")
    );
    assert_eq!(
        reopen_url(
            "https://lms.s.isct.ac.jp/2025/course/view.php?id=3&SessKey=abc&confirm=1&section=2&token=t"
        )
        .unwrap()
        .as_str(),
        "https://lms.s.isct.ac.jp/2025/course/view.php?id=3&section=2"
    );
    assert!(reopen_url("https://example.com/?d=1").is_none());
}

#[test]
fn only_the_lms_assignment_draft_action_survives_in_a_restore_record() {
    let base = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100";
    assert_eq!(
        restore_record(&format!("{base}&action=editsubmission&sesskey=secret")).unwrap(),
        format!("{base}&action=editsubmission")
    );
    for action in ["submit", "savesubmission", "delete", "editsubmission_extra"] {
        assert_eq!(
            restore_record(&format!("{base}&action={action}")).unwrap(),
            base
        );
    }
    for raw in [
        "https://lms.s.isct.ac.jp/2025/course/view.php?id=100&action=editsubmission",
        "https://syllabus.s.isct.ac.jp/mod/assign/view.php?id=100&action=editsubmission",
    ] {
        assert!(!restore_record(raw).unwrap().contains("action="));
    }
}
