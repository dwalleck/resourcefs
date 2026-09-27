//! Hermetic fixtures shared by test targets through the `test-support` feature.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use tokio::sync::Mutex;
use url::Url;

use crate::{ArtifactId, ErrorCategory, ResourceError, SessionStorage, WorkspacePath};

fn fixture_path(relative_path: &str) -> PathBuf {
    let relative_path = WorkspacePath::new(relative_path).expect("valid relative fixture path");
    #[cfg(windows)]
    let root = Path::new(r"C:\resourcefs-fixtures");
    #[cfg(not(windows))]
    let root = Path::new("/resourcefs-fixtures");
    root.join(relative_path.as_path())
}

/// Returns a native-absolute file URI without accessing the environment or filesystem.
///
/// The path is synthetic: callers must not use it for filesystem I/O.
/// Panics if `relative_path` is not a valid nonempty Workspace Path.
pub fn file_uri(relative_path: &str) -> Url {
    Url::from_file_path(fixture_path(relative_path)).expect("absolute fixture file path")
}

/// Returns a native-absolute directory URI, including its trailing slash.
///
/// Like [`file_uri`], this uses a synthetic path and performs no filesystem I/O.
/// Panics if `relative_path` is not a valid nonempty Workspace Path.
pub fn directory_uri(relative_path: &str) -> Url {
    Url::from_directory_path(fixture_path(relative_path)).expect("absolute fixture directory path")
}

/// An in-memory [`SessionStorage`] that records what it was handed.
///
/// Six contract targets across two crates each wrote their own copy of this
/// fake, differing only in field names, error wording, and which one
/// observation they exposed (rfs-exoi). This one exposes all four. A missing
/// object is `not_found` and stored bytes that are not UTF-8 are
/// `source_unavailable`, so a missing object and a corrupt one stay distinct.
/// `path_session_contract.rs` keeps its own richer fake (call log, injected
/// write failure) on purpose.
#[derive(Default)]
pub struct MemoryStorage {
    state: Mutex<MemoryState>,
}

/// Content and write log behind one lock, so a write's log entry and its
/// content become visible together.
#[derive(Default)]
struct MemoryState {
    content: HashMap<ArtifactId, Vec<u8>>,
    written: Vec<ArtifactId>,
}

impl MemoryStorage {
    /// How many `write_atomic` calls reached the store.
    pub async fn write_calls(&self) -> usize {
        self.state.lock().await.written.len()
    }

    /// Every id the store was handed, in write order.
    pub async fn written_ids(&self) -> Vec<ArtifactId> {
        self.state.lock().await.written.clone()
    }

    /// How many objects are stored now.
    pub async fn count(&self) -> usize {
        self.state.lock().await.content.len()
    }

    /// Every stored object keyed by its id, read from the store itself so it
    /// cannot agree with a session by construction.
    pub async fn inventory(&self) -> BTreeMap<ArtifactId, Vec<u8>> {
        self.state
            .lock()
            .await
            .content
            .iter()
            .map(|(id, bytes)| (*id, bytes.clone()))
            .collect()
    }
}

#[async_trait]
impl SessionStorage for MemoryStorage {
    async fn content_equals(&self, id: ArtifactId, content: &[u8]) -> Result<bool, ResourceError> {
        Ok(self
            .state
            .lock()
            .await
            .content
            .get(&id)
            .is_some_and(|stored| stored.as_slice() == content))
    }

    async fn write_atomic(&self, id: ArtifactId, content: &[u8]) -> Result<(), ResourceError> {
        let mut state = self.state.lock().await;
        state.written.push(id);
        state.content.insert(id, content.to_vec());
        Ok(())
    }

    async fn read(&self, id: ArtifactId) -> Result<String, ResourceError> {
        let bytes = self
            .state
            .lock()
            .await
            .content
            .get(&id)
            .cloned()
            .ok_or_else(|| ResourceError::new(ErrorCategory::NotFound, "missing test artifact"))?;
        String::from_utf8(bytes).map_err(|_| {
            ResourceError::new(
                ErrorCategory::SourceUnavailable,
                "invalid test artifact UTF-8",
            )
        })
    }

    async fn remove(&self, id: ArtifactId) -> Result<(), ResourceError> {
        self.state.lock().await.content.remove(&id);
        Ok(())
    }

    async fn mark_disconnected(&self) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// One policy for wall-clock performance budgets in tests (rfs-cn1r).
///
/// A wall-clock ceiling measures the machine as much as the code. On an idle
/// workstation `100 ms` is a real assertion; on a shared three-core CI runner
/// executing hundreds of test processes the same number is a coin flip, and
/// every raise made to quiet it (rfs-1e6h) weakened what it could catch.
/// Enforcement therefore belongs to a controlled host, and a shared runner
/// should report the number without failing on it.
///
/// `RFS_BUDGETS` selects the policy:
///
/// - unset or `enforce`: an over-budget measurement fails the test. This is
///   the default, so a local run of `scripts/ci-gates.py` enforces.
/// - `report`: every measurement prints one line,
///   `budget <name>: <elapsed> against <budget> (<verdict>)`, and never fails.
///   The GitHub-hosted CI legs set this.
///
/// Any other value is a configuration error and panics rather than silently
/// picking a policy.
///
/// Only host-dominated wall-clock ceilings go through here. Allocation
/// ceilings, timeout and cancellation contracts ("refused without waiting for
/// the deadline"), fixture control deadlines, and liveness backstops are not
/// budgets and keep asserting directly.
pub mod wall_budget {

