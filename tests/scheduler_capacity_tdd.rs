use galaxy_router::scheduler::capacity::ChannelCapacityManager;

#[test]
fn scheduler_capacity_unlimited_channels_always_acquire() {
    let manager = ChannelCapacityManager::new();

    let first = manager.try_acquire("ch-unlimited", 0);
    let second = manager.try_acquire("ch-unlimited", 0);
    let third = manager.try_acquire("ch-unlimited", 0);

    assert!(first.is_some());
    assert!(second.is_some());
    assert!(third.is_some());
}

#[test]
fn scheduler_capacity_enforces_max_concurrency() {
    let manager = ChannelCapacityManager::new();

    let first = manager.try_acquire("ch-limited", 2);
    let second = manager.try_acquire("ch-limited", 2);
    let third = manager.try_acquire("ch-limited", 2);

    assert!(first.is_some());
    assert!(second.is_some());
    assert!(third.is_none());
}

#[test]
fn scheduler_capacity_releases_slot_when_permit_is_dropped() {
    let manager = ChannelCapacityManager::new();

    let first = manager.try_acquire("ch-release", 1).expect("first permit");
    assert!(manager.try_acquire("ch-release", 1).is_none());

    drop(first);

    assert!(manager.try_acquire("ch-release", 1).is_some());
}

#[test]
fn scheduler_capacity_tracks_channels_independently() {
    let manager = ChannelCapacityManager::new();

    let _a = manager.try_acquire("ch-a", 1).expect("a permit");
    let b = manager.try_acquire("ch-b", 1);

    assert!(manager.try_acquire("ch-a", 1).is_none());
    assert!(b.is_some());
}

/// P2.4: 模拟流式场景 — spawned task 持有 permit，task 结束后 permit 自动释放
#[tokio::test]
async fn capacity_permit_stream_release_allows_reacquire_after_task_ends() {
    let manager = std::sync::Arc::new(ChannelCapacityManager::new());

    // max_concurrency=1，只允许一个并发
    let permit = manager.try_acquire("ch-stream", 1).expect("first acquire");
    assert!(
        manager.try_acquire("ch-stream", 1).is_none(),
        "should be at capacity"
    );

    // 将 permit 移入 spawned task（模拟流式处理持有 permit）
    let handle = tokio::spawn(async move {
        let _permit = permit; // RAII: drop 时释放
        // 模拟流式处理耗时
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        // permit 在此处 drop
    });

    // 等待 task 结束
    handle.await.expect("task should complete");

    // task 结束后 permit 已释放，可以重新获取
    let reacquired = manager.try_acquire("ch-stream", 1);
    assert!(
        reacquired.is_some(),
        "permit should be released after task ends"
    );
}
