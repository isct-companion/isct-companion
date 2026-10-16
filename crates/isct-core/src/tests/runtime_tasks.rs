use super::*;
use futures_util::FutureExt;
use tokio::sync::oneshot;
#[tokio::test]
async fn close_rejects_new_work_and_waits_for_existing_work() {
    let tasks = RuntimeTasks::default();
    let (send, receive) = oneshot::channel();
    assert!(tasks.spawn(async move {
        receive.await.unwrap();
    }));
    tasks.close();
    assert!(!tasks.spawn(async {}));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), tasks.wait())
            .await
            .is_err()
    );
    send.send(()).unwrap();
    tasks.wait().await;
    assert_eq!(
        tasks.run(async { 42 }).await,
        Err(RuntimeTaskError::Stopped)
    );
}

#[test]
fn missing_runtime_is_a_recoverable_admission_error() {
    let tasks = RuntimeTasks::default();
    assert!(!tasks.spawn(async {}));
    assert_eq!(
        tasks.run(async { 42 }).now_or_never(),
        Some(Err(RuntimeTaskError::Unavailable))
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert_eq!(runtime.block_on(tasks.run(async { 42 })), Ok(42));
    tasks.close();
    runtime.block_on(tasks.wait());
}

#[tokio::test]
async fn worker_panic_is_typed_and_does_not_close_admission() {
    let tasks = RuntimeTasks::default();
    let result = tasks
        .run(async { panic!("synthetic worker failure") })
        .await;
    assert_eq!(result, Err(RuntimeTaskError::Panicked));
    assert_eq!(tasks.run(async { 42 }).await, Ok(42));
    tasks.close();
    tasks.wait().await;
}

#[tokio::test]
async fn unexpected_worker_cancellation_has_its_own_category() {
    let worker = tokio::spawn(std::future::pending::<()>());
    worker.abort();
    assert_eq!(
        RuntimeTaskError::from(worker.await.unwrap_err()),
        RuntimeTaskError::Interrupted
    );
}
