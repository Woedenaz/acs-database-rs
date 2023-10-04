use anyhow::{anyhow, Result};
use clap::Parser;
use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use log::error;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json;
use std::{clone::Clone, sync::Arc};
use tokio::{fs, time::Duration, sync::Semaphore};

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
	#[arg(short, long, default_value_t = 1)]
	start: u16,

	#[arg(short, long, default_value_t = 7999)]
	end: u16,

	#[arg(short, long, default_value_t = 10)]
	limit: u16,
}

#[derive(Serialize, Deserialize)]
struct SCPInfo {
	number: u16,
	name: String,
}

#[derive(Serialize, Deserialize)] 
struct Range {
	start: u16,
	end: u16 
}

#[derive(Serialize, Deserialize)]
struct ACS {
	name: String,
	number: String,
	clearance: String,
	contain: String,
	secondary: String,
	disrupt: String,
	risk: String,
	url: String,
}

#[derive(Clone)]
struct Selectors {
	acs_bar: Selector,
	clearance: Selector,
	contain: Selector,
	secondary: Selector,
	disrupt: Selector,
	risk: Selector,
	li_selector: Selector,
	a_selector: Selector,
}

impl Selectors {
	fn new() -> Result<Self> {
		Ok(Self {
			acs_bar: Selector::parse("div.anom-bar-container").map_err(|e| anyhow!(e.to_string()))?,
			clearance: Selector::parse("div.top-right-box > div.level").map_err(|e| anyhow!(e.to_string()))?,
			contain: Selector::parse("div.contain-class > div.class-text").map_err(|e| anyhow!(e.to_string()))?,
			secondary: Selector::parse("div.second-class > div.class-text").map_err(|e| anyhow!(e.to_string()))?,
			disrupt: Selector::parse("div.disrupt-class > div.class-text").map_err(|e| anyhow!(e.to_string()))?,
			risk: Selector::parse("div.risk-class > div.class-text").map_err(|e| anyhow!(e.to_string()))?,
			li_selector: Selector::parse("[id*='toc'] + ul li").map_err(|e| anyhow!(e.to_string()))?,
			a_selector: Selector::parse("a:not(.newpage)").map_err(|e| anyhow!(e.to_string()))?,
		})
	}
}

static SELECTORS: Lazy<Selectors> = Lazy::new(|| {
	Selectors::new().unwrap()	
});

static RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"scp-([0-9]{1,4})$").unwrap());

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

//Helper Functions
fn get_scp_number(url: &str) -> Option<u16> {
	let cap = &RE.captures(url)?;
	let number = &cap[1];
	number.parse::<u16>().ok()
}

fn get_text_from_selector(element: ElementRef, selector: &Selector) -> Option<String> {
	element.select(&selector)
		.next()?
		.text()
		.collect::<Vec<_>>()
		.join("")
		.trim()
		.to_string()
		.into()
}

fn clean_text(text: String) -> String {
	if text.contains("{$") || text.to_lowercase() == "none" {
		String::new()
	} else {
		text
	}
}

fn extract_word_after_colon(text: &str) -> String {
	text.splitn(2, ':')
		.nth(1)
		.and_then(|s| s.split_whitespace().next())
		.unwrap_or("")
		.to_string()
}


async fn write_json<T: Serialize>(data: &[T], path: &str) -> Result<()> {
	let json = serde_json::to_string(data)?;
	fs::write(path, json).await?;
	Ok(())
}

// Scrape SCP Series Pages -> Get SCP Names -> Write them to JSON File
async fn init_scp_info_json() ->  Result<()> {
	let mut scp_info_vec: Vec<SCPInfo> = Vec::new();

	let pb_scp_info = ProgressBar::new_spinner();
	pb_scp_info.set_style(ProgressStyle::default_bar()
		.template("{msg} {spinner:.green} {pos:>7}")
		.expect("Failed to set progress bar style.")
		.progress_chars("=> ")
	);
	pb_scp_info.set_message("Initializing SCP Info");
	
	for url in SERIES_URLS.iter() {		
		let body = reqwest::get(*url).await?.text().await?;
		let document = Html::parse_document(&body);
		let lis = document.select(&SELECTORS.li_selector);
		
		for li in lis {
			if let Some(link) = li.select(&SELECTORS.a_selector).next() {
				let link_text = link.value().attr("href").unwrap_or("");
				let name = if li.select(&SELECTORS.a_selector).next().is_some() {
					li.text().collect::<Vec<_>>().join("")
						.split("- ").nth(1).map(|s| s.trim().to_string())
						.unwrap_or_default()
				} else {
					String::new()
				};

				if let Some(scp_number) = get_scp_number(link_text) {
					scp_info_vec.push(SCPInfo {
						number: scp_number,
						name,
					});

					pb_scp_info.inc(1);
				}			
			}
		}
	}

	write_json(&scp_info_vec, "output/scp_names.json").await?;
	pb_scp_info.finish_with_message("SCP Info Initialized");
	Ok(())
}

// Get SCP Name from SCP Names JSON based on Number
async fn get_scp_name(number: u16) -> Result<String> {
	let json_data = fs::read_to_string("output/scp_names.json").await?;
	let scp_info_vec: Vec<SCPInfo> = serde_json::from_str(&json_data)?;

	let scp_info = scp_info_vec.iter().find(|&scp| scp.number == number)
		.ok_or_else(|| anyhow!("Name not found for number: {}", number))?;

	Ok(scp_info.name.to_owned())
}

