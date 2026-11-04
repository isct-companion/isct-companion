use crate::queue::*;
use adpt_printer::StudentId;
use std::path::Path;

fn student() -> StudentId {
    StudentId::parse("24M12345").unwrap()
}

// The student ID travels in the queue's root-owned device URI; the backend
// reads it back and hands the system lpd backend the plain printer URI.
// A URI without a valid ID is refused, so no job goes out unbilled.
#[test]
fn the_device_uri_carries_the_student_id_to_the_backend() {
    let uri = device_uri(&student());
    assert_eq!(uri, "titech-lpd://172.20.83.212/kyocera?student=24M12345");
    assert_eq!(
        parse_device_uri(&uri).unwrap(),
        (student(), "lpd://172.20.83.212/kyocera".to_string())
    );
    for bad in [
        "lpd://172.20.83.212/kyocera?student=24M12345",
        "titech-lpd://172.20.83.212/kyocera",
        "titech-lpd://172.20.83.212/kyocera?student=someone",
        "titech-lpd://172.20.83.212/kyocera?other=1",
    ] {
        assert!(parse_device_uri(bad).is_err(), "{bad}");
    }
}

// The queue is the installing login's alone and never shared, so nobody
// else, on this computer or the network, prints on this student's bill.
#[test]
fn the_queue_takes_only_the_installing_login() {
    let uri = device_uri(&student());
    let args = lpadmin_args(&student(), "taro.y", "/tmp/q.ppd");
    assert_eq!(
        args[..11],
        [
            "-p",
            PRINTER_NAME,
            "-E",
            "-v",
            &uri,
            "-P",
            "/tmp/q.ppd",
            "-u",
            "allow:taro.y",
            "-o",
            "printer-is-shared=false"
        ]
    );
    assert!(args
        .windows(2)
        .any(|pair| pair == ["-o", "Option18=HardDisk"]));

    for login in ["taro", "taro.y", "s_24m-1", "_spool"] {
        assert!(is_login_name(login), "{login}");
    }
    let long = "a".repeat(256);
    for login in ["", "-x", "a,b", "@admin", "a b", "a;b", "名前", &long] {
        assert!(!is_login_name(login), "{login:?}");
    }
}

#[test]
fn delegate_args_force_the_student_and_one_copy() {
    let args: Vec<String> = [
        "titech-lpd",
        "42",
        "cups-user",
        "title",
        "3",
        "Duplex=None",
        "/tmp/input.ps",
    ]
    .map(String::from)
    .into();
    assert_eq!(
        delegate_args(&args, 0, &student(), None),
        [
            "42",
            "24M12345",
            "title",
            "1",
            "Duplex=None",
            "/tmp/input.ps"
        ]
    );
    let spooled = Path::new("/tmp/spooled.ps");
    assert_eq!(
        delegate_args(&args[..6], 1, &student(), Some(spooled)),
        [
            "43",
            "24M12345",
            "title",
            "1",
            "Duplex=None",
            "/tmp/spooled.ps"
        ]
    );
    assert_eq!(parse_copies("1"), Ok(1));
    assert!(parse_copies("0").is_err());
    assert!(parse_copies("x").is_err());
}

#[test]
fn the_queue_ppd_drops_only_the_kyocera_driver_lines() {
    let ppd = b"*NickName: \"Test\"\r\n\
*cupsPreFilter: \"application/pdf 0 kyoprefilter_F\"\r\n\
*cupsFilter: \"application/vnd.cups-postscript 0 kyofilter_F\"\r\n\
*APDialogExtension: \"/Library/Printers/Kyocera/PDEs/KfSummary.plugin\"\r\n\
*Duplex None/None: \"statusdict begin false setduplexmode false settumble end\"\r\n";
    assert_eq!(
        without_kyocera_driver(ppd),
        b"*NickName: \"Test\"\r\n\
*Duplex None/None: \"statusdict begin false setduplexmode false settumble end\"\r\n"
            .to_vec()
    );
}

// The bundled PPD is Kyocera's, unaltered, and works without Kyocera's
// software once its three driver lines are left out of the queue's copy.
#[test]
fn the_bundled_ppd_needs_no_kyocera_software() {
    let ppd = String::from_utf8_lossy(PPD);
    assert!(ppd.contains("*NickName: \"Kyocera ECOSYS P8060cdn (KPDL)\""));
    assert!(ppd.contains("*FileVersion: \"8.4506\""));
    assert_eq!(PPD.len(), 316_022, "the PPD must stay byte-for-byte");

    let queue = String::from_utf8_lossy(&without_kyocera_driver(PPD)).into_owned();
    assert!(!queue.contains("kyofilter_F"));
    assert!(!queue.contains("kyoprefilter_F"));
    assert!(!queue.contains("/Library/Printers/Kyocera"));
    for (name, value) in INSTALLABLE_OPTIONS {
        assert!(
            ppd.contains(&format!("*{name} {value}/")),
            "missing *{name} {value}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn kyocera_driver_files_lists_filters_and_plugin() {
    use std::path::PathBuf;
    assert_eq!(
        kyocera_driver_files(PPD, Path::new("/cups/filter")),
        vec![
            PathBuf::from("/cups/filter/kyoprefilter_F"),
            PathBuf::from("/cups/filter/kyofilter_F"),
            PathBuf::from("/Library/Printers/Kyocera/PDEs/KfSummary.plugin"),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn check_kyocera_driver_requires_installed_runnable_files() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("adpt-cups-driver-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Driver.plugin")).unwrap();
    for (name, bytes) in [
        ("filter", &b"#!/bin/sh\nexit 255\n"[..]),
        ("foreign", &b"\xcf\xfa\xed\xfe not runnable"[..]),
    ] {
        fs::write(dir.join(name), bytes).unwrap();
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }

    let installed = vec![dir.join("filter"), dir.join("Driver.plugin")];
    let missing = vec![dir.join("filter"), dir.join("Missing.plugin")];
    let foreign = vec![dir.join("foreign"), dir.join("Driver.plugin")];

    assert_eq!(check_kyocera_driver(&installed), Ok(()));
    assert!(check_kyocera_driver(&missing)
        .unwrap_err()
        .contains("is not installed"));
    assert!(check_kyocera_driver(&foreign)
        .unwrap_err()
        .contains("cannot run on this machine"));
    assert!(check_kyocera_driver(&[]).is_err());

    fs::remove_dir_all(&dir).unwrap();
}
