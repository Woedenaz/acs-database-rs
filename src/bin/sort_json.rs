use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fs::{File, OpenOptions}, io::{BufReader, BufWriter}};
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
	#[arg(long, required = true)]
	file: String,

	#[arg(long, default_value = "number")]
	field: String,
}

pub trait SortableField {
	fn get_field(&self, field: &str) -> Cow<str>;
}

#[derive(Serialize, Deserialize, Debug)]
struct ACS {
	name: String,
	number: String,
	clearance: String,
	contain: String,
	secondary: String,
	disrupt: String,
	risk: String,
	url: String,
	fragment: bool,
}

impl SortableField for ACS {
  fn get_field(&self, field: &str) -> Cow<str> {
    match field {
      "number" => Cow::Borrowed(&self.number),
      "name" => Cow::Borrowed(&self.name),
      "clearance" => Cow::Borrowed(&self.clearance),
      "contain" => Cow::Borrowed(&self.contain),
      "secondary" => Cow::Borrowed(&self.secondary),
      "disrupt" => Cow::Borrowed(&self.disrupt),
      "risk" => Cow::Borrowed(&self.risk),
      "url" => Cow::Borrowed(&self.url),
      "fragment" => Cow::Owned(self.fragment.to_string()),
      _ => panic!("Invalid field: {}", field),
    }
  }
}

fn extract_number(s: &str) -> Option<i32> {
	if s.starts_with("SCP-") {
		s[4..].parse::<i32>().ok()
	} else {
		None
	}
}

pub fn sort<T: SortableField>(mut entries: Vec<T>, sort_field: &str) -> Vec<T> {
	entries.sort_by(|a, b| {
		let a_field = a.get_field(sort_field);
		let b_field = b.get_field(sort_field);

		match (extract_number(&a_field), extract_number(&b_field)) {
			(Some(a_number), Some(b_number)) => a_number.cmp(&b_number),
			(Some(_), None) => std::cmp::Ordering::Less,
			(None, Some(_)) => std::cmp::Ordering::Greater,
			(None, None) => a_field.cmp(&b_field),
		}
	});

	entries
}

fn main() -> Result<(), anyhow::Error> {
	let args = Args::parse();
	let file_name = args.file.clone();

	let file = File::open(&args.file).expect("Unable to open file");
	let reader = BufReader::new(file);
	let entries: Vec<ACS> = serde_json::from_reader(reader).expect("Unable to parse JSON");

	let field = args.field;
	let sorted_entries = sort(entries, &field);

	let file = OpenOptions::new().write(true).open(&file_name).expect("Unable to open file for writing");
	let writer = BufWriter::new(file);
	serde_json::to_writer_pretty(writer, &sorted_entries)?;

	Ok(())
}