    use std::time::Duration;

    /// The environment variable that selects the budget policy.
    pub const BUDGET_POLICY_VARIABLE: &str = "RFS_BUDGETS";

    /// Whether an over-budget measurement fails or is only reported.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BudgetPolicy {
        /// Fail the test when the measurement exceeds its budget.
        Enforce,
        /// Print the measurement and its verdict, never fail.
        Report,
    }

    impl BudgetPolicy {
        /// Parses the value of [`BUDGET_POLICY_VARIABLE`]; `None` means unset.
        ///
        /// # Errors
        ///
        /// Returns the rejected value when it names no policy.
        pub fn parse(value: Option<&str>) -> Result<Self, String> {
            match value {
                None | Some("enforce") => Ok(Self::Enforce),
                Some("report") => Ok(Self::Report),
                Some(other) => Err(other.to_owned()),
            }
        }

        /// Reads the policy from the environment.
        ///
        /// # Panics
        ///
        /// Panics when the variable is set to a value that is not a policy, or is
        /// not valid Unicode.
        #[must_use]
        pub fn from_environment() -> Self {
            let value = match std::env::var(BUDGET_POLICY_VARIABLE) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => {
                    panic!(
                        "{BUDGET_POLICY_VARIABLE} must be `enforce` or `report`, not non-Unicode bytes"
                    )
                }
            };
            Self::parse(value.as_deref()).unwrap_or_else(|rejected| {
                panic!("{BUDGET_POLICY_VARIABLE} must be `enforce` or `report`, not {rejected:?}")
            })
        }
    }

    /// What a measurement means under a policy.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BudgetVerdict {
        /// Within budget.
        Within,
        /// Over budget and the policy enforces: the caller must fail.
        Exceeded,
        /// Over budget but only reported.
        ExceededReported,
    }

    /// Classifies one measurement without side effects.
    #[must_use]
    pub fn evaluate(policy: BudgetPolicy, elapsed: Duration, budget: Duration) -> BudgetVerdict {
        if elapsed <= budget {
            BudgetVerdict::Within
        } else {
            match policy {
                BudgetPolicy::Enforce => BudgetVerdict::Exceeded,
                BudgetPolicy::Report => BudgetVerdict::ExceededReported,
            }
        }
    }

    /// Applies `policy` to one measurement: reports it, and panics only when an
    /// enforcing policy sees it over budget.
    ///
    /// # Panics
    ///
    /// Panics when `policy` is [`BudgetPolicy::Enforce`] and `elapsed` exceeds
    /// `budget`.
    pub fn check_wall_budget_with(
        policy: BudgetPolicy,
        name: &str,
        elapsed: Duration,
        budget: Duration,
    ) {
        let verdict = evaluate(policy, elapsed, budget);
        if policy == BudgetPolicy::Report {
            let label = match verdict {
                BudgetVerdict::Within => "within",
                BudgetVerdict::Exceeded | BudgetVerdict::ExceededReported => {
                    "OVER, not enforced here"
                }
            };
            eprintln!("budget {name}: {elapsed:?} against {budget:?} ({label})");
        }
        assert!(
            verdict != BudgetVerdict::Exceeded,
            "budget {name}: {elapsed:?} exceeded {budget:?}"
        );
    }

    /// Applies the policy selected by [`BUDGET_POLICY_VARIABLE`] to one
    /// measurement. See the module documentation.
    ///
    /// # Panics
    ///
    /// Panics when enforcing and over budget, or when the variable is invalid.
    pub fn check_wall_budget(name: &str, elapsed: Duration, budget: Duration) {
        check_wall_budget_with(BudgetPolicy::from_environment(), name, elapsed, budget);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const BUDGET: Duration = Duration::from_millis(100);
        const OVER: Duration = Duration::from_millis(101);

        #[test]
        fn policy_defaults_to_enforce_and_rejects_unknown_values() {
            assert_eq!(BudgetPolicy::parse(None), Ok(BudgetPolicy::Enforce));
            assert_eq!(
                BudgetPolicy::parse(Some("enforce")),
                Ok(BudgetPolicy::Enforce)
            );
            assert_eq!(
                BudgetPolicy::parse(Some("report")),
                Ok(BudgetPolicy::Report)
            );
            assert_eq!(
                BudgetPolicy::parse(Some("Report")),
                Err("Report".to_owned())
            );
            assert_eq!(BudgetPolicy::parse(Some("")), Err(String::new()));
        }

        #[test]
        fn report_never_fails_even_far_over_budget() {
            check_wall_budget_with(
                BudgetPolicy::Report,
                "report_over",
                Duration::from_secs(3600),
                BUDGET,
            );
            assert_eq!(
                evaluate(BudgetPolicy::Report, OVER, BUDGET),
                BudgetVerdict::ExceededReported
            );
        }

        #[test]
        fn enforce_passes_at_the_budget_exactly() {
            check_wall_budget_with(BudgetPolicy::Enforce, "enforce_at", BUDGET, BUDGET);
            assert_eq!(
                evaluate(BudgetPolicy::Enforce, BUDGET, BUDGET),
                BudgetVerdict::Within
            );
        }

        #[test]
        #[should_panic(expected = "budget enforce_over: 101ms exceeded 100ms")]
        fn enforce_fails_over_budget() {
            check_wall_budget_with(BudgetPolicy::Enforce, "enforce_over", OVER, BUDGET);
        }
    }
}
