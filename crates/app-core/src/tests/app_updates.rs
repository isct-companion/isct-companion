use super::*;
#[test]
fn stable_update_admission_and_wakeups_respect_version_and_cadence() {
    assert!(!newer_release("1.1.0", "v1.0.9").unwrap());
    // A dev build is not offered the release it was built on top of, but a dev
    // build of a release candidate still is offered the final release.
    assert!(!newer_release("1.1.3-dev.20261021120000+g0041bff", "v1.1.3").unwrap());
    assert!(newer_release("1.0.0-rc.3.dev.20261021120000", "v1.0.0").unwrap());
    assert!(matches!(
        newer_release("1.0.0", "broken"),
        Err(AppUpdateError::Version(_))
    ));
    let now = "2026-10-16T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
    assert!(check_due(None, now, false));
    assert!(!check_due(Some("2026-10-16T01:00:00Z"), now, false));
    assert!(check_due(Some("2026-10-16T00:00:00Z"), now, false));
    assert!(check_due(Some("2026-10-16T11:59:00Z"), now, true));
    assert!(!automatic_updates(&serde_json::json!({"settings":{}})));
    assert!(automatic_updates(
        &serde_json::json!({"settings":{"automaticAppUpdates":true}})
    ));
}

#[test]
fn last_install_is_judged_by_the_version_that_launched() {
    let record = |phase, message: Option<&str>| InstallRecord {
        phase,
        version: "1.1.4".into(),
        message: message.map(str::to_owned),
        at: "2026-10-19T12:00:00Z".into(),
    };
    // Windows exits before the installer finishes, so `installing` is the
    // success record there; only the running version tells them apart.
    assert_eq!(
        last_install(&record(InstallPhase::Installing, None), "1.1.4").unwrap(),
        LastInstall::Installed {
            version: "1.1.4".into()
        }
    );
    // A blocked installer (Smart App Control) leaves the old version running.
    assert_eq!(
        last_install(&record(InstallPhase::Installing, None), "1.1.3").unwrap(),
        LastInstall::Incomplete {
            version: "1.1.4".into()
        }
    );
    assert_eq!(
        last_install(&record(InstallPhase::Error, Some("denied")), "1.1.3").unwrap(),
        LastInstall::Failed {
            version: "1.1.4".into(),
            message: "denied".into()
        }
    );
}

#[test]
fn android_is_offered_a_newer_release_only_once_its_apk_is_attached() {
    let apk = |version: &str| {
        serde_json::json!({
            "name": format!("IsctCompanion_{version}_android.apk"),
            "browser_download_url": format!("https://github.com/isct-companion/isct-companion/releases/download/v{version}/IsctCompanion_{version}_android.apk"),
        })
    };
    let release = |assets: Vec<Value>| serde_json::json!({ "tag_name": "v1.2.0", "draft": false, "prerelease": false, "assets": assets });
    let offered = android_release(&release(vec![apk("1.2.0")]), "1.1.6")
        .unwrap()
        .unwrap();
    assert_eq!(offered.version, "1.2.0");
    assert!(offered
        .apk_url
        .ends_with("/v1.2.0/IsctCompanion_1.2.0_android.apk"));
    // The release is published before the Android build attaches its APK.
    let dmg = serde_json::json!({ "name": "IsctCompanion_1.2.0_aarch64-apple-darwin.dmg", "browser_download_url": "https://github.com/isct-companion/isct-companion/releases/download/v1.2.0/x.dmg" });
    assert_eq!(android_release(&release(vec![dmg]), "1.1.6").unwrap(), None);
    // Already running it, or an APK hosted anywhere else, is never offered.
    assert_eq!(
        android_release(&release(vec![apk("1.2.0")]), "1.2.0").unwrap(),
        None
    );
    let elsewhere = serde_json::json!({ "name": "IsctCompanion_1.2.0_android.apk", "browser_download_url": "https://example.com/IsctCompanion_1.2.0_android.apk" });
    assert_eq!(
        android_release(&release(vec![elsewhere]), "1.1.6").unwrap(),
        None
    );
    let mut draft = release(vec![apk("1.2.0")]);
    draft["prerelease"] = Value::Bool(true);
    assert_eq!(android_release(&draft, "1.1.6").unwrap(), None);
}
