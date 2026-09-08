use resourcefs_core::{
    AcquisitionLimitKind, ErrorCategory, ErrorReason, HttpStatus, LimitDetail, OperationGuard,
    ReadAcquisitionLimits, ResourceError, ResourceErrorDetails, RetryGuidance,
};

use super::{
    BoundedHttpResponse, HttpRequest, HttpSubstrate, LogicalDeadline, RedirectBehavior,
    retry_wait_fits,
};

const MAX_HTTP_READ_ATTEMPTS: usize = 10;

/// Caller-owned attempt and retry permits for one bounded collection operation.
///
/// Keep this budget across every fetch, including parent reads and retries.
/// It deliberately cannot be cloned or reset.
#[derive(Debug)]
pub(crate) struct HttpReadBudget {
    remaining_attempts: usize,
    max_attempts: usize,
    byte_limits: Option<ReadAcquisitionLimits>,
    accepted_body_bytes: usize,
    retry_available: bool,
}

impl HttpReadBudget {
    pub(crate) fn new(max_attempts: usize) -> Result<Self, ResourceError> {
        if !(1..=MAX_HTTP_READ_ATTEMPTS).contains(&max_attempts) {
            return Err(ResourceError::new(
                ErrorCategory::InvalidReference,
                "HTTP read attempt limit must be between 1 and 10",
            )
            .with_details(ResourceErrorDetails::new(
                ErrorReason::InvalidAcquisitionLimit,
            )));
        }
        Ok(Self {
            remaining_attempts: max_attempts,
            max_attempts,
            byte_limits: None,
            accepted_body_bytes: 0,
            retry_available: true,
        })
    }

    pub(crate) fn with_limits(limits: &ReadAcquisitionLimits) -> Self {
        Self {
            remaining_attempts: limits.max_attempts(),
            max_attempts: limits.max_attempts(),
            byte_limits: Some(*limits),
            accepted_body_bytes: 0,
            retry_available: true,
        }
    }

    pub(crate) fn used_attempts(&self) -> usize {
        self.max_attempts - self.remaining_attempts
    }

    pub(crate) fn accepted_body_bytes(&self) -> usize {
        self.accepted_body_bytes
    }

    /// Charges only a verified body the source chooses to use, including cache reuse.
    /// Refusal does not reset prior admissions or consumed attempts.
    pub(crate) fn admit_body(&mut self, bytes: usize) -> Result<(), ResourceError> {
        if let Some(limits) = self.byte_limits {
            if bytes > limits.max_response_bytes() {
                return Err(limit_error(
                    AcquisitionLimitKind::ResponseBodyBytes,
                    limits.max_response_bytes(),
                    Some(bytes),
                ));
            }
            let remaining = limits.max_accepted_body_bytes() - self.accepted_body_bytes;
            if bytes > remaining {
                return Err(limit_error(
                    AcquisitionLimitKind::AcceptedBodyBytes,
                    limits.max_accepted_body_bytes(),
                    self.accepted_body_bytes.checked_add(bytes),
                ));
            }
        }
        self.accepted_body_bytes =
            self.accepted_body_bytes.checked_add(bytes).ok_or_else(|| {
                ResourceError::new(
                    ErrorCategory::LimitExceeded,
                    "HTTP admitted byte counter overflow",
                )
                .with_details(ResourceErrorDetails::new(ErrorReason::LimitExceeded))
            })?;
        Ok(())
    }
    pub(crate) fn remaining_attempts(&self) -> usize {
        self.remaining_attempts
    }

