//! Applies the wall-clock budget policy for a test target (rfs-cn1r).
//!
//! The policy itself is `resourcefs_core::test_support::wall_budget`; this
//! module adds the side effects a test needs: failing, printing, and
//! appending each measurement to the file named by `RFS_BUDGET_REPORT`. It
//! lives here rather than in core's `test_support` because appending to a
//! file is platform I/O, which `core_holds_no_platform_io` forbids in
//! `resourcefs-core/src`. Test targets include it with `#[path]`.
//!
//! The report file exists because the harnesses discard a passing test's
//! output: nextest keeps only failures, and the release leg runs libtest
//! captured. `scripts/ci-gates.py` points the variable at a per-phase file
//! and prints the collected lines, over-budget first, when the phase ends.
#![allow(dead_code, reason = "each test target uses a subset")]

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use resourcefs_core::test_support::wall_budget::{
    BUDGET_REPORT_VARIABLE, Bound, BudgetPolicy, judge,
};

/// Checks `elapsed <= budget` under the environment's policy.
///
/// # Panics
///
/// Panics when the policy fails the measurement, when either variable is
/// invalid, or when the report file cannot be written.
pub fn check_wall_budget(name: &str, elapsed: Duration, budget: Duration) {
    apply(Bound::AtMost, name, elapsed, budget);
}

/// Checks `elapsed < budget` under the environment's policy, for a ceiling
/// that was a strict comparison before it was routed through the policy.
///
/// # Panics
///
/// As [`check_wall_budget`].
pub fn check_wall_budget_below(name: &str, elapsed: Duration, budget: Duration) {
    apply(Bound::Below, name, elapsed, budget);
}

fn apply(bound: Bound, name: &str, elapsed: Duration, budget: Duration) {
    let policy = BudgetPolicy::from_environment();
    let judgement = judge(policy, bound, name, elapsed, budget);
    if let Some(path) = report_path() {
        append_report_line(path, judgement.line());
    }
    if policy == BudgetPolicy::Report {
        eprintln!("{}", judgement.line());
    }
    if let Some(failure) = judgement.failure() {
        panic!("{failure}");
    }
}

/// The report file named by `RFS_BUDGET_REPORT`, read once per process.
fn report_path() -> Option<&'static Path> {
    static REPORT: OnceLock<Option<PathBuf>> = OnceLock::new();
    REPORT
        .get_or_init(|| {
            let value = std::env::var_os(BUDGET_REPORT_VARIABLE)?;
            assert!(
                !value.is_empty(),
                "{BUDGET_REPORT_VARIABLE} is set but empty; name a file or unset it"
            );
            Some(PathBuf::from(value))
        })
        .as_deref()
}

/// Appends one line to `path` with a single write on an append-mode handle,
/// so lines from concurrent test processes do not interleave.
///
/// # Panics
///
/// Panics when the file cannot be opened or written: the gate runner asked
/// for the record, and losing it silently would hide the numbers the report
/// exists to show.
pub fn append_report_line(path: &Path, line: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|error| {
            panic!(
                "{BUDGET_REPORT_VARIABLE}: cannot open {}: {error}",
                path.display()
            )
        });
    file.write_all(format!("{line}\n").as_bytes())
        .unwrap_or_else(|error| {
            panic!(
                "{BUDGET_REPORT_VARIABLE}: cannot append to {}: {error}",
                path.display()
            )
        });
}
