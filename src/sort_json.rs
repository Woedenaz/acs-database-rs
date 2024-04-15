use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{from_reader, to_writer_pretty};
use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};

#[derive(Parser, Debug)]
#[clap(about = "Sort JSON entries", version = "1.0", author = "Your Name")]
struct Args {
	#[arg(long, required = true)]
	file: String,

	#[arg(long, default_value = "actual_number")]
	field: String,
}

pub trait SortableField {
	fn get_field(&self, field: &str) -> Cow<str>;
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Acs {
	name: String,
	actual_number: String,
	display_number: String,
	clearance: String,
	clearance_text: String,
	contain: String,
	secondary: String,
	disrupt: String,
	risk: String,
	url: String,
	fragment: bool,
	scraper: String,
}

impl SortableField for Acs {
	fn get_field(&self, field: &str) -> Cow<str> {
		match field {
			"actual_number" => Cow::Borrowed(&self.actual_number),
			"display_number" => Cow::Borrowed(&self.display_number),
			"name" => Cow::Borrowed(&self.name),
			"clearance" => Cow::Borrowed(&self.clearance),
			"clearance_text" => Cow::Borrowed(&self.clearance_text),
			"contain" => Cow::Borrowed(&self.contain),
			"secondary" => Cow::Borrowed(&self.secondary),
			"disrupt" => Cow::Borrowed(&self.disrupt),
			"risk" => Cow::Borrowed(&self.risk),
			"url" => Cow::Borrowed(&self.url),
			"fragment" => Cow::Owned(self.fragment.to_string()),
			"scraper" => Cow::Borrowed(&self.scraper),
			_ => panic!("Invalid field: {}", field),
		}
	}
}

fn extract_scp_number(s: &str) -> Option<u16> {
	if s.len() < 7 {
		return None;
	}

	let prefix_match = s
		.chars()
		.take(4)
		.zip("SCP-".chars())
		.all(|(a, b)| a.eq_ignore_ascii_case(&b));

	if prefix_match {
		s[4..]
			.chars()
			.take_while(|c| c.is_ascii_digit())
			.collect::<String>()
			.parse::<u16>()
			.ok()
	} else {
		None
	}
}

pub fn sort<T: SortableField>(entries: &mut [T], sort_field: &str) {
	entries.sort_by(|a, b| {
		let a_field = a.get_field(sort_field);
		let b_field = b.get_field(sort_field);

		match (extract_scp_number(&a_field), extract_scp_number(&b_field)) {
			(Some(a_number), Some(b_number)) => a_number.cmp(&b_number),
			(Some(_), None) => std::cmp::Ordering::Less,
			(None, Some(_)) => std::cmp::Ordering::Greater,
			(None, None) => a_field.cmp(&b_field),
		}
	});
}

#[cfg(not(test))]
#[allow(dead_code)]
fn main() {
	let args = Args::parse();

	let file = File::open(&args.file).expect("File not found");
	let reader = BufReader::new(file);
	let mut entries: Vec<Acs> = from_reader(reader).expect("Error reading json");

	sort(&mut entries, &args.field);

	let file = OpenOptions::new()
		.write(true)
		.truncate(true)
		.open(&args.file)
		.expect("Failed to open file for writing");

	let writer = BufWriter::new(file);

	to_writer_pretty(writer, &entries).expect("Error writing json");

	println!("File sorted and overwritten successfully.");
}
