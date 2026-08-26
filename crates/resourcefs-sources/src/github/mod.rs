mod wire;

#[cfg(feature = "test-support")]
pub use wire::{GithubWireKindForTest, GithubWireObservation, inspect_github_wire_for_test};
