use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::process::Command;
use std::str;
use std::time::SystemTime;
use thiserror::Error;

use chrono::{Local, NaiveDate, Offset, Utc};
use lopdf::Document;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug};

/// One column with Substitutions from the PDF
#[derive(Serialize, Deserialize, PartialOrd, PartialEq, Debug)]
pub struct SubstitutionColumn {
	#[serde(rename(serialize = "0"))]
	#[serde(rename(deserialize = "0"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_0: Option<String>,
	#[serde(rename(serialize = "1"))]
	#[serde(rename(deserialize = "1"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_1: Option<String>,
	#[serde(rename(serialize = "2"))]
	#[serde(rename(deserialize = "2"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_2: Option<String>,
	#[serde(rename(serialize = "3"))]
	#[serde(rename(deserialize = "3"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_3: Option<String>,
	#[serde(rename(serialize = "4"))]
	#[serde(rename(deserialize = "4"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_4: Option<String>,
	#[serde(rename(serialize = "5"))]
	#[serde(rename(deserialize = "5"))]
	#[serde(skip_serializing_if = "Option::is_none")]
	pub block_5: Option<String>,
}

/// Represents a column from the substitution PDF.
/// Does not include the class name, only the substitutions.
impl SubstitutionColumn {
	pub fn new() -> Self {
		Self {
			block_0: None,
			block_1: None,
			block_2: None,
			block_3: None,
			block_4: None,
			block_5: None,
		}
	}
}

impl Default for SubstitutionColumn {
	fn default() -> Self {
		Self::new()
	}
}

impl Display for SubstitutionColumn {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", serde_json::to_string_pretty(self).unwrap())
	}
}

/// Contains the extracted PDF data of the schedule PDF
#[derive(Serialize, Deserialize, Debug)]
pub struct SubstitutionSchedule {
	/// The creation date inside the PDF in milliseconds.
	pub pdf_issue_date: i64,
	/// The name of the class is the Key and the Value is a Substitutions struct.
	entries: HashMap<String, SubstitutionColumn>,
	/// The time when the struct was created, used for comparing the age.
	struct_time: u64,
}

impl SubstitutionSchedule {
	/// Constructs an instance of `Self` from a document saved on disk.
	pub fn from_pdf<T: AsRef<Path> + AsRef<OsStr>>(path: T) -> Result<Self, Box<dyn std::error::Error>> {
		let pdf = match Document::load(&path) {
			Ok(pdf) => pdf,
			Err(_) => return Err(Box::new(PDFJsonError::PDFReadError)),
		};

		let page_numbers = get_all_page_numbers(&pdf);
		let pdf = pdf.extract_text(&*page_numbers)?;

		let date_idx_start = pdf.find("Datum: ").ok_or("date not found")?;
		let date_idx_end = pdf[date_idx_start..].find('\n').ok_or("date end not found")? + date_idx_start;

		let date_str: Vec<u32> = pdf[date_idx_start..date_idx_end].split(", ")
			.last()
			.ok_or("date string has no ','")?
			.split('.')
			.collect::<Vec<&str>>()
			.iter()
			.map(|s| (*s).parse::<u32>().unwrap())
			.collect();

		#[allow(clippy::cast_possible_wrap)]
			let date = chrono::Date::<Local>::from_utc(
			NaiveDate::from_ymd(date_str[2] as i32, date_str[1], date_str[0]),
			Utc.fix(),
		).and_hms_milli(0, 0, 0, 0).timestamp_millis();

		debug!("Calling tabula");
		let output = Command::new("java")
			.arg("-jar")
			.arg("./tabula/tabula.jar")
			.arg("-g")
			.arg("-f")
			.arg("JSON")
			.arg("-p")
			.arg("all")
			.arg(path)
			.output()?;

		debug!("Parsing tabulas json");
		let table = parse_tabula_json(str::from_utf8(&output.stdout).unwrap())?;

		Ok(Self::from_table(&table, date))
	}

