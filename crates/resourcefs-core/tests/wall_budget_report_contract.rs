//! The budget report file collects every measurement intact (rfs-cn1r).
//!
//! `scripts/ci-gates.py` points `RFS_BUDGET_REPORT` at one file per phase
//! while nextest runs hundreds of test processes against it, so the append
//! must keep each line whole under concurrent writers.
#[path = "support/wall_budget.rs"]
mod wall_budget;

use std::{path::PathBuf, thread};

fn report_file(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rfs-budget-report-{label}-{}.log",
        std::process::id()
    ));
    if let Err(error) = std::fs::remove_file(&path) {
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
    }
    path
}

#[test]
fn appended_lines_arrive_in_order() {
    let path = report_file("order");
    wall_budget::append_report_line(&path, "budget first: 1ms against 2ms (within)");
    wall_budget::append_report_line(&path, "budget second: 3ms against 2ms (OVER)");
    let recorded = std::fs::read_to_string(&path).expect("report file");
    std::fs::remove_file(&path).expect("remove report file");
    assert_eq!(
        recorded,
        "budget first: 1ms against 2ms (within)\nbudget second: 3ms against 2ms (OVER)\n"
    );
}

#[test]
fn concurrent_writers_never_split_a_line() {
    const WRITERS: usize = 8;
    const LINES: usize = 200;
    let path = report_file("concurrent");
    thread::scope(|scope| {
        for writer in 0..WRITERS {
            let path = &path;
            scope.spawn(move || {
                for line in 0..LINES {
                    wall_budget::append_report_line(
                        path,
                        &format!("budget w{writer}l{line}: {}", "x".repeat(200)),
                    );
                }
            });
        }
    });
    let recorded = std::fs::read_to_string(&path).expect("report file");
    std::fs::remove_file(&path).expect("remove report file");
    let lines: Vec<&str> = recorded.lines().collect();
    assert_eq!(lines.len(), WRITERS * LINES);
    for line in lines {
        assert!(
            line.starts_with("budget w") && line.ends_with(&"x".repeat(200)),
            "split or interleaved line: {line:?}"
        );
    }
}
