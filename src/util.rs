use std::path::Path;
use tracing::{trace};
use uuid::Uuid;
use crate::TEMP_ROOT_DIR;

/// Returns a random name (UUID).
/// Used for temp directories and temp files for example.
pub fn get_random_name() -> String {
	trace!("Returning random name");
	let random_name = Uuid::new_v4();
	trace!("Random name generated: {random_name}");
	format!("{random_name}")
}

/// Creates a temp dir and returns the path as String.
pub fn make_temp_dir() -> String {
	trace!("Creating temp directory");
	let temp_dir_name = get_random_name();
	let temp_dir = format!("{}/{}", TEMP_ROOT_DIR, temp_dir_name);
	// Gets unwrapped because we can't continue at all if the temp dir failed to create.
	std::fs::create_dir(Path::new(&temp_dir)).expect("Could not create temp dir");
	trace!("Created temp dir {temp_dir}");

	temp_dir
}