// Text Strings scraping if ACS Bar is not found. 
// Searches the pages for specific phrases/words and adds them to the database if found
async fn backup_function(document: &Html, url: &str) -> Option<ACS> {
	let text = document.root_element().text().collect::<String>().to_lowercase();
	let mut contain = String::new();
	let mut disrupt = String::new();
	let mut risk = String::new();
	let mut secondary = String::new();

	if let Some(index) = text.find("containment class:") {
		contain = extract_word_after_colon(&text[index..]);
	}
	if let Some(index) = text.find("disruption class:") {
		disrupt = extract_word_after_colon(&text[index..]);
	}
	if let Some(index) = text.find("risk class:") {
		risk = extract_word_after_colon(&text[index..]);
	}
	if let Some(index) = text.find("secondary class:") {
		secondary = extract_word_after_colon(&text[index..]);
	}

	for &keyword in &[" vlam ", " keneq ", " ekhi ", " amida "] {
		if text.contains(keyword) {
			disrupt = keyword.to_string();
			break;
		}
	}

	if !contain.is_empty() && (!disrupt.is_empty() || !risk.is_empty() || !secondary.is_empty()) || !disrupt.is_empty() || !risk.is_empty() || !secondary.is_empty() {
		let raw_number = get_scp_number(url).ok_or_else(|| anyhow!("Missing number: {}", url)).ok()?;
		let number = if raw_number <= 999 {
			format!("SCP-{:03}", raw_number)
		} else {
			format!("SCP-{}", raw_number)
		};

		let name = get_scp_name(raw_number).await.ok()?;
		Some(ACS {
			name,
			number,
			clearance: String::new(),
			contain: clean_text(contain),
			secondary: clean_text(secondary),
			disrupt: clean_text(disrupt),
			risk: clean_text(risk),
			url: url.to_string(),
		})
	} else {
		None
	}
}

// Searches the page for the ACS Bar
// If found, selects and scrapes specific elements
// If not found, resorts to Text Strings scraping
async fn get_acs_data(url: &str) ->  Result<Option<ACS>> {
	let body = reqwest::get(url).await?.text().await?;
	let document = Html::parse_document(&body);
	
	let has_anom_bar = document.select(&SELECTORS.acs_bar).next().is_some();
		
	if !has_anom_bar {
		return Ok(backup_function(&document, &url).await);
	}

	let raw_number = get_scp_number(&url).ok_or_else(|| anyhow!("Missing number: {}", &url))?;
	let number = if raw_number <= 999 {
		format!("SCP-{:03}", raw_number)
	} else {
		format!("SCP-{}", raw_number)
	};

	let name = get_scp_name(raw_number).await?;
	let clearance = get_text_from_selector(document.root_element(), &SELECTORS.clearance).unwrap_or_default();
	let contain = get_text_from_selector(document.root_element(), &SELECTORS.contain).unwrap_or_default();
	let secondary = get_text_from_selector(document.root_element(), &SELECTORS.secondary).unwrap_or_default();
	let disrupt = get_text_from_selector(document.root_element(), &SELECTORS.disrupt).unwrap_or_default();
	let risk = get_text_from_selector(document.root_element(), &SELECTORS.risk).unwrap_or_default();

	Ok(Some(ACS {
		name,
		number,
		clearance: clean_text(clearance),
		contain: clean_text(contain),
		secondary: clean_text(secondary),
		disrupt: clean_text(disrupt),
		risk: clean_text(risk),
		url: url.to_string(),
	}))
}

// Main Function
#[tokio::main]
async fn main() -> Result<()> {
	let matches = Args::parse();
	let start = matches.start;
	let end = matches.end;
	let limit = matches.limit;
	let range = Range { start, end };

	init_scp_info_json().await?;
	
	let total = (range.end - range.start + 1) as u16;
	
	let pb_main = ProgressBar::new_spinner();
	pb_main.set_style(ProgressStyle::default_bar()
		.template("{msg} {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta})")
		.expect("Failed to set progress bar style.")
		.progress_chars("##-")
	);
	pb_main.set_message("Fetching ACS data");
	pb_main.set_length(total.into());

	let semaphore = Arc::new(Semaphore::new(limit.into()));

	let acs_data: Vec<ACS> = (start..=end)
	.map(|scp_number| {
		let url_string = format!("https://scp-wiki.wikidot.com/scp-{}", scp_number);
		let pb = pb_main.clone();
		let semaphore = Arc::clone(&semaphore);
		Box::pin(async move {
			let _permit = match semaphore.acquire().await {
				Ok(permit) => permit,
				Err(e) => {
					error!("Failed to acquire semaphore permit: {}", e);
					return None;
				}
			};
			let result = get_acs_data(&url_string).await;
			match result {
				Ok(Some(data)) => {
					pb.inc(1);
					tokio::time::sleep(Duration::from_millis(200)).await;
					Some(data)
				}
				Ok(None) => {
					pb.inc(1);
					None
				}
				Err(e) => {
					error!("Error fetching ACS data for {}: {}", scp_number, anyhow!(e));
					pb.inc(1);
					None
				}
			}
		})
	})
	.collect::<futures::stream::FuturesUnordered<_>>()  // Convert the iterator into a FuturesUnordered
	.collect::<Vec<Option<ACS>>>()
	.await
	.into_iter()
	.filter_map(|x| x)
	.collect();
		
	pb_main.finish_with_message("Done");

	write_json(&acs_data, "output/acs_database.json").await?;
	Ok(())
}