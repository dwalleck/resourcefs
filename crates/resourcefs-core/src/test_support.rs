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
/// Enforcement therefore belongs to a quiet dedicated host (rfs-63xx, not
/// yet built), and a contended one should report the number without failing
/// on it short of a gross regression.
///
/// This module is the policy and holds no I/O, because `resourcefs-core`'s
/// modules hold no platform I/O (`core_holds_no_platform_io`). Test targets
/// apply it through `crates/resourcefs-core/tests/support/wall_budget.rs`,
/// included with `#[path]`, which also appends each line to the file named
/// by [`BUDGET_REPORT_VARIABLE`].
///
/// `RFS_BUDGETS` selects the policy, read once per process. There are three
/// behaviors:
///
/// - unset or `report` (the default in-process): a measurement between its
///   budget and its [`HardCeiling`] is recorded as over and passes.
///   Contended hosts run this way: `.github/workflows/ci.yml` sets it for
///   every leg, and a local `scripts/ci-gates.py` uses it for the parallel
///   functional and release legs.
/// - report's hard ceiling: at or over [`HardCeiling::DEFAULT`], which is
///   [`REPORT_HARD_CEILING_FACTOR`] times the budget unless the row sets its
///   own, the test fails anyway, because the overrun is a code regression,
///   not host noise.
/// - `enforce`: over budget at all fails the test. A local
///   `scripts/ci-gates.py` with `RFS_BUDGETS` unset applies it to the serial
///   ignored production budgets, and the dedicated benchmark runner of
///   rfs-63xx will apply it to everything.
///
/// Any other value is a configuration error and panics rather than silently
/// picking a policy; `scripts/ci-gates.py` rejects it before building.
///
/// Every measurement produces one line,
/// `budget <test>[ <label>]: <elapsed> against <budget> (<verdict>)`.
///
/// Only host-dominated wall-clock ceilings belong here. Allocation ceilings,
/// timeout and cancellation contracts ("refused without waiting for the
/// deadline"), fixture control deadlines, and liveness backstops are not
/// budgets and keep asserting directly.
pub mod wall_budget {
    use std::{sync::OnceLock, time::Duration};

    /// The environment variable that selects the budget policy.
    pub const BUDGET_POLICY_VARIABLE: &str = "RFS_BUDGETS";

    /// The environment variable naming a file each measurement is appended
    /// to by the test-target support module.
    pub const BUDGET_REPORT_VARIABLE: &str = "RFS_BUDGET_REPORT";

    /// The [`BUDGET_POLICY_VARIABLE`] value for [`BudgetPolicy::Report`], also
    /// what an unset variable means. `scripts/ci-gates.py` reads this
    /// declaration, so the policy names and the default live in one place.
    pub const REPORT_POLICY: &str = "report";

    /// The [`BUDGET_POLICY_VARIABLE`] value for [`BudgetPolicy::Enforce`].
    /// `scripts/ci-gates.py` reads this declaration too.
    pub const ENFORCE_POLICY: &str = "enforce";

    /// How far over its budget a measurement may be in report mode before it
    /// fails anyway.
    ///
    /// Host noise is real but bounded. The worst on record is rfs-1e6h's fifth
    /// instance: a CPU-bound task of about 7 ms took 108 ms on a contended
    /// hosted runner, roughly 15x its own work. That row's budget was already
    /// 14x that work, so even then it missed by 1.08x. Against their budgets,
    /// the largest recorded miss on any row routed here is the heartbeat,
    /// 505 ms on windows-latest against today's 100 ms: about 5x. The rows
    /// are dominated by real work rather than scheduling, so one taking 10x
    /// its budget is a code regression, not a noisy host. When this was set,
    /// no routed row's budget was close enough to its measured work for
    /// contended noise to reach 10x: the tightest headroom across 71 release
    /// and 50 debug measurements was 3.9x (`operation_journal_budget`'s 10,000
    /// journal transitions, 6.4 ms against 25 ms).
    ///
    /// `scripts/ci-gates.py` reads this declaration for its startup banner;
    /// keep it a plain integer literal.
    pub const REPORT_HARD_CEILING_FACTOR: u32 = 10;

