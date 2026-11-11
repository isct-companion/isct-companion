use super::*;

// New endpoints listen outside Windows' dynamic range, where boot-time
// exclusions land; repairs replace ephemeral ports.
#[test]
fn endpoints_stay_out_of_the_dynamic_port_range() {
    let token = uuid::Uuid::new_v4();
    let ports: Vec<u16> = candidate_ports(token).collect();
    assert_eq!(ports.len(), usize::from(PORT_ATTEMPTS));
    assert!(ports.iter().all(|port| STABLE_PORTS.contains(port)));
    const { assert!(STABLE_PORTS.end <= 49_152) };
    let stable = Endpoint {
        port: ports[0],
        token,
    };
    assert!(stable.stable());
    assert!(!Endpoint {
        port: 51_234,
        ..stable.clone()
    }
    .stable());
    // The queue's marker follows the URL, so a moved endpoint needs a repair.
    assert_ne!(
        stable.marker(),
        Endpoint {
            port: ports[1],
            ..stable.clone()
        }
        .marker()
    );
}
