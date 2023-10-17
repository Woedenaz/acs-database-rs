mod backlinks;
mod sort_json;

use crate::sort_json::SortableField;
use anyhow::{anyhow, Result};
use clap::Parser;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use log::error;
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::{
	borrow::Cow,
	clone::Clone,
	collections::HashMap,
	fs::File,
	sync::{
		atomic::{AtomicU64, Ordering},
		Arc,
	},
};
use tokio::{fs, sync::Semaphore, time::Duration};

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
	#[arg(long, default_value_t = 1)]
	start: u16,

	#[arg(long, default_value_t = 7999)]
	end: u16,

	#[arg(short, long, default_value_t = 10)]
	limit: u16,

	#[arg(short, long, default_value_t = 5)]
	retries: u16,

	#[clap(short, long)]
	backlinks: bool,

	#[clap(short, long)]
	cross: bool,

	#[clap(short, long)]
	getnames: bool,

	#[clap(short, long)]
	scraper: bool,
}

#[derive(Serialize, Deserialize, Debug)]
struct SCPInfo {
	number: String,
	name: String,
}

impl SortableField for SCPInfo {
	fn get_field(&self, field: &str) -> Cow<str> {
		match field {
			"number" => Cow::Borrowed(&self.number),
			"name" => Cow::Borrowed(&self.name),
			_ => panic!("Invalid field: {}", field),
		}
	}
}

#[derive(Serialize, Deserialize, Debug)]
struct Range {
	start: u16,
	end: u16,
}

#[derive(Serialize, Deserialize, Debug)]
struct SharedAcs {	
	contain: String,
	secondary: String,
	disrupt: String,
	scraper: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
enum Acs {
	Vanilla {
		#[serde(flatten)]
		shared: SharedAcs,

		name: String,
		number: String,		
		clearance: String,
		clearance_text: String,
		risk: String,
		url: String,
		fragment: bool,
	},
	Bar {
		#[serde(flatten)]
		shared: SharedAcs,

		clearance: String,
		clearance_text: String,
		risk: String,
	},
	Flops {
		#[serde(flatten)]
		shared: SharedAcs,

		clearance: String,
		clearance_text: String,
	},
	Aim {
		#[serde(flatten)]
		shared: SharedAcs,

		clearance: String,
	},
	Backup {
		#[serde(flatten)]
		shared: SharedAcs,
		
		risk: String,
	},
}

impl SharedAcs {
	fn get_shared_field(&self, field: &str) -> Option<Cow<str>> {
		match field {
			"contain" => Some(Cow::Borrowed(&self.contain)),
			"secondary" => Some(Cow::Borrowed(&self.secondary)),
			"disrupt" => Some(Cow::Borrowed(&self.disrupt)),
			"scraper" => Some(Cow::Borrowed(&self.scraper)),
			_ => None,
		}
	}
}

impl SortableField for Acs {
	fn get_field(&self, field: &str) -> Cow<str> {
		match self {
			Acs::Vanilla { shared, name, number, clearance, clearance_text, risk, url, fragment, .. } => {
				if let Some(shared_field) = shared.get_shared_field(field) {
					return shared_field;
				}
				match field {
					"name" => Cow::Borrowed(name),
					"number" => Cow::Borrowed(number),
					"clearance" => Cow::Borrowed(clearance),
					"clearance_text" => Cow::Borrowed(clearance_text),
					"risk" => Cow::Borrowed(risk),
					"url" => Cow::Borrowed(url),
					"fragment" => Cow::Owned(fragment.to_string()),
					_ => panic!("Invalid field: {}", field),
				}
			},
			Acs::Bar { shared, clearance, clearance_text, risk, .. } => {
				if let Some(shared_field) = shared.get_shared_field(field) {
					return shared_field;
				}
				match field {
					"clearance" => Cow::Borrowed(clearance),
					"clearance_text" => Cow::Borrowed(clearance_text),
					"risk" => Cow::Borrowed(risk),
					_ => panic!("Invalid field: {}", field),
				}
			},
			Acs::Flops { shared, clearance, clearance_text, .. } => {
				if let Some(shared_field) = shared.get_shared_field(field) {
					return shared_field;
				}
				match field {
					"clearance" => Cow::Borrowed(clearance),
					"clearance_text" => Cow::Borrowed(clearance_text),
					_ => panic!("Invalid field: {}", field),
				}
			},
			Acs::Aim { shared, clearance, .. } => {
				if let Some(shared_field) = shared.get_shared_field(field) {
					return shared_field;
				}
				match field {
					"clearance" => Cow::Borrowed(clearance),
					_ => panic!("Invalid field: {}", field),
				}
			},
			Acs::Backup { shared, risk, .. } => {
				if let Some(shared_field) = shared.get_shared_field(field) {
					return shared_field;
				}
				match field {
					"risk" => Cow::Borrowed(risk),
					_ => panic!("Invalid field: {}", field),
				}
			},
		}
	}
}

#[derive(Serialize, Deserialize, Debug)]
struct BacklinksInfo {
	fragment: bool,
	name: String,
	number: String,
	url: String,
}

// SCP Names Selectors
static LI_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("[id*='toc'] + ul li").unwrap());
static LINK_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("a:not(.newpage)").unwrap());
static SCP_NUM_RE: Lazy<Regex> =
	Lazy::new(|| Regex::new(r"(?i)scp-([0-9]{1,4})").unwrap());

