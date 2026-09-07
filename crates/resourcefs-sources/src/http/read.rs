use resourcefs_core::{ErrorCategory, OperationGuard, ResourceError};

use super::{
    BoundedHttpResponse, HttpFetchFailure, HttpRequest, HttpSubstrate, LogicalDeadline,
    RedirectBehavior, retry_wait_fits,
};

const MAX_HTTP_READ_ATTEMPTS: usize = 10;

/// Caller-owned attempt and retry permits for one bounded collection operation.
///
/// Keep this budget across every fetch, including parent reads and retries.
/// It deliberately cannot be cloned or reset.
#[derive(Debug)]
pub(crate) struct HttpReadBudget {
    remaining_attempts: usize,
    retry_available: bool,
}

impl HttpReadBudget {
    pub(crate) fn new(max_attempts: usize) -> Result<Self, ResourceError> {
        if !(1..=MAX_HTTP_READ_ATTEMPTS).contains(&max_attempts) {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "HTTP read attempt limit must be between 1 and 10",
            ));
        }
        Ok(Self {
            remaining_attempts: max_attempts,
            retry_available: true,
        })
    }

    pub(crate) fn remaining_attempts(&self) -> usize {
        self.remaining_attempts
    }

    fn charge_attempt(&mut self) -> Result<(), ResourceError> {
        self.remaining_attempts = self.remaining_attempts.checked_sub(1).ok_or_else(|| {
            ResourceError::new(
                ErrorCategory::LimitExceeded,
                "HTTP logical read exhausted its physical attempt budget",
            )
        })?;
        Ok(())
    }

    fn can_retry(&self) -> bool {
        self.retry_available && self.remaining_attempts > 0
    }
}

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
        self.fetch_with_optional_budget(request, None).await
    }

    pub(crate) async fn fetch_with_budget(
        self,
        mut request: HttpRequest,
        budget: &mut HttpReadBudget,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        // Redirects would add physical requests outside this budget's ledger.
        // Mutations and budgeted reads share the existing nonredirecting client.
        request.redirect = RedirectBehavior::Refuse;
        self.fetch_with_optional_budget(request, Some(budget)).await
    }

    async fn fetch_with_optional_budget(
        self,
        request: HttpRequest,
        budget: Option<&mut HttpReadBudget>,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        if request.method != super::HttpMethod::Get {
            if let Some(budget) = budget {
                budget.charge_attempt()?;
            }
            return self
                .substrate
                .fetch_attempt(request, self.operation)
                .await
                .map_err(HttpFetchFailure::into_error);
        }
        let retry = if budget
            .as_ref()
            .is_none_or(|budget| budget.retry_available && budget.remaining_attempts > 1)
        {
            request.retry_copy()
        } else {
            None
        };
        tokio::time::timeout_at(
            self.deadline.0,
            self.fetch_idempotent(request, retry, budget),
        )
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
        mut retry: Option<HttpRequest>,
        mut budget: Option<&mut HttpReadBudget>,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        loop {
            if let Some(budget) = budget.as_deref_mut() {
                budget.charge_attempt()?;
                if !budget.can_retry() {
                    retry = None;
                }
            }
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
                    if let Some(budget) = budget.as_deref_mut() {
                        budget.retry_available = false;
                    }
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
            if let Some(budget) = budget.as_deref_mut() {
                budget.retry_available = false;
            }
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
