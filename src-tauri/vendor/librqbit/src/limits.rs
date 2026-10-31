use arc_swap::ArcSwapOption;
use governor::DefaultDirectRateLimiter as RateLimiter;
use governor::Quota;
use serde::Deserialize;
use serde::Serialize;
use std::num::NonZeroU32;
use std::sync::Arc;

#[derive(Default, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitsConfig {
    pub upload_bps: Option<NonZeroU32>,
    pub download_bps: Option<NonZeroU32>,
}

struct Limit {
    limiter: ArcSwapOption<RateLimitBucket>,
}

struct RateLimitBucket {
    limiter: RateLimiter,
    burst: NonZeroU32,
}

impl Limit {
    fn new_inner(bps: Option<NonZeroU32>) -> Option<Arc<RateLimitBucket>> {
        let bps = bps?;
        Some(Arc::new(RateLimitBucket { limiter: RateLimiter::direct(Quota::per_second(bps)), burst: bps }))
    }

    fn new(bps: Option<NonZeroU32>) -> Self {
        Self {
            limiter: ArcSwapOption::new(Self::new_inner(bps)),
        }
    }

    async fn acquire(&self, size: NonZeroU32) -> crate::Result<()> {
        let mut remaining = size.get();
        while remaining > 0 {
            let Some(bucket) = self.limiter.load_full() else { break; };
            // A BitTorrent block can exceed a low per-second quota. Reserve
            // bounded portions instead of failing with InsufficientCapacity.
            let part = NonZeroU32::new(remaining.min(bucket.burst.get())).unwrap_or(NonZeroU32::MIN);
            bucket.limiter.until_n_ready(part).await?;
            remaining -= part.get();
        }
        Ok(())
    }

    fn set(&self, limit: Option<NonZeroU32>) {
        // Reapplying an unchanged setting must not replenish burst tokens.
        if self.get() == limit { return; }
        let new = Self::new_inner(limit);
        self.limiter.swap(new);
    }

    fn get(&self) -> Option<NonZeroU32> {
        self.limiter.load().as_ref().map(|bucket| bucket.burst)
    }
}

pub struct Limits {
    down: Limit,
    up: Limit,
}

impl Default for Limits {
    fn default() -> Self { Self::new(LimitsConfig::default()) }
}

impl Limits {
    pub fn new(config: LimitsConfig) -> Self {
        Self {
            down: Limit::new(config.download_bps),
            up: Limit::new(config.upload_bps),
        }
    }

    pub async fn prepare_for_upload(&self, len: NonZeroU32) -> crate::Result<()> {
        self.up.acquire(len).await
    }

    pub async fn prepare_for_download(&self, len: NonZeroU32) -> crate::Result<()> {
        self.down.acquire(len).await
    }

    pub fn set_upload_bps(&self, bps: Option<NonZeroU32>) {
        self.up.set(bps);
    }

    pub fn set_download_bps(&self, bps: Option<NonZeroU32>) {
        self.down.set(bps);
    }

    pub fn get_upload_bps(&self) -> Option<NonZeroU32> {
        self.up.get()
    }

    pub fn get_download_bps(&self) -> Option<NonZeroU32> {
        self.down.get()
    }

    pub fn get_config(&self) -> LimitsConfig {
        LimitsConfig {
            upload_bps: self.get_upload_bps(),
            download_bps: self.get_download_bps(),
        }
    }
}