// ACS Bar Selectors
static ACS_BAR_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.anom-bar-container").unwrap());
static ACS_LITE_BAR_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.anom-lite-bar-container").unwrap());
static CLEARANCE_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.top-right-box > div.level").unwrap());
static CLEARANCE_TEXT_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.top-right-box > div.clearance").unwrap());
static CONTAIN_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.contain-class > div.class-text").unwrap());
static SECONDARY_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.second-class > div.class-text").unwrap());
static DISRUPT_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.disrupt-class > div.class-text").unwrap());
static RISK_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.risk-class > div.class-text").unwrap());

// ACS Hybrid Bar Selectors
static ACS_HYBRID_BAR_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.acs-hybrid-text-bar").unwrap());
static HYBRID_CLEARANCE_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.acs-clear > strong").unwrap());
static HYBRID_CLEARANCE_TEXT_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.acs-clear > span.clearance-level-text").unwrap());
static HYBRID_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse("div.acs-contain > div.acs-text > span:nth-of-type(2)").unwrap()
});
static HYBRID_SECONDARY_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse("div.acs-secondary > div.acs-text > span:nth-of-type(2)").unwrap()
});
static HYBRID_DISRUPT_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.acs-disrupt > div.acs-text").unwrap());
static HYBRID_RISK_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.acs-risk > div.acs-text").unwrap());

// Flops Header Selectors
static FLOPS_HEADER_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse(".itemInfo.darkbox").unwrap());
static FLOPS_CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse(".itemInfo.darkbox > tbody:nth-child(1) > tr:nth-child(1) > td:nth-child(2) > span:nth-child(1)").unwrap()
});
static FLOPS_CLEARANCE_TEXT_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse(".itemInfo.darkbox > tbody:nth-child(1) > tr:nth-child(2) > td:nth-child(2) > span:nth-child(1)").unwrap()
});
static FLOPS_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse(
		".itemInfo.darkbox > tbody:nth-child(1) > tr:nth-child(2) > td:nth-child(1)",
	)
	.unwrap()
});
static FLOPS_DISRUPT_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse(".itemInfo.darkbox + p > a.disruptionHeader").unwrap());

// AIM Header Selectors
static AIM_HEADER_SELECTOR: Lazy<Selector> =
	Lazy::new(|| Selector::parse("div.desktop-aim").unwrap());
static AIM_CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse(
		"div.desktop-aim > div.w-container > div > div:nth-child(2) > p > span > span",
	)
	.unwrap()
});
static AIM_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse("div.desktop-aim > div.w-container > div > div:nth-child(3) > p")
		.unwrap()
});
static AIM_DISRUPT_SELECTOR: Lazy<Selector> = Lazy::new(|| {
	Selector::parse("div.desktop-aim > div.w-container > div > div:nth-child(4) > p")
		.unwrap()
});

