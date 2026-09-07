use resourcefs_core::{ErrorCategory, OperationGuard, ResourceError};

use super::{
    BoundedHttpResponse, HttpFetchFailure, HttpRequest, HttpSubstrate, LogicalDeadline,
    retry_wait_fits,
};

/// One source-neutral logical read sharing an immutable deadline across fetches.
#[derive(Clone, Copy)]
pub(crate) struct BoundedRead<'a> {
    pub(super) substrate: &'a HttpSubstrate,
    pub(super) operation: &'a OperationGuard,
    pub(super) deadline: LogicalDeadline,
}

impl BoundedRead<'_> {
    pub(crate) async fn fetch(
        self,
        request: HttpRequest,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        let Some(retry) = request.retry_copy() else {
            return self
                .substrate
                .fetch_attempt(request, self.operation)
                .await
                .map_err(HttpFetchFailure::into_error);
        };
        tokio::time::timeout_at(self.deadline.0, self.fetch_idempotent(request, retry))
            .await
            .map_err(|_| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "HTTP logical read exceeded the configured deadline",
                )
            })?
    }

    async fn fetch_idempotent(
        self,
        mut request: HttpRequest,
        retry: HttpRequest,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        let mut retry = Some(retry);
        loop {
            let response = match self.substrate.fetch_attempt(request, self.operation).await {
                Ok(response) => response,
                Err(failure)
                    if failure.is_retryable()
                        && retry.is_some()
                        && !self.deadline.remaining().is_zero() =>
                {
                    let Some(next) = retry.take() else {
                        return Err(failure.into_error());
                    };
                    request = next;
                    continue;
                }
                Err(failure) => return Err(failure.into_error()),
            };
            if !matches!(response.status(), 429 | 503) || retry.is_none() {
                return Ok(response);
            }
            let Some(delay) = response.retry_after() else {
                return Ok(response);
            };
            let jitter = (self.substrate.retry_runtime.jitter)()?;
            let remaining = self.deadline.remaining();
            if !retry_wait_fits(remaining, delay, jitter) {
                return Err(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "HTTP Retry-After guidance exceeds the remaining logical deadline",
                ));
            }
            let wait = delay.checked_add(jitter).ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "HTTP Retry-After guidance exceeds the remaining logical deadline",
                )
            })?;
            tokio::select! {
                biased;
                () = self.operation.cancelled() => {
                    return Err(ResourceError::new(
                        ErrorCategory::Cancelled,
                        "HTTP retry wait was cancelled",
                    ));
                }
                () = tokio::time::sleep(wait) => {}
            }
            if self.deadline.remaining().is_zero() {
                return Err(ResourceError::new(
                    ErrorCategory::SourceUnavailable,
                    "HTTP Retry-After guidance left no time for the follow-up request",
                ));
            }
            let Some(next) = retry.take() else {
                return Ok(response);
            };
            request = next;
        }
    }
}