    fn charge_attempt(&mut self) -> Result<(), ResourceError> {
        self.remaining_attempts = self.remaining_attempts.checked_sub(1).ok_or_else(|| {
            limit_error(
                AcquisitionLimitKind::Attempts,
                self.max_attempts,
                self.max_attempts.checked_add(1),
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
    /// Readable crate-wide so a refusal can name the bound it enforced.
    pub(crate) response_ceiling: usize,
    timeout: std::time::Duration,
}

impl BoundedRead<'_> {
    /// Final source acceptance shares the network deadline and cancellation state.
    pub(crate) fn check_acceptance(self) -> Result<(), ResourceError> {
        if !self.operation.is_active() && !self.operation.is_committing() {
            return Err(
                ResourceError::new(ErrorCategory::Cancelled, "HTTP read was cancelled")
                    .with_details(ResourceErrorDetails::new(ErrorReason::Cancelled)),
            );
        }
        if self.deadline.remaining().is_zero() {
            return Err(
                self.deadline_error("HTTP logical read exceeded the configured deadline", None)
            );
        }
        Ok(())
    }

    fn deadline_error(
        self,
        message: &'static str,
        response: Option<&BoundedHttpResponse>,
    ) -> ResourceError {
        let mut details = ResourceErrorDetails::new(ErrorReason::DeadlineExceeded).with_limit(
            LimitDetail::new(
                AcquisitionLimitKind::ElapsedNanoseconds,
                u64::try_from(self.timeout.as_nanos()).expect("bounded HTTP timeout"),
                None,
            )
            .expect("positive HTTP timeout"),
        );
        if let Some(response) = response {
            details =
                details.with_http_status(HttpStatus::new(response.status()).expect("HTTP status"));
            if let Some(delay) = response.retry_after() {
                details = details.with_retry_guidance(RetryGuidance::DelaySeconds(delay.as_secs()));
            }
            if let Some(reset) = response.rate_limit_reset() {
                details = details.with_rate_limit_reset(reset);
            }
        }
        ResourceError::new(ErrorCategory::SourceUnavailable, message).with_details(details)
    }

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
            self.fetch_bounded_attempts(request, retry, budget),
        )
        .await
        .map_err(|_| {
            self.deadline_error("HTTP logical read exceeded the configured deadline", None)
        })?
    }

    async fn fetch_bounded_attempts(
        self,
        mut request: HttpRequest,
        mut retry: Option<HttpRequest>,
        mut budget: Option<&mut HttpReadBudget>,
    ) -> Result<BoundedHttpResponse, ResourceError> {
        loop {
            // timeout_at polls its future first: refuse expired/cancelled work
            // before charging an attempt or initiating egress.
            self.check_acceptance()?;
            if let Some(budget) = budget.as_deref_mut() {
                budget.charge_attempt()?;
                if !budget.can_retry() {
                    retry = None;
                }
            }
            let response = match self
                .substrate
                .fetch_attempt(request, self.operation, self.response_ceiling)
                .await
            {
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
                return Err(self.deadline_error(
                    "HTTP Retry-After guidance exceeds the remaining logical deadline",
                    Some(&response),
                ));
            }
            let wait = delay.checked_add(jitter).ok_or_else(|| {
                self.deadline_error(
                    "HTTP Retry-After guidance exceeds the remaining logical deadline",
                    Some(&response),
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
                    ).with_details(ResourceErrorDetails::new(ErrorReason::Cancelled)));
                }
                () = tokio::time::sleep(wait) => {}
            }
            if self.deadline.remaining().is_zero() {
                return Err(self.deadline_error(
                    "HTTP Retry-After guidance left no time for the follow-up request",
                    Some(&response),
                ));
            }
            let Some(next) = retry.take() else {
                return Ok(response);
            };
            request = next;
        }
    }
}

fn limit_error(kind: AcquisitionLimitKind, bound: usize, observed: Option<usize>) -> ResourceError {
    ResourceError::new(
        ErrorCategory::LimitExceeded,
        "HTTP logical read exceeded an acquisition limit",
    )
    .with_details(
        ResourceErrorDetails::new(ErrorReason::LimitExceeded).with_limit(
            LimitDetail::new(kind, bound as u64, observed.map(|value| value as u64))
                .expect("validated positive acquisition bound"),
        ),
    )
}

impl HttpSubstrate {
    /// Legacy reads keep substrate policy, without an implicit cumulative cap.
    pub(crate) fn begin_read<'a>(
        &'a self,
        operation: &'a OperationGuard,
    ) -> Result<BoundedRead<'a>, ResourceError> {
        Ok(BoundedRead {
            substrate: self,
            operation,
            deadline: LogicalDeadline::new(tokio::time::Instant::now(), self.ceilings.timeout())?,
            response_ceiling: self.ceilings.fetch_bytes(),
            timeout: self.ceilings.timeout(),
        })
    }

    pub(crate) fn begin_read_with_limits<'a>(
        &'a self,
        operation: &'a OperationGuard,
        limits: &ReadAcquisitionLimits,
    ) -> Result<(BoundedRead<'a>, ReadAcquisitionLimits), ResourceError> {
        let effective = ReadAcquisitionLimits::new(
            Some(limits.max_attempts()),
            Some(self.ceilings.timeout().min(limits.timeout())),
            Some(self.ceilings.fetch_bytes().min(limits.max_response_bytes())),
            Some(limits.max_accepted_body_bytes()),
            Some(limits.max_representation_bytes()),
        )?;
        Ok((
            BoundedRead {
                substrate: self,
                operation,
                deadline: LogicalDeadline::new(tokio::time::Instant::now(), effective.timeout())?,
                response_ceiling: effective.max_response_bytes(),
                timeout: effective.timeout(),
            },
            effective,
        ))
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