const SERIES_URLS: [&str; 8] = [
	"https://scp-wiki.wikidot.com/scp-series",
	"https://scp-wiki.wikidot.com/scp-series-2",
	"https://scp-wiki.wikidot.com/scp-series-3",
	"https://scp-wiki.wikidot.com/scp-series-4",
	"https://scp-wiki.wikidot.com/scp-series-5",
	"https://scp-wiki.wikidot.com/scp-series-6",
	"https://scp-wiki.wikidot.com/scp-series-7",
	"https://scp-wiki.wikidot.com/scp-series-8",
];

const MAX_LEVEL: u8 = 9;

//Helper Functions
fn extract_scp_number(s: &str) -> Option<u16> {
	let cap = SCP_NUM_RE.captures(s)?;

	let number = match cap[1].parse::<u16>() {
		Ok(num) => Some(num),
		Err(e) => {
			log::error!("Failed to parse SCP number {}: {}", s, e);
			None
		}
	}?;

	Some(number)
}

fn format_number(number: u16) -> String {
	if number <= 99 {
		format!("SCP-{:03}", number)
	} else if number > 99 {
		format!("SCP-{}", number)
	} else {
		number.to_string()
	}
}

fn extract_text(element: ElementRef, selector: &Selector) -> Option<String> {
	element
		.select(selector)
		.next()?
		.text()
		.collect::<Vec<_>>()
		.join("")
		.trim()
		.to_string()
		.into()
}

fn extract_class(element: ElementRef, selector: &Selector) -> Option<String> {
	element
		.select(selector)
		.next()?
		.value()
		.attr("class")
		.map(|s| s.to_string())
}

fn is_valid_containment_class(class: &str) -> bool {
	[
		"safe",
		"euclid",
		"keter",
		"neutralized",
		"pending",
		"explained",
		"esoteric",
	]
	.iter()
	.any(|&valid_class| class.eq_ignore_ascii_case(valid_class))
}

fn extract_string_after_colon(text: &str) -> String {
	text.split_once(':')
		.map(|(_, rest)| rest.split_once('\n').unwrap_or((rest, "")).0.trim_start())
		.unwrap_or("")
		.to_string()
}

#[test]
fn test_extract_string_after_colon() {
	const TEST_STRINGS: [(&str, &str); 8] = [
		("containment class: keter\n", "keter"),
		("disruption class: amida\n", "amida"),
		("risk class: critical\n", "critical"),
		("secondary class: foo bar\n", "foo bar"),
		("containment class: {field left blank}\\n", "{field left blank}"),
		("no colon here\n", ""),
		("", ""),
		("containment class: ", "")
	];

	for (input, expected) in TEST_STRINGS {
		let result = extract_string_after_colon(input);
		assert_eq!(
			result.as_str(),
			expected,
			"Expected '{}' for input '{}', but got '{}'",
			expected,
			input,
			result
		);
	}
}

fn clean_text(text: &str) -> String {
	let text = text.trim().to_string();
	if text.contains("{$") || text.eq_ignore_ascii_case("none") {
		return String::new();
	}
	if text.contains(':') {
		return extract_string_after_colon(&text);
	}
	if !(text.contains("n/a") || text.contains("N/A")) && text.contains('/') {
		return text.split_once('/').map(|(_, rest)| rest).unwrap_or("").to_string();
	} else {
		text
	}
}

#[test]
fn test_clean_text() {
	const TEST_STRINGS: [(&str, &str); 6] = [
		("Containment Class: Keter", "Keter"),
		("Secondary Class: {FIELD LEFT BLANK}", "{FIELD LEFT BLANK}"),
		("{$secondary-class}", ""),
		("2/Vlam", "Vlam"),
		("None", ""),
		("N/A", "N/A"),
	];

	for (input, expected) in TEST_STRINGS {
		let result = clean_text(input);
		assert_eq!(
			result.as_str(),
			expected,
			"Expected '{}' for input '{}', but got '{}'",
			expected,
			input,
			result
		);
	}
}

fn create_acs(acs: Acs) -> Result<Acs> {
	match acs {
		Acs::Vanilla {
			shared,
			name,
			number,
			clearance,
			clearance_text,
			risk,
			url,
			fragment,
			..
		} => {
			let mut number = number.clone();
			if name.to_lowercase().contains("scp-") {
				number = name.clone();
			}

			Ok(Acs::Vanilla {
				shared: SharedAcs {
					contain: clean_text(&shared.contain),
					secondary: clean_text(&shared.secondary),
					disrupt: clean_text(&shared.disrupt),
					scraper: clean_text(&shared.scraper),
				},
				name,
				number,
				clearance: clean_text(&clearance),
				clearance_text: clean_text(&clearance_text),
				risk: clean_text(&risk),
				url,
				fragment,
			})
		}
		_ => {
			log::error!("The provided Acs data is not of variant Vanilla: {:?}", acs);
			Err(anyhow::anyhow!("The provided Acs data is not of variant Vanilla"))
		}
	}	
}

