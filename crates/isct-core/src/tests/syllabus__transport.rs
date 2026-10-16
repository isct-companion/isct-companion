use super::*;

#[tokio::test]
async fn concurrent_request_starts_are_spaced_apart() {
    let start = Instant::now();
    let starts = (0..3)
        .map(|_| {
            tokio::spawn(async {
                pace().await;
                Instant::now()
            })
        })
        .collect::<Vec<_>>();
    let mut times = Vec::new();
    for handle in starts {
        times.push(handle.await.unwrap());
    }
    times.sort();
    // Three starts take at least two spacings, and not much more.
    let spread = times[2] - times[0];
    assert!(
        spread >= SPACING * 2 - Duration::from_millis(5),
        "{spread:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(1));
}
