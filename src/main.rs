use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use actix_web::{App, HttpServer};
use chrono::{Datelike, DateTime, Local, Weekday};
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use substitution_pdf_to_json::SubstitutionSchedule;
use tokio::sync::RwLock;
use tracing::{debug, error, info, trace};
use tracing_core::Level;
use tracing_subscriber::EnvFilter;

use crate::json_endpoint::get_schoolday_pdf_json;

mod util;
mod json_endpoint;

const TEMP_ROOT_DIR: &str = "/tmp/school-substitution-scanner-temp-dir";
const SOURCE_URLS: [&str; 5] = [
	"https://buessing.schule/plaene/VertretungsplanA4_Montag.pdf",
	"https://buessing.schule/plaene/VertretungsplanA4_Dienstag.pdf",
	"https://buessing.schule/plaene/VertretungsplanA4_Mittwoch.pdf",
	"https://buessing.schule/plaene/VertretungsplanA4_Donnerstag.pdf",
	"https://buessing.schule/plaene/VertretungsplanA4_Freitag.pdf",
];
const PDF_GET_LOOP_SLEEP_TIME: Duration = Duration::from_secs(20);

lazy_static! {
	static ref PDF_JSON_STORE: RwLock<HashMap<Schoolday, String>> = RwLock::new(HashMap::new());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

	let env_filter = EnvFilter::from_default_env()
		.add_directive(Level::INFO.into())
		.add_directive("lopdf=error".parse()?);

	tracing_subscriber::fmt()
		.with_env_filter(env_filter)
		.with_line_number(true)
		.with_file(true)
		.init();


	// Make sure the temp path exists
	std::fs::create_dir_all(TEMP_ROOT_DIR)?;

	tokio::spawn(async {
		let pdf_getter = Arc::new(SubstitutionPDFGetter::default());
		let mut counter: u32 = 0;

		info!("Starting loop!");
		loop {
			trace!("loop started");

			let local: DateTime<Local> = Local::now();
			let next_valid_school_weekday = Schoolday::from(local.weekday());
			let day_after = next_valid_school_weekday.next_day();

			debug!("Local day: {}; next valid school day: {}; day after that: {}",
			local.weekday(),
			next_valid_school_weekday,
			day_after
		);

			let pdf_getter_arc = pdf_getter.clone();
			if let Err(why) = check_weekday_pdf(next_valid_school_weekday, pdf_getter_arc).await {
				error!("{}", why);
			}

			let pdf_getter_arc = pdf_getter.clone();
			tokio::spawn(async move {
				if let Err(why) = check_weekday_pdf(day_after, pdf_getter_arc).await {
					error!("{}", why);
				}
			});

			counter += 1;
			debug!("Loop ran {counter} times and fetched {} PDFs", counter * 2);
			trace!("Loop end before sleep");
			tokio::time::sleep(PDF_GET_LOOP_SLEEP_TIME).await;
		}
	});

	HttpServer::new(move || {
		// let json_config = web::JsonConfig::default()
		// 	.limit(4096);

		App::new()
			.service(get_schoolday_pdf_json)
	})
		.bind("127.0.0.1:8080")?
		.run()
		.await?;

	Ok(())
}


/// Downloads the pdf of the current weekday, converts it to a json and adds it to the map of jsons.
#[allow(clippy::or_fun_call)]
async fn check_weekday_pdf(day: Schoolday, pdf_getter: Arc<SubstitutionPDFGetter<'_>>) -> Result<(), Box<dyn std::error::Error>> {
	info!("Checking PDF for {}", day);
	let temp_dir_path = util::make_temp_dir();
	let temp_file_name = util::get_random_name();
	let temp_file_path = format!("{}/{}", temp_dir_path, temp_file_name);
	let temp_file_path = Path::new(&temp_file_path);

	let pdf = pdf_getter.get_weekday_pdf(day).await?;
	let mut temp_pdf_file = std::fs::File::create(temp_file_path).expect("Couldn't create temp pdf file");
	temp_pdf_file.write_all(&pdf)?;

	let new_schedule = SubstitutionSchedule::from_pdf(temp_file_path)?;
	let schedule_json = serde_json::to_string_pretty(&new_schedule).expect("Couldn't write the new Json");

	{
		let mut jsons = PDF_JSON_STORE.write().await;
		let _ = jsons.insert(day, schedule_json);
	}

	std::fs::remove_file(temp_file_path)?;
	std::fs::remove_dir(temp_dir_path)?;


	Ok(())
}

/// Enum with the weekdays where a Substitution PDF is available.
#[derive(Debug, PartialOrd, PartialEq, Clone, Copy, Hash, Eq, Serialize, Deserialize)]
pub enum Schoolday {
	Monday = 0,
	Tuesday = 1,
	Wednesday = 2,
	Thursday = 3,
	Friday = 4,
}

impl Schoolday {
	//It is not &self, just self here due to https://rust-lang.github.io/rust-clippy/master/index.html#trivially_copy_pass_by_ref
	//Thank clippy :p
	pub fn next_day(self) -> Self {
		match self {
			Schoolday::Monday => Schoolday::Tuesday,
			Schoolday::Tuesday => Schoolday::Wednesday,
			Schoolday::Wednesday => Schoolday::Thursday,
			Schoolday::Thursday => Schoolday::Friday,
			Schoolday::Friday => Schoolday::Monday,
		}
	}
}

impl Display for Schoolday {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		let self_as_string = match self {
			Schoolday::Monday => "Monday",
			Schoolday::Tuesday => "Tuesday",
			Schoolday::Wednesday => "Wednesday",
			Schoolday::Thursday => "Thursday",
			Schoolday::Friday => "Friday",
		};

		write!(f, "{}", self_as_string)
	}
}

impl From<Weekday> for Schoolday {
	fn from(day: Weekday) -> Self {
		match day {
			Weekday::Tue => Schoolday::Tuesday,
			Weekday::Wed => Schoolday::Wednesday,
			Weekday::Thu => Schoolday::Thursday,
			Weekday::Fri => Schoolday::Friday,
			_ => Schoolday::Monday,
		}
	}
}

#[derive(Debug)]
pub struct SubstitutionPDFGetter<'a> {
	urls: [&'a str; 5],
	client: Client,
}

impl<'a> SubstitutionPDFGetter<'a> {
	pub fn new(client: Client) -> Self {
		Self {
			urls: SOURCE_URLS,
			client,
		}
	}

	/// Returns result with an Err or a Vector with the binary data of the request-response
	/// Does not check if the response is valid, this is the responsibility of the caller.
	pub async fn get_weekday_pdf(&self, day: Schoolday) -> Result<Vec<u8>, reqwest::Error> {
		let url = self.urls[day as usize];
		let request = self.client
			.get(url)
			.header("Authorization", "Basic aGJzdXNlcjpoYnNwYXNz")
			.build()
			.unwrap();

		let response = self.client.execute(request).await?;
		let bytes = response.bytes().await?;

		Ok(bytes.to_vec())
	}
}

impl<'a> Default for SubstitutionPDFGetter<'a> {
	fn default() -> Self {
		let client = Client::builder()
			.connect_timeout(Duration::from_secs(20))
			.timeout(Duration::from_secs(20))
			.build()
			.unwrap();

		Self::new(client)
	}
}