//Helper Async Functions
async fn request_page(url: &str) -> Result<Option<Html>> {
	let client = reqwest::Client::new();
	let response = client
		.get(url)
		.header(reqwest::header::USER_AGENT, "reqwest/0.11.20 (rust)")
		.send()
		.await?;

	log::debug!("Received status {} from {}", response.status(), url);

	if response.status() == reqwest::StatusCode::NOT_FOUND {
		return Ok(None);
	} else if !response.status().is_success() {
		return Err(anyhow!(
			"Failed to fetch URL: {} - Status: {}",
			url,
			response.status()
		));
	}

	let body = response.text().await?;
	Ok(Some(Html::parse_document(&body)))
}

async fn write_json<T: Serialize>(data: &[T], path: &str) -> Result<()> {
	let file = File::create(path)?;
	serde_json::to_writer_pretty(file, &data)?;

	Ok(())
}

// Scrape SCP Series Pages -> Get SCP Names -> Write them to json File
async fn init_scp_names_json() -> Result<()> {
	let mut scp_names_vec: Vec<SCPInfo> = Vec::new();

	let progress_bar_scp_names = ProgressBar::new_spinner();
	progress_bar_scp_names.set_style(
		ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} {pos:>7}")
			.expect("Failed to set progress bar style.")
			.progress_chars("=> "),
	);
	progress_bar_scp_names.set_message("Initializing SCP Info");

	for series_url in SERIES_URLS.iter() {
		let document_option = request_page(series_url).await?;
		if let Some(document) = document_option {
			let lis = document.select(&LI_SELECTOR);

			for li in lis {
				if let Some(link) = li.select(&LINK_SELECTOR).next() {
					let link_url = link.value().attr("href").unwrap_or("");

					let scp_string = if link_url.to_lowercase().contains("scp-") {
						link_url.to_string()
					} else if link.inner_html().to_lowercase().contains("scp-") {
						link.inner_html()
					} else {
						String::new()
					};

					let name_html: String = li.inner_html();
					let name_parts: Vec<&str> = name_html.split(" - ").collect();
					let name: String = if name_parts.len() > 1 {
						name_parts[1].to_string()
					} else {
						String::new()
					};

					if let Some(scp_number) = extract_scp_number(&scp_string) {
						let number = format_number(scp_number);

						scp_names_vec.push(SCPInfo { number, name });

						progress_bar_scp_names.inc(1);
					}
				}
			}
		} else {
			log::error!("Page not found: {}", series_url);
		}
	}

	sort_json::sort(&mut scp_names_vec, "number");
	write_json(&scp_names_vec, "output/acs_database.json").await?;

	progress_bar_scp_names.finish_with_message("SCP Info Initialized");
	Ok(())
}

// Get SCP Name from SCP Names json based on Number
async fn get_scp_name(number: &str) -> Result<String> {
	let json_data = fs::read_to_string("output/scp_names.json").await?;
	let scp_names_vec: Vec<SCPInfo> = serde_json::from_str(&json_data)?;

	let scp_name = scp_names_vec
		.iter()
		.find(|&scp| scp.number == number)
		.map(|scp| scp.name.to_owned())
		.unwrap_or_else(|| number.to_string());

	Ok(scp_name)
}

