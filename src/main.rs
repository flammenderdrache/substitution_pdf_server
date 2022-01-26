use std::env;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use actix_cors::Cors;

use actix_web::{App, HttpServer};
use chrono::{Datelike, DateTime, Local, Weekday};
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing::{debug, error, info, trace};
use tracing_core::Level;
use tracing_subscriber::EnvFilter;

use crate::json_endpoint::get_schoolday_pdf_json;
use crate::json_handler::JsonHandler;

mod util;
mod json_endpoint;
mod json_handler;

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
	static ref JSON_HANDLER: JsonHandler = JsonHandler::new();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let env_filter = EnvFilter::from_default_env()
		.add_directive(Level::INFO.into())
		.add_directive("substitution_pdf_server=debug".parse()?)
		.add_directive("lopdf=error".parse()?);

	tracing_subscriber::fmt()
		.with_env_filter(env_filter)
		.with_line_number(true)
		.with_file(true)
		.init();

	info!("Connecting to the database...");
	let pool = PgPoolOptions::new()
		.max_lifetime(Duration::from_secs(60 * 60 * 12)) // 12 hours
		.min_connections(2)
		.max_connections(5)
		.connect(env::var("DATABASE_URL").expect("Couldn't find DB URL in env!").as_str())
		.await?;
	info!("Done!");

	info!("Migrating the database...");
	sqlx::migrate!()
		.run(&pool)
		.await?;
	info!("Done!");

	// Make sure the temp path exists
	std::fs::create_dir_all(TEMP_ROOT_DIR)?;

	tokio::spawn(async move {
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
			let pool_clone = pool.clone();
			tokio::spawn(async move {
				if let Err(why) = check_weekday_pdf(
					next_valid_school_weekday,
					pdf_getter_arc,
					pool_clone,
				).await {
					error!("{why}");
				}
			});

			let pdf_getter_arc = pdf_getter.clone();
			let pool_clone = pool.clone();
			tokio::spawn(async move {
				if let Err(why) = check_weekday_pdf(
					day_after,
					pdf_getter_arc,
					pool_clone,
				).await {
					error!("{}", why);
				}
			});

			counter += 1;
			debug!("Loop ran {counter} times and fetched {} PDFs", counter * 2);
			trace!("Loop end before sleep");
			tokio::time::sleep(PDF_GET_LOOP_SLEEP_TIME).await;
		}
	});

	info!("Starting actix server...");
	HttpServer::new(move || {
		// let json_config = web::JsonConfig::default()
		// 	.limit(4096);

		let cors = Cors::default()
			.allowed_methods(vec!["GET", "POST"])
			.allow_any_origin()
			.allow_any_header()
			.max_age(3600);

		App::new()
			.wrap(cors)
			.service(get_schoolday_pdf_json)
	})
		.bind("127.0.0.1:8081")?
		.run()
		.await?;

	Ok(())
}


/// Downloads the pdf of the current weekday, converts it to a json and adds it to the map of jsons.
#[allow(clippy::or_fun_call)]
async fn check_weekday_pdf(day: Schoolday, pdf_getter: Arc<SubstitutionPDFGetter<'_>>, pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {
	debug!("Getting pdf for {day}");
	let pdf = pdf_getter.get_weekday_pdf(day).await?;

	JSON_HANDLER.update(day, pdf, pool).await?;

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