    /// Whether an over-budget measurement fails or is only reported.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BudgetPolicy {
        /// Fail the test when the measurement exceeds its budget.
        Enforce,
        /// Record an over-budget measurement and pass, unless it reaches
        /// [`REPORT_HARD_CEILING_FACTOR`] times its budget.
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
                None | Some(REPORT_POLICY) => Ok(Self::Report),
                Some(ENFORCE_POLICY) => Ok(Self::Enforce),
                Some(other) => Err(other.to_owned()),
            }
        }

        /// The policy selected by the environment, read and validated once.
        ///
        /// # Panics
        ///
        /// Panics when the variable is set to a value that is not a policy, or
        /// is not valid Unicode.
        #[must_use]
        pub fn from_environment() -> Self {
            static POLICY: OnceLock<BudgetPolicy> = OnceLock::new();
            *POLICY.get_or_init(|| {
                let value = match std::env::var(BUDGET_POLICY_VARIABLE) {
                    Ok(value) => Some(value),
                    Err(std::env::VarError::NotPresent) => None,
                    Err(std::env::VarError::NotUnicode(_)) => panic!(
                        "{BUDGET_POLICY_VARIABLE} must be `enforce` or `report`, not non-Unicode bytes"
                    ),
                };
                Self::parse(value.as_deref()).unwrap_or_else(|rejected| {
                    panic!(
                        "{BUDGET_POLICY_VARIABLE} must be `enforce` or `report`, not {rejected:?}"
                    )
                })
            })
        }
    }

    /// Where report mode stops tolerating an over-budget row and fails it.
    ///
    /// [`REPORT_HARD_CEILING_FACTOR`] times the budget is right for most rows.
    /// A row overrides it when 10x is wrong for it: an absolute ceiling keeps
    /// a row CI previously held at a raised bound from getting looser than
    /// that bound, which is what the two 64 MiB write rows do. A larger factor
    /// would keep a row whose budget sits close to its contended work clear of
    /// host noise; no row needs one today.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HardCeiling {
        /// Fail at or over this many times the budget.
        Factor(u32),
        /// Fail at or over this duration. It must not be below the budget.
        At(Duration),
    }

    impl HardCeiling {
        /// The default: [`REPORT_HARD_CEILING_FACTOR`] times the budget.
        pub const DEFAULT: Self = Self::Factor(REPORT_HARD_CEILING_FACTOR);

        /// The ceiling as a duration for `budget`.
        ///
        /// # Panics
        ///
        /// Panics on a factor of zero or an absolute ceiling below the budget,
        /// either of which would fail a within-budget row.
        #[must_use]
        pub fn for_budget(self, budget: Duration) -> Duration {
            match self {
                Self::Factor(factor) => {
                    assert!(factor >= 1, "a hard-ceiling factor must be at least 1");
                    budget.saturating_mul(factor)
                }
                Self::At(ceiling) => {
                    assert!(
                        ceiling >= budget,
                        "hard ceiling {ceiling:?} is below its {budget:?} budget"
                    );
                    ceiling
                }
            }
        }

        fn describe(self, budget: Duration) -> String {
            match self {
                Self::Factor(factor) => format!("{factor}x its {budget:?} budget"),
                Self::At(ceiling) => {
                    format!("its {ceiling:?} hard ceiling ({budget:?} budget)")
                }
            }
        }
    }

    /// How a measurement is compared with its budget, so each converted
    /// assertion keeps the comparison it had.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Bound {
        /// Within budget while `elapsed <= budget`.
        AtMost,
        /// Within budget only while `elapsed < budget`.
        Below,
    }

    impl Bound {
        /// Whether `elapsed` breaks this bound.
        #[must_use]
        pub fn is_over(self, elapsed: Duration, budget: Duration) -> bool {
            match self {
                Self::AtMost => elapsed > budget,
                Self::Below => elapsed >= budget,
            }
        }
    }

    /// What one measurement means under a policy.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Judgement {
        line: String,
        failure: Option<String>,
    }

    impl Judgement {
        /// The measurement's one-line record.
        #[must_use]
        pub fn line(&self) -> &str {
            &self.line
        }

        /// The message the test must fail with, if the policy fails it.
        #[must_use]
        pub fn failure(&self) -> Option<&str> {
            self.failure.as_deref()
        }
    }

    /// Judges one measurement without side effects.
    ///
    /// # Panics
    ///
    /// Panics when `hard` is invalid for `budget`; see
    /// [`HardCeiling::for_budget`].
    #[must_use]
    pub fn judge(
        policy: BudgetPolicy,
        bound: Bound,
        hard: HardCeiling,
        name: &str,
        elapsed: Duration,
        budget: Duration,
    ) -> Judgement {
        let over = bound.is_over(elapsed, budget);
        let hard_ceiling = hard.for_budget(budget);
        let (verdict, fails) = match (policy, over) {
            (_, false) => ("within", false),
            (BudgetPolicy::Enforce, true) => ("OVER", true),
            (BudgetPolicy::Report, true) if elapsed >= hard_ceiling => {
                ("OVER the report hard ceiling", true)
            }
            (BudgetPolicy::Report, true) => ("OVER, not enforced here", false),
        };
        let line = format!("budget {name}: {elapsed:?} against {budget:?} ({verdict})");
        let failure = fails.then(|| match policy {
            BudgetPolicy::Enforce => line.clone(),
            BudgetPolicy::Report => format!(
                "budget {name}: {elapsed:?} is over {}; that is a regression, not host noise",
                hard.describe(budget)
            ),
        });
        Judgement { line, failure }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const BUDGET: Duration = Duration::from_millis(100);
        const OVER: Duration = Duration::from_millis(101);

        #[test]
        fn policy_defaults_to_report_and_rejects_unknown_values() {
            assert_eq!(BudgetPolicy::parse(None), Ok(BudgetPolicy::Report));
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
        fn report_passes_between_the_budget_and_its_hard_ceiling() {
            let just_under_ceiling = BUDGET * REPORT_HARD_CEILING_FACTOR - Duration::from_nanos(1);
            for bound in [Bound::AtMost, Bound::Below] {
                for elapsed in [OVER, just_under_ceiling] {
                    let judgement = judge(
                        BudgetPolicy::Report,
                        bound,
                        HardCeiling::DEFAULT,
                        "noisy",
                        elapsed,
                        BUDGET,
                    );
                    assert_eq!(judgement.failure(), None, "{}", judgement.line());
                    assert!(judgement.line().ends_with("(OVER, not enforced here)"));
                }
            }
        }

        #[test]
        fn report_fails_at_its_hard_ceiling() {
            let at_ceiling = BUDGET * REPORT_HARD_CEILING_FACTOR;
            for bound in [Bound::AtMost, Bound::Below] {
                for elapsed in [at_ceiling, Duration::from_secs(3600)] {
                    let judgement = judge(
                        BudgetPolicy::Report,
                        bound,
                        HardCeiling::DEFAULT,
                        "regressed",
                        elapsed,
                        BUDGET,
                    );
                    assert_eq!(
                        judgement.failure(),
                        Some(
                            format!(
                                "budget regressed: {elapsed:?} is over 10x its 100ms budget; \
                                 that is a regression, not host noise"
                            )
                            .as_str()
                        )
                    );
                    assert!(judgement.line().ends_with("(OVER the report hard ceiling)"));
                }
            }
        }

        #[test]
        fn at_most_admits_the_budget_exactly_and_below_does_not() {
            let at_most = judge(
                BudgetPolicy::Enforce,
                Bound::AtMost,
                HardCeiling::DEFAULT,
                "at",
                BUDGET,
                BUDGET,
            );
            assert_eq!(at_most.failure(), None);
            assert_eq!(at_most.line(), "budget at: 100ms against 100ms (within)");
            let below = judge(
                BudgetPolicy::Enforce,
                Bound::Below,
                HardCeiling::DEFAULT,
                "at",
                BUDGET,
                BUDGET,
            );
            assert_eq!(
                below.failure(),
                Some("budget at: 100ms against 100ms (OVER)")
            );
        }

        #[test]
        fn an_absolute_hard_ceiling_replaces_the_factor() {
            // The 64 MiB rows' shape: a 5 s budget held to the old 30 s CI raise.
            let budget = Duration::from_secs(5);
            let hard = HardCeiling::At(Duration::from_secs(30));
            let under = Duration::from_secs(30) - Duration::from_nanos(1);
            let passes = judge(
                BudgetPolicy::Report,
                Bound::AtMost,
                hard,
                "raised",
                under,
                budget,
            );
            assert_eq!(passes.failure(), None, "{}", passes.line());
            let fails = judge(
                BudgetPolicy::Report,
                Bound::AtMost,
                hard,
                "raised",
                Duration::from_secs(30),
                budget,
            );
            assert_eq!(
                fails.failure(),
                Some(
                    "budget raised: 30s is over its 30s hard ceiling (5s budget); \
                     that is a regression, not host noise"
                )
            );
            // Enforce still fails at 1x whatever the hard ceiling.
            let enforced = judge(
                BudgetPolicy::Enforce,
                Bound::AtMost,
                hard,
                "raised",
                budget + Duration::from_nanos(1),
                budget,
            );
            assert!(enforced.failure().is_some());
        }

        #[test]
        fn a_factor_hard_ceiling_replaces_the_default() {
            let hard = HardCeiling::Factor(25);
            let at_twenty = BUDGET * 20;
            let passes = judge(
                BudgetPolicy::Report,
                Bound::AtMost,
                hard,
                "noisy",
                at_twenty,
                BUDGET,
            );
            assert_eq!(passes.failure(), None, "{}", passes.line());
            let at_default = judge(
                BudgetPolicy::Report,
                Bound::AtMost,
                HardCeiling::DEFAULT,
                "noisy",
                at_twenty,
                BUDGET,
            );
            assert!(at_default.failure().is_some());
            let fails = judge(
                BudgetPolicy::Report,
                Bound::AtMost,
                hard,
                "noisy",
                BUDGET * 25,
                BUDGET,
            );
            assert_eq!(
                fails.failure(),
                Some(
                    "budget noisy: 2.5s is over 25x its 100ms budget; \
                     that is a regression, not host noise"
                )
            );
        }

        #[test]
        #[should_panic(expected = "hard ceiling 50ms is below its 100ms budget")]
        fn a_hard_ceiling_below_the_budget_is_rejected() {
            let _ = HardCeiling::At(Duration::from_millis(50)).for_budget(BUDGET);
        }

        #[test]
        fn enforce_fails_at_one_times_its_budget() {
            let judgement = judge(
                BudgetPolicy::Enforce,
                Bound::AtMost,
                HardCeiling::DEFAULT,
                "over",
                OVER,
                BUDGET,
            );
            assert_eq!(
                judgement.failure(),
                Some("budget over: 101ms against 100ms (OVER)")
            );
        }
    }
}