// Text Strings scraping if ACS Bar is not found.
// Searches the pages for specific phrases/words and adds them to the database if found
async fn backup_acs_function(
	document: &Html,
) -> Option<Acs> {
	let text = document
		.root_element()
		.text()
		.collect::<String>()
		.to_lowercase();
	
	let mut results = HashMap::new();

	let keywords = [
		("containment class:", "contain"),
		("disruption class:", "disrupt"),
		("risk class:", "risk"),
		("secondary class:", "secondary"),
	];

	for &(search_str, result_key) in &keywords {
		if let Some(index) = text.find(search_str) {            
			let result_value = extract_string_after_colon(&text[index..]);            
			results.insert(result_key.to_string(), result_value);            
		}
	}

	for &keyword in &[" vlam ", " keneq ", " ekhi ", " amida "] {
		if text.contains(keyword) {
			results.insert("disrupt".to_string(), keyword.trim().to_string());
			break;
		}
	}

	match (
		results.get("contain"),
		results.get("disrupt"),
		results.get("risk"),
		results.get("secondary"),
	) {
		(Some(contain), Some(disrupt), Some(risk), Some(secondary)) => Some(
			Acs::Backup {
				shared: SharedAcs {
					contain: clean_text(contain),
					secondary: clean_text(secondary),
					disrupt: clean_text(disrupt),
					scraper: "Brute Force".to_string(),
				},
				risk: clean_text(risk),
			}
		),
		_ => None,
	}
}

// ACS Bar Scraper
// ACS Bar Scraper
async fn get_acs_bar(
	document: &Html,
) -> Acs {
	let mut clearance = clean_text(
		&extract_text(document.root_element(), &CLEARANCE_SELECTOR).unwrap_or_default(),
	);
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let clearance_text = clean_text(
		&extract_text(document.root_element(), &CLEARANCE_TEXT_SELECTOR)
			.unwrap_or_default(),
	);
	let contain = clean_text(
		&extract_text(document.root_element(), &CONTAIN_SELECTOR).unwrap_or_default(),
	);
	let secondary = clean_text(
		&extract_text(document.root_element(), &SECONDARY_SELECTOR).unwrap_or_default(),
	);
	let disrupt = clean_text(
		&extract_text(document.root_element(), &DISRUPT_SELECTOR).unwrap_or_default(),
	);
	let risk = clean_text(
		&extract_text(document.root_element(), &RISK_SELECTOR).unwrap_or_default(),
	);

	Acs::Bar {
		shared: SharedAcs {
			contain,
			secondary,
			disrupt,
			scraper: "ACS Bar".to_string(),
		},
		clearance,
		clearance_text,
		risk,
	}
}

// ACS Hybrid Bar Scraper
async fn get_acs_hybrid_bar(
	document: &Html,
) -> Acs {
	let mut clearance = extract_text(document.root_element(), &HYBRID_CLEARANCE_SELECTOR)
		.unwrap_or_default();
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let clearance_text = {
		let temp = clean_text(
			&extract_text(document.root_element(), &HYBRID_CLEARANCE_TEXT_SELECTOR)
				.unwrap_or_default(),
		);
		if temp.eq_ignore_ascii_case("Clearance") {
			String::new()
		} else {
			temp
		}
	};
	let contain = clean_text(
		&extract_text(document.root_element(), &HYBRID_CONTAIN_SELECTOR)
			.unwrap_or_default(),
	);
	let secondary = clean_text(
		&extract_text(document.root_element(), &HYBRID_SECONDARY_SELECTOR)
			.unwrap_or_default(),
	);
	let disrupt = clean_text(
		&extract_text(document.root_element(), &HYBRID_DISRUPT_SELECTOR)
			.unwrap_or_default(),
	);
	let risk = clean_text(
		&extract_text(document.root_element(), &HYBRID_RISK_SELECTOR).unwrap_or_default(),
	);

	Acs::Bar {
		shared: SharedAcs {
			contain,
			secondary,
			disrupt,
			scraper: "ACS Hybrid Bar".to_string(),
		},
		clearance,
		clearance_text,
		risk,
	}
}

// Flops Header Scraper
async fn get_flops_header(
	document: &Html,
) -> Acs {
	let mut clearance = clean_text(
		&extract_text(document.root_element(), &FLOPS_CLEARANCE_SELECTOR)
			.unwrap_or_default(),
	);
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let clearance_text = clean_text(
		&extract_text(document.root_element(), &FLOPS_CLEARANCE_TEXT_SELECTOR)
			.unwrap_or_default(),
	);
	let mut contain = clean_text(
		&extract_text(document.root_element(), &FLOPS_CONTAIN_SELECTOR)
			.unwrap_or_default(),
	);
	let mut secondary = String::new();

	if !is_valid_containment_class(&contain) {
		secondary = contain;
		contain = "esoteric".to_string();
	}
	let disrupt = clean_text(
		&extract_text(document.root_element(), &FLOPS_DISRUPT_SELECTOR)
			.unwrap_or_default(),
	);

	Acs::Flops {
		shared: SharedAcs {
			contain,
			secondary,
			disrupt,
			scraper: "Flops Header".to_string(),
		},
		clearance,
		clearance_text,
	}
}

