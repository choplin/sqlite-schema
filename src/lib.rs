//! Library support for constructing and inspecting SQLite schema state.

mod desired_state;

pub use desired_state::{DesiredState, DesiredStateError};
