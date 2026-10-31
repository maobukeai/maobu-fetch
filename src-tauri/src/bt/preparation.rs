//! Metadata/recheck preparation must release reconciliation when a user stops or removes the task.
use super::BtEngine;
use std::future::Future;
use tokio_util::sync::CancellationToken;

pub(super) struct PreparationState {
    id: String,
    generation: String,
    cancel: CancellationToken,
}
pub(super) struct Preparation<'a> {
    engine: &'a BtEngine,
    generation: String,
    cancel: CancellationToken,
}
impl BtEngine {
    pub(super) fn begin_seed_preparation(&self, id: &str) -> Preparation<'_> {
        let generation = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let mut current = self
            .seed_preparation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *current = Some(PreparationState {
            id: id.into(),
            generation: generation.clone(),
            cancel: cancel.clone(),
        });
        Preparation {
            engine: self,
            generation,
            cancel,
        }
    }
    pub(super) fn cancel_seed_preparation(&self, id: Option<&str>) {
        let current = self
            .seed_preparation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(current) = current
            .as_ref()
            .filter(|current| id.is_none_or(|id| current.id == id))
        {
            current.cancel.cancel();
        }
    }
    #[cfg(test)]
    pub(super) fn has_seed_preparation(&self, id: &str) -> bool {
        self.seed_preparation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|current| current.id == id)
    }
}
impl Preparation<'_> {
    pub(super) async fn run<F: Future>(&self, future: F) -> Option<F::Output> {
        tokio::select! { biased; _ = self.cancel.cancelled() => None, result = future => Some(result) }
    }
}
impl Drop for Preparation<'_> {
    fn drop(&mut self) {
        let mut current = self
            .engine
            .seed_preparation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if current
            .as_ref()
            .is_some_and(|current| current.generation == self.generation)
        {
            current.take();
        }
    }
}

#[cfg(test)]
mod tests;