// AIM Header Scraper
async fn get_aim_header(document: &Html) -> Acs {
	let clearance_item = extract_class(document.root_element(), &AIM_CLEARANCE_SELECTOR)
		.unwrap_or_default();
	let clearance = match clearance_item.as_str() {
		"one" => "LEVEL 1",
		"two" => "LEVEL 2",
		"three" => "LEVEL 3",
		"four" => "LEVEL 4",
		"five" => "LEVEL 5",
		"six" => "LEVEL 6",
		_ => "",
	}
	.to_string();
	let mut contain = clean_text(
		&extract_text(document.root_element(), &AIM_CONTAIN_SELECTOR).unwrap_or_default(),
	);
	let mut secondary = String::new();

	if !is_valid_containment_class(&contain) {
		secondary = contain;
		contain = "esoteric".to_string();
	}
	let disrupt = clean_text(
		&extract_text(document.root_element(), &AIM_DISRUPT_SELECTOR).unwrap_or_default(),
	);

	Acs::Aim {
		shared: SharedAcs {
			contain,
			secondary,
			disrupt,
			scraper: "AIM Header".to_string(),
		},
		clearance,
	}
}

// Searches the page for the ACS Bar & ACS Hybrid Bar
// If found, selects and scrapes specific elements
// If not found, resorts to Text Strings scraping
async fn fetch_acs_data(
	number: &str,
	mut name: Option<&str>,
	url: &str,
	fragment: &bool
) -> Result<Option<Acs>> {
	log::debug!("Fetching data from: {}", url);
	let document = request_page(url).await?;

	fn convert_to_vanilla(acs: Acs, name: &str, number: &str, url: &str, fragment: &bool) -> Acs {
		match acs {
			Acs::Vanilla { .. } => acs,
			Acs::Bar { shared, clearance, clearance_text, risk } => Acs::Vanilla {
				shared,
				name: name.to_string(),
				number: number.to_string(),
				clearance,
				clearance_text,
				risk,
				url: url.to_string(),
				fragment: *fragment,
			},
			Acs::Flops { shared, clearance, clearance_text } => Acs::Vanilla {
				shared,
				name: name.to_string(),
				number: number.to_string(),
				clearance,
				clearance_text,
				risk: String::new(),
				url: url.to_string(),
				fragment: *fragment,
			},
			Acs::Aim { shared, clearance } => Acs::Vanilla {
				shared,
				name: name.to_string(),
				number: number.to_string(),
				clearance,
				clearance_text: String::new(),
				risk: String::new(),
				url: url.to_string(),
				fragment: *fragment,
			},
			Acs::Backup { shared, risk } => Acs::Vanilla {
				shared,
				name: name.to_string(),
				number: number.to_string(),
				clearance: String::new(),
				clearance_text: String::new(),
				risk,
				url: url.to_string(),
				fragment: *fragment,
			},			
		}
	}

	if let Some(document) = document {
		let has_anom_bar = document.select(&ACS_BAR_SELECTOR).next().is_some();
		let has_lite_anom_bar = document.select(&ACS_LITE_BAR_SELECTOR).next().is_some();
		let has_hybrid_anom_bar = document.select(&ACS_HYBRID_BAR_SELECTOR).next().is_some();
		let has_flops_header = document.select(&FLOPS_HEADER_SELECTOR).next().is_some();
		let has_aim_header = document.select(&AIM_HEADER_SELECTOR).next().is_some();

		let name_string: String;

		if !number.eq_ignore_ascii_case("scp-000")
			&& !number.eq_ignore_ascii_case("scp-001")
			&& ( name.map_or(true, |n| n.is_empty()) 
				&& ( number.contains("scp-") || number.contains("SCP-") ) 
			)
		{
			name_string = get_scp_name(number).await?;
    	name = Some(&name_string);	
		}

		let acs_data: Acs;

		if has_anom_bar || has_lite_anom_bar {
			acs_data = get_acs_bar(&document).await;
		} else if has_hybrid_anom_bar {
			acs_data = get_acs_hybrid_bar(&document).await;
		} else if has_flops_header {
			acs_data = get_flops_header(&document).await;
		} else if has_aim_header {
			acs_data = get_aim_header(&document).await;
		} else {
			match backup_acs_function(&document).await {
				Some(Acs::Backup { shared, risk }) => {
					acs_data = Acs::Backup { shared, risk };
				}
				_ => {
					log::debug!("No data retrieved using backup_acs_function for URL: {}", url);
					return Ok(None);
				}
			};
		}

		let vanilla_acs = convert_to_vanilla(acs_data, name.unwrap_or(""), number, url, fragment);

		match create_acs(vanilla_acs) {
			Ok(acs_data) => Ok(Some(acs_data)),
			Err(e) => Err(e),
		}
	} else {
		log::error!("Page not found: {}", url);
		Ok(None)
	}
}

