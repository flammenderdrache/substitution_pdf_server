use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use chrono::{NaiveDateTime, Utc};
use sha2::{Sha512, Digest};
use sqlx::PgPool;
use substitution_pdf_to_json::SubstitutionSchedule;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace};
use crate::{Schoolday, util};
use std::fs;
use crate::util::get_today_string;

pub struct JsonHandler {
	jsons: RwLock<HashMap<Schoolday, String>>,
	hashes: RwLock<HashMap<Schoolday, String>>,
}

impl JsonHandler {
	pub fn new() -> Self {
		let jsons = RwLock::new(HashMap::new());
		let hashes = RwLock::new(HashMap::new());

		Self {
			jsons,
			hashes,
		}
	}

	/// Updates the internal json store.
	/// Also saves the json in the database.
	#[allow(clippy::similar_names)]
	pub async fn update(&self, day: Schoolday, pdf: Vec<u8>, pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
		let mut hasher = Sha512::new();
		Digest::update(&mut hasher, &pdf);
		let hash_bytes = hasher.finalize();
		let hash = hex::encode(hash_bytes);

		let hashes = self.hashes.read().await;
		if let Some(old_hash) = hashes.get(&day) {
			if hash == *old_hash {
				debug!("{day}: New hash matched old hash");
				return Ok(());
			}
		}

		// Drop the read lock as it is not needed anymore.
		// We would also deadlock as we request a read lock next.
		std::mem::drop(hashes);

		{
			trace!("Putting new hash into hash store.");
			let mut hashes = self.hashes.write().await;
			let _ = hashes.insert(day, hash.clone());
		}

		debug!("Creating temp dir to store pdf for tabula...");
		let temp_dir_path = util::make_temp_dir();
		let temp_file_name = util::get_random_name();
		debug!("Created temp dir for the pdf!");

		debug!("Writing pdf to temp file...");
		let temp_file_path = format!("{}/{}", temp_dir_path, temp_file_name);
		let temp_file_path = Path::new(&temp_file_path);
		let mut temp_file = std::fs::File::create(temp_file_path).expect("Couldn't create temp pdf file");
		temp_file.write_all(&pdf).expect("Couldn't write pdf");
		debug!("Wrote pdf!");

		debug!("Creating json with tabula...");
		let new_schedule = SubstitutionSchedule::from_pdf(temp_file_path)?;
		let json = serde_json::to_string(&new_schedule)?;
		debug!("Created json!");

		debug!("Spawning database update and pdf save task.");
		tokio::spawn(async move {
			let pdf_date = &new_schedule.pdf_issue_date / 1000; // Its in milliseconds but we need seconds.
			let pdf_date = NaiveDateTime::from_timestamp(pdf_date, 0);
			let json_value = serde_json::to_value(new_schedule).unwrap();

			update_db(&hash, pdf_date, json_value, pool).await;
		});

		{
			let mut json_store = self.jsons.write().await;

			info!("Adding new json for {day} to the json map.");
			let old = json_store.insert(day, json);

			if old.is_some() {
				trace!("An old json was replaced");
			}
		}

		info!("Removing temp pdf file and accompanying temp directory.");
		std::fs::remove_file(temp_file_path)?;
		std::fs::remove_dir(temp_dir_path)?;

		Ok(())
	}

	/// Gets a json from the internal json store.
	pub async fn get_json(&self, day: Schoolday) -> Option<String> {
		let jsons = self.jsons.read().await;
		jsons.get(&day).map(std::clone::Clone::clone)
	}
}

/// Inserts the json into the db.
async fn update_db(hash: &String, pdf_date: NaiveDateTime, json: serde_json::Value, pool: PgPool) {
	let insertion_time = Utc::now();
	let insertion_time = insertion_time.naive_utc();

	let query_result = sqlx::query!(
		r#"
		INSERT INTO substitution_json
		VALUES($1, $2, $3, $4)
		"#,
		hash,
		pdf_date,
		insertion_time,
		json
	)
		.execute(&pool)
		.await;

	if let Err(why) = query_result {
		error!("{why}");
	}
}

/// Saves the PDF to disk
async fn save_pdf_to_disk(day: Schoolday, pdf: &[u8]) -> Result<(), Err>{
	let today = get_today_string();
	let location = format!("{PDF_STORE_LOCATION}/{today}");
	fs::create_dir(location)?;

	let mut hasher = Sha512::new();
	Digest::update(&mut hasher, &pdf);
	let hash_bytes = hasher.finalize();
	let hash = hex::encode(hash_bytes);

	let file_location = format!("{location}/{hash}");

	Ok(())
}