	/// Constructs an instance of `Self` from a table.
	#[allow(clippy::ptr_arg)]
	pub fn from_table(tables: &Vec<Vec<Vec<String>>>, pdf_create_date: i64) -> Self {
		let mut entries = HashMap::new();

		for table in tables {
			entries.extend(Self::table_to_substitutions(table));
		}

		let time_now = SystemTime::now();
		let since_the_epoch = time_now
			.duration_since(SystemTime::UNIX_EPOCH)
			.expect("Time got fucked");

		#[allow(clippy::cast_possible_truncation)]
			let time_millis = since_the_epoch.as_millis() as u64;

		Self {
			pdf_issue_date: pdf_create_date,
			entries,
			struct_time: time_millis,
		}
	}

	/// Grabs the classes and their substitutions from a table and turns them into a HashMap.
	#[allow(clippy::ptr_arg)]
	fn table_to_substitutions(table: &Vec<Vec<String>>) -> HashMap<String, SubstitutionColumn> {
		let mut entries: HashMap<String, SubstitutionColumn> = HashMap::new();

		let classes = &table[0][1..];

		for class in classes {
			entries.insert(class.to_string(), SubstitutionColumn::new());
		}

		let mut row = 1;

		for lesson_idx in 0..5 {
			loop {
				for (i, substitution_part) in table[row][1..].iter().enumerate() {
					let substitutions = entries.get_mut(&classes[i]).unwrap();

					let block_option = match lesson_idx {
						0 => &mut substitutions.block_0,
						1 => &mut substitutions.block_1,
						2 => &mut substitutions.block_2,
						3 => &mut substitutions.block_3,
						4 => &mut substitutions.block_4,
						5 => &mut substitutions.block_5,
						_ => panic!("more then 5 lessons used"),
					};

					if !substitution_part.is_empty() {
						if let Some(block) = block_option {
							block.push_str(&format!("\n{}", substitution_part.clone()));
						} else {
							let _ = block_option.insert(substitution_part.clone());
						}
					}
				}

				if table[row][0].starts_with('-') {
					break;
				}
				row += 1;
			}

			row += 1;
		}

		entries
	}
}

/// Gets all pages from the pdf document.
fn get_all_page_numbers(pdf: &Document) -> Box<[u32]> {
	let pages = pdf
		.get_pages()
		.keys()
		.map(|page| *page)
		.collect::<Box<[u32]>>();

	pages
}

/// Extracts the text from the rows and cells in the json that gets outputted by tabula.
pub fn parse_tabula_json(content: &str) -> Result<Vec<Vec<Vec<String>>>, Box<dyn std::error::Error>> {
	let json: Value = serde_json::from_str(content)?;
	let array = json.as_array().ok_or("Json malformed")?;

	let mut tables = Vec::new();
	for entry in array {
		let object = entry.as_object().ok_or("Json malformed")?;
		let data = object.get("data").ok_or("Json data field missing")?;

		let mut table_rows = Vec::new();
		for row in data.as_array().ok_or("Json data missing")? {
			let row: Vec<Cell> = serde_json::from_value(row.clone())?;
			let row = Row {
				row
			};
			table_rows.push(row);
		}
		tables.push(table_rows);
	}

	let mut tables_with_rows_as_text = Vec::new();
	for table_rows in tables {
		let mut rows_as_text = Vec::new();
		for mut row in table_rows {
			rows_as_text.push(row.extract_text());
		}
		tables_with_rows_as_text.push(rows_as_text);
	}

	Ok(tables_with_rows_as_text)
}

/// A row in the substitution table
#[derive(Debug, Deserialize, Serialize)]
struct Row {
	row: Vec<Cell>,
}

impl Row {
	/// Gets the string content of every Cell inside the Row
	pub fn extract_text(&mut self) -> Vec<String> {
		let mut text = Vec::new();
		for cell in &self.row {
			text.push(cell.text.clone());
		}

		text
	}
}

/// A cell in the substitution table
#[derive(Debug, Deserialize, Serialize)]
struct Cell {
	top: f64,
	left: f64,
	width: f64,
	height: f64,
	text: String,
}

impl Display for Cell {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.text)
	}
}

#[derive(Error, Debug)]
pub enum PDFJsonError {
	#[error("There was an error while reading the PDF File.")]
	PDFReadError
}
