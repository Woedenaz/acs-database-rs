use clap::Parser;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

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
struct Acs {
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
		let number = match s[4..].parse::<u16>() {
			Ok(num) => Some(num),
			Err(e) => {
				log::error!("Failed to parse SCP number {}: {}", s, e);				
				None
			}
		}?;
		Some(number)
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
