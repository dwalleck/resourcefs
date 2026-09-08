use async_trait::async_trait;

use crate::{OperationGuard, PathReference, ReadAcquisitionLimits, ResourceError, SourceResource};

/// Source-neutral seam implemented by every compiled Source Adapter.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// Reads one Resource, honoring cancellation.
    ///
    /// The [`OperationGuard`] is threaded so a source whose read can block —
    /// notably a network source, whose request otherwise runs to its full
    /// timeout — can abandon the work when the caller cancels. Local sources
    /// may ignore it. This mirrors [`crate::DiscoveryAdapter`], whose `search`
    /// already carries a guard; a read without one was the asymmetry.
    ///
    /// Explicit acquisition controls must be honored or rejected; they must
    /// never be silently ignored by an unsupported resource.
    async fn read(
        &self,
        reference: &PathReference,
        operation: &OperationGuard,
        acquisition: Option<&ReadAcquisitionLimits>,
    ) -> Result<SourceResource, ResourceError>;
}
