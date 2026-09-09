//! Cheapest design falsifier: are the four proposed `/facts` spellings free
//! and unambiguous, and do the four existing human routes keep their meaning?
//!
//! Compile against the real public parser:
//!   rustc --edition 2024 -L dependency=target/debug/deps \
//!     --extern resourcefs_core=target/debug/libresourcefs_core.rlib \
//!     .rfs-r31i/falsifiers/grammar_probe.rs -o /tmp/grammar_probe && /tmp/grammar_probe
use resourcefs_core::PathReference;

fn main() {
    let proposed = [
        "pr://rust-lang/rust/159232/reviews/facts",
        "pr://rust-lang/rust/159232/reviews/4988827796/facts",
        "pr://rust-lang/rust/159232/review-comments/facts",
        "pr://rust-lang/rust/159232/review-comments/3826494362/facts",
    ];
    let human = [
        "pr://rust-lang/rust/159232/reviews",
        "pr://rust-lang/rust/159232/reviews/4988827796",
        "pr://rust-lang/rust/159232/review-comments",
        "pr://rust-lang/rust/159232/review-comments/3826494362",
    ];
    let existing = [
        "pr://rust-lang/rust/159232/facts",
        "pr://rust-lang/rust/159232/comments/facts",
        "pr://rust-lang/rust/159232/comments/4959513954/facts",
    ];
    for reference in proposed {
        match PathReference::parse(reference) {
            Ok(parsed) => println!("PROPOSED-CLAIMED {reference} -> {}", parsed.requested()),
            Err(error) => println!("PROPOSED-FREE {reference} -> {:?}", error.category()),
        }
    }
    for reference in human {
        match PathReference::parse(reference) {
            Ok(parsed) => println!("HUMAN-OK {reference} -> {}", parsed.requested()),
            Err(error) => println!("HUMAN-BROKEN {reference} -> {:?}", error.category()),
        }
    }
    for reference in existing {
        match PathReference::parse(reference) {
            Ok(parsed) => println!("EXISTING-OK {reference} -> {}", parsed.requested()),
            Err(error) => println!("EXISTING-BROKEN {reference} -> {:?}", error.category()),
        }
    }
}