// Compare ACS Backlinks and add to Database if not included
async fn fetch_and_update_entry(
	number: &str,
	name: &str,
	url: &str,
	fragment: bool,
) -> Result<Acs> {
	log::debug!("Fetching data from: {}", url);
	match fetch_acs_data(number, Some(name), url, &fragment).await {
		Ok(Some(acs_data)) => {
			match acs_data {
				Acs::Vanilla {
					shared,
					name,
					number,
					clearance,
					clearance_text,
					risk,
					url,
					fragment,
				} => {
					log::debug!("Successfully fetched ACS Bar Data from: {}", url);
					match create_acs(Acs::Vanilla {
						shared: SharedAcs {
							contain: shared.contain,
							secondary: shared.secondary,
							disrupt: shared.disrupt,
							scraper: shared.scraper,
						},
						name: name.to_string(),
						number: number.to_string(),
						clearance: clearance,
						clearance_text: clearance_text,            
						risk: risk,
						url: url.to_string(),
						fragment: fragment,
					}) {
						Ok(new_entry) => Ok(new_entry),
						Err(e) => Err(anyhow::anyhow!("Failed to create acs: {}", e)),
					}
				},
				_ => Err(anyhow!("The provided Acs data is not of variant Vanilla.")),
			}
		}

		Ok(None) => Err(anyhow!(
			"f: fetch_and_update_entry | Failed to fetch ACS data for: {}",
			url
		)),
		Err(e) => Err(anyhow!(
			"f: fetch_and_update_entry | Error fetching ACS data for {}: {}",
			url,
			e
		)),
	}
}

async fn cross_compare_and_update(limit: u16) -> Result<()> {
	let acs_bar_backlinks_data = fs::read_to_string("output/acs_backlinks.json").await?;
	let acs_database_data = fs::read_to_string("output/acs_database.json").await?;

	let acs_bar_backlinks: Vec<BacklinksInfo> = serde_json::from_str(&acs_bar_backlinks_data)?;
	let mut acs_database: Vec<Acs> = serde_json::from_str(&acs_database_data)?;

	let semaphore = Arc::new(Semaphore::new(limit.into()));
	let matches = Arc::new(AtomicU64::new(0));

	let total_entries = acs_bar_backlinks.len() as u64;
	let pb = ProgressBar::new(total_entries);
	pb.set_style(
		ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta_precise})")
			.expect("Failed to set progress bar style.")
			.progress_chars("##-")
	);

	let new_entries_futures: FuturesUnordered<_> = acs_bar_backlinks
		.into_iter()
		.filter_map(|link_item| {
			let backlinks_number = &link_item.number;
			let backlinks_name = &link_item.name;

			if acs_database.iter().any(|db_item| {
				match db_item {
					Acs::Vanilla{number, name, fragment, ..} => {
						number.eq_ignore_ascii_case(&backlinks_number) 
							|| name.eq_ignore_ascii_case(&backlinks_name)
							|| *fragment
					}
					_ => false,
				}
			}) {
				None
			} else {
				Some(link_item)
			}
		})
		.map(|link_item| {
			let semaphore = Arc::clone(&semaphore);
			let matches = Arc::clone(&matches);
			let pb = pb.clone();

			Box::pin(async move {
				let _permit = semaphore
					.acquire()
					.await
					.expect("Failed to acquire semaphore");

				let number = &link_item.number;
				let name = &link_item.name;
				let url = &link_item.url;
				let fragment = link_item.fragment;

				match fetch_and_update_entry(number, name, url, fragment).await {
					Ok(data) => {
						matches.fetch_add(1, Ordering::Relaxed);
						pb.set_message(format!(
						"Cross comparing ACS Bar Backlinks to ACS Database - Matches: {}",
						matches.load(Ordering::Relaxed)
					));
						Some(data)
					}
					Err(e) => {
						error!(
							"f: cross_compare_and_update | Error fetching ACS data for {}",
							format!("{:?}: {:?}", link_item, e)
					);
						None
					}
				}
			})
		})
		.collect();

	let new_entries: Vec<Acs> = new_entries_futures
		.collect::<Vec<Option<Acs>>>()
		.await
		.into_iter()
		.flatten() 
		.collect::<Vec<Acs>>();

	let finish_message = format!("Done! - Matches: {}", matches.load(Ordering::Relaxed));
	pb.finish_with_message(finish_message);

	acs_database.extend(new_entries);

	sort_json::sort(&mut acs_database, "number");
	write_json(&acs_database, "output/acs_database.json").await?;

	Ok(())
}

