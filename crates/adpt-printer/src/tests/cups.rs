use super::*;

fn student() -> StudentId {
    StudentId::parse("24M12345").unwrap()
}

// The stock lpd backend bills the user in the device URI, so the queue's
// URI must name the student ID as that user, on the campus printer.
// The queue is the installing login's alone and never shared, so nobody
// else, on this computer or the network, prints on this student's bill.
#[test]
fn the_queue_bills_the_student_and_takes_only_the_installing_login() {
    let args = lpadmin_args(&student(), "taro.y", "/tmp/q.ppd").unwrap();
    assert_eq!(
        args[..11],
        [
            "-p",
            PRINTER_NAME,
            "-E",
            "-v",
            "lpd://24M12345@172.20.83.212/kyocera",
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
        assert!(lpadmin_args(&student(), login, "/q.ppd").is_some(), "{login}");
    }
    let long = "a".repeat(256);
    for login in ["", "-x", "a,b", "@admin", "a b", "a;b", "名前", &long] {
        assert!(lpadmin_args(&student(), login, "/q.ppd").is_none(), "{login:?}");
    }
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

    let queue = String::from_utf8_lossy(&queue_ppd()).into_owned();
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
