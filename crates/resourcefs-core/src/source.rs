use async_trait::async_trait;

use crate::{
    ErrorCategory, ErrorReason, OperationGuard, PathReference, ReadAcquisitionLimits,
    ResourceError, ResourceErrorDetails, SourceResource,
};

/// Refuses explicit acquisition controls on a Resource that cannot honor them.
///
/// [`SourceAdapter::read`] requires controls to be honored or rejected, never
/// ignored. Every Resource that cannot honor them owes the caller the same
/// refusal, so it lives here rather than being restated per adapter: an
/// adapter that forgets to call it silently drops caller limits, which is the
/// one outcome the contract forbids and no test would catch.
pub fn reject_acquisition(
    acquisition: Option<&ReadAcquisitionLimits>,
) -> Result<(), ResourceError> {
    if acquisition.is_none() {
        return Ok(());
    }
    Err(ResourceError::new(
        ErrorCategory::UnsupportedProjection,
        "acquisition controls are not supported for this resource",
    )
    .with_details(ResourceErrorDetails::new(
        ErrorReason::AcquisitionControlsUnsupported,
    )))
}

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
