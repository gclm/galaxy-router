use std::sync::Arc;

/// 请求队列（流量控制）
#[derive(Clone)]
pub struct RequestQueue {
    semaphore: Arc<tokio::sync::Semaphore>,
    max_queue_size: usize,
    timeout_secs: u64,
}

impl RequestQueue {
    pub fn new(max_queue_size: usize, timeout_secs: u64) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_queue_size)),
            max_queue_size,
            timeout_secs,
        }
    }

    /// 获取队列许可（超时返回 429）
    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, QueueError> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(self.timeout_secs),
            self.semaphore.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(QueueError::QueueClosed),
            Err(_) => Err(QueueError::QueueFull {
                max: self.max_queue_size,
                timeout: self.timeout_secs,
            }),
        }
    }
}

/// 队列错误
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("队列已满，最大排队数: {max}，超时: {timeout}s")]
    QueueFull { max: usize, timeout: u64 },

    #[error("队列已关闭")]
    QueueClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_returns_permit_within_capacity() {
        let q = RequestQueue::new(2, 5);
        let p1 = q.acquire().await.unwrap();
        let p2 = q.acquire().await.unwrap();
        // 容量已满，第三次应当超时
        assert!(matches!(q.acquire().await, Err(QueueError::QueueFull { .. })));
        drop(p1);
        drop(p2);
    }

    #[tokio::test]
    async fn permit_release_allows_new_acquire() {
        let q = RequestQueue::new(1, 5);
        let p = q.acquire().await.unwrap();
        let q2 = q.clone();
        let handle = tokio::spawn(async move {
            // 阻塞直到 permit 被释放
            let _permit = q2.acquire().await.unwrap();
        });
        drop(p);
        // 给 worker 一点时间
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        handle.await.unwrap();
    }
}
