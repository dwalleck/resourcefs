use async_trait::async_trait;

use crate::{PathReference, ReadResource, ResourceError};

/// Source-neutral seam implemented by every compiled Source Adapter.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    async fn read(&self, reference: &PathReference) -> Result<ReadResource, ResourceError>;
}