// Main Function
#[tokio::main]
async fn main() -> Result<()> {
	if pretty_env_logger::try_init().is_err() {
		log::warn!("Logger is already initialized.");
	}

	let args = Args::parse();
	let start = args.start;
	let end = args.end;
	let limit = args.limit;
	let range = Range { start, end };

	if args.getnames {
		init_scp_names_json().await?;
	}

	if args.backlinks {
		match tokio::task::spawn_blocking(backlinks::fetch_backlinks).await {
			Ok(Ok(_)) => log::info!("Completed fetch_backlinks successfully."),
			Ok(Err(e)) => log::error!("Error in fetch_backlinks: {:?}", e),
			Err(e) => log::error!("Task aborted due to panic: {:?}", e),
		}
	}

	if args.scraper {
		let total = range.end - range.start + 1;

		let progress_bar = ProgressBar::new_spinner();
		progress_bar.set_style(ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta_precise})")
			.expect("Failed to set progress bar style.")
			.progress_chars("##-")
		);
		progress_bar.set_message("Fetching ACS data");
		progress_bar.set_length(total.into());

		let semaphore = Arc::new(Semaphore::new(limit.into()));

		let mut acs_data: Vec<Acs> = (start..=end)
			.map(|number| {
				let scp_number = format_number(number);
				let scp_url_string =
					format!("https://scp-wiki.wikidot.com/{}", scp_number);
				let pb = progress_bar.clone();
				let semaphore = Arc::clone(&semaphore);
				Box::pin(async move {
					let _permit = semaphore
						.acquire()
						.await
						.map_err(|e| {
							error!(
								"Failed to acquire semaphore permit for {}: {}",
								scp_number, e
							);
							e
						})
						.ok()?;
					let mut retries = 0;
					let mut result =
						fetch_acs_data(&scp_number, None, &scp_url_string, &false).await;
					while result.is_err() && retries < args.retries.into() {
						retries += 1;
						tokio::time::sleep(Duration::from_secs(2 * retries)).await;
						result = fetch_acs_data(&scp_number, None, &scp_url_string, &false).await;
					}
					match result {
						Ok(Some(data)) => {
							pb.inc(1);
							tokio::time::sleep(Duration::from_millis(1000)).await;
							Some(data)
						}
						Ok(None) => {
							pb.inc(1);
							None
						}
						Err(e) => {
							error!(
								"f: main > scraper | Error fetching ACS data for {}: {}",
								scp_number, e
							);
							pb.inc(1);
							None
						}
					}
				})
			})
			.collect::<futures::stream::FuturesUnordered<_>>()
			.collect::<Vec<Option<Acs>>>()
			.await
			.into_iter()
			.flatten()
			.collect();

		progress_bar.finish_with_message("Done");

		sort_json::sort(&mut acs_data, "number");
		write_json(&acs_data, "output/acs_database.json").await?;
	}

	if args.cross {
		cross_compare_and_update(args.limit).await?;
	}

	Ok(())
}
