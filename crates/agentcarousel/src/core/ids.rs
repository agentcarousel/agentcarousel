use crate::RunId;
use ulid::Ulid;

/// Allocate a new time-ordered [`RunId`] (ULID).
pub fn new_run_id() -> RunId {
    RunId(Ulid::new().to_string())
}
