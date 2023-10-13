mod backlinks;

use anyhow::{anyhow, Result};
use clap::Parser;
use futures::stream::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use log::error;
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs::File, clone::Clone, sync::{Arc, atomic::{AtomicU64, Ordering}}};
use tokio::{fs, time::Duration, sync::Semaphore};

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
	fragment: bool,
}

// SCP Names Selectors
static LI_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("[id*='toc'] + ul li").unwrap());
static LINK_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("a:not(.newpage)").unwrap());

// ACS Bar Selectors
static ACS_BAR_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.anom-bar-container").unwrap());
static CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.top-right-box > div.level").unwrap());
static CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.contain-class > div.class-text").unwrap());
static SECONDARY_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.second-class > div.class-text").unwrap());
static DISRUPT_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.disrupt-class > div.class-text").unwrap());
static RISK_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.risk-class > div.class-text").unwrap());

// ACS Hybrid Bar Selectors
static ACS_HYBRID_BAR_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-hybrid-text-bar").unwrap());
static HYBRID_CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-clear > strong").unwrap());
static HYBRID_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-contain > div.acs-text > span:nth-of-type(2)").unwrap());
static HYBRID_SECONDARY_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-secondary > div.acs-text > span:nth-of-type(2)").unwrap());
static HYBRID_DISRUPT_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-disrupt > div.acs-text").unwrap());
static HYBRID_RISK_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.acs-risk > div.acs-text").unwrap());

// Flops Header Selectors
static FLOPS_HEADER_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.itemInfo.darkbox").unwrap());
static FLOPS_CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.itemInfo.darkbox > tbody:nth-child(1) > tr:nth-child(1) > td:nth-child(2) > span:nth-child(1)").unwrap());
static FLOPS_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.itemInfo.darkbox > tbody:nth-child(1) > tr:nth-child(2) > td:nth-child(1)").unwrap());
static FLOPS_DISRUPT_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.itemInfo.darkbox + p > a.disruptionHeader").unwrap());

// AIM Header Selectors
static AIM_HEADER_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.desktop-aim").unwrap());
static AIM_CLEARANCE_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.desktop-aim > div.w-container > div > div:nth-child(2) > p > span > span").unwrap());
static AIM_CONTAIN_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.desktop-aim > div.w-container > div > div:nth-child(3) > p").unwrap());
static AIM_DISRUPT_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("div.desktop-aim > div.w-container > div > div:nth-child(4) > p").unwrap());

static SCP_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)scp-([0-9]{1,4})").unwrap());

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
fn extract_scp_number(scp_str: &str) -> Option<u16> {

  let cap = SCP_NUM_RE.captures(scp_str)?;
  
  let number = match cap[1].parse::<u16>() {
    Ok(num) => Some(num),
    Err(e) => {
      log::error!("Failed to parse SCP number {}: {}", scp_str, e);
      None
    }
  }?;

  Some(number)

}

fn extract_text(element: ElementRef, selector: &Selector) -> Option<String> {
	element.select(&selector)
		.next()?
		.text()
		.collect::<Vec<_>>()
		.join("")
		.trim()
		.to_string()
		.into()
}

fn extract_class(element: ElementRef, selector: &Selector) -> Option<String> {
	element.select(&selector)
		.next()?
		.value()
		.attr("class")
		.map(|s| s.to_string())
		.into()
}

fn is_valid_containment_class(class: &str) -> bool {
	["safe", "euclid", "keter", "neutralized", "pending", "explained", "esoteric"].iter()
		.any(|&valid_class| class.eq_ignore_ascii_case(valid_class))
}

fn extract_word_after_colon(text: &str) -> String {
	text.splitn(2, ':')
		.nth(1)
		.and_then(|s| s.split_whitespace().next())
		.unwrap_or("")
		.to_string()
}

fn clean_text(text: String) -> String {
	if text.contains("{$") || text.eq_ignore_ascii_case("none") {
		return String::new()
	} if text.contains(":") {
		return extract_word_after_colon(&text);
	} if ( text != "N/A" || text != "n/a" ) && text.contains("/") {
		text.splitn(2, "/")
			.nth(1)
			.unwrap_or("")
			.to_string()
	} else {
		text
	}
}

fn create_acs(
	name: String,
	mut number: String,
	clearance: String,
	contain: String,
	secondary: String,
	disrupt: String,
	risk: String,
	scp_url: String,
	fragment: bool
) -> ACS {
	if name.as_str().to_lowercase().contains("scp-") {
		number = name.clone();
	}

	ACS {
		name,
		number,
		clearance: clean_text(clearance),
		contain: clean_text(contain),
		secondary: clean_text(secondary),
		disrupt: clean_text(disrupt),
		risk: clean_text(risk),
		url: scp_url,
		fragment,
	}
}

//Helper Async Functions
async fn request_page(url: &str) -> Result<Option<Html>> {
	let client = reqwest::Client::new();
	let response = client.get(url)
		.header(reqwest::header::USER_AGENT, "reqwest/0.11.20 (rust)")
		.send()
		.await?;
	
	log::info!("Received status {} from {}", response.status(), url);
	
	if response.status() == reqwest::StatusCode::NOT_FOUND {
		return Ok(None);
	} else if !response.status().is_success() {
		return Err(anyhow!("Failed to fetch URL: {} - Status: {}", url, response.status()));
	}

	let body = response.text().await?;
	Ok(Some(Html::parse_document(&body)))
}


async fn write_json<T: Serialize>(data: &[T], path: &str) -> Result<()> {
	let file = File::create(path)?;
	serde_json::to_writer_pretty(file, data)?;
	
	Ok(())
}

// Scrape SCP Series Pages -> Get SCP Names -> Write them to JSON File
async fn init_scp_names_json() -> Result<()> {
	let mut scp_names_vec: Vec<SCPInfo> = Vec::new();

	let progress_bar_scp_names = ProgressBar::new_spinner();
	progress_bar_scp_names.set_style(ProgressStyle::default_bar()
		.template("{msg} {spinner:.green} {pos:>7}")
		.expect("Failed to set progress bar style.")
		.progress_chars("=> ")
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
						scp_names_vec.push(SCPInfo {
							number: scp_number,
							name,
						});

						progress_bar_scp_names.inc(1);
					}
				}
			}
		} else {
			log::info!("Page not found: {}", series_url);
		}
	}

	write_json(&scp_names_vec, "output/scp_names.json").await?;
	progress_bar_scp_names.finish_with_message("SCP Info Initialized");
	Ok(())
}

// Get SCP Name from SCP Names JSON based on Number
async fn get_scp_name(number: u16) -> Result<String> {
	let json_data = fs::read_to_string("output/scp_names.json").await?;
	let scp_names_vec: Vec<SCPInfo> = serde_json::from_str(&json_data)?;

	let scp_names = scp_names_vec.iter().find(|&scp| scp.number == number)
		.ok_or_else(|| anyhow!("Name not found for number: {}", number))?;

	Ok(scp_names.name.to_owned())
}

// Text Strings scraping if ACS Bar is not found.
// Searches the pages for specific phrases/words and adds them to the database if found
async fn backup_acs_function(document: &Html) -> Option<(String, String, String, String)> {
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
		Some((
			clean_text(contain),
			clean_text(secondary),
			clean_text(disrupt),
			clean_text(risk)
		))
	} else {
		None
	}
}

// ACS Bar Scraper
async fn get_acs_bar(document: &Html) -> (String, String, String, String, String) {
	let mut clearance = clean_text(extract_text(document.root_element(), &CLEARANCE_SELECTOR).unwrap_or_default());
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let contain = clean_text(extract_text(document.root_element(), &CONTAIN_SELECTOR).unwrap_or_default());
	let secondary = clean_text(extract_text(document.root_element(), &SECONDARY_SELECTOR).unwrap_or_default());
	let disrupt = clean_text(extract_text(document.root_element(), &DISRUPT_SELECTOR).unwrap_or_default());
	let risk = clean_text(extract_text(document.root_element(), &RISK_SELECTOR).unwrap_or_default());

	(
		clearance,
		contain,
		secondary,
		disrupt,
		risk
	)
}

// ACS Hybrid Bar Scraper
async fn get_acs_hybrid_bar(document: &Html) -> (String, String, String, String, String) {
	let mut clearance = extract_text(document.root_element(), &HYBRID_CLEARANCE_SELECTOR).unwrap_or_default();
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let contain = clean_text(extract_text(document.root_element(), &HYBRID_CONTAIN_SELECTOR).unwrap_or_default());
	let secondary = clean_text(extract_text(document.root_element(), &HYBRID_SECONDARY_SELECTOR).unwrap_or_default());
	let disrupt = clean_text(extract_text(document.root_element(), &HYBRID_DISRUPT_SELECTOR).unwrap_or_default());
	let risk = clean_text(extract_text(document.root_element(), &HYBRID_RISK_SELECTOR).unwrap_or_default());

	(
		clearance,
		contain,
		secondary,
		disrupt,
		risk
	)
}

// Flops Header Scraper
async fn get_flops_header(document: &Html) -> (String, String, String, String) {
	let mut clearance = clean_text(extract_text(document.root_element(), &FLOPS_CLEARANCE_SELECTOR).unwrap_or_default());
	for i in 1..=MAX_LEVEL {
		if clearance.contains(&format!("{}", i)) {
			clearance = format!("LEVEL {}", i);
			break;
		}
	}
	let mut contain = clean_text(extract_text(document.root_element(), &FLOPS_CONTAIN_SELECTOR).unwrap_or_default());
	let mut secondary = String::new();

	if !is_valid_containment_class(&contain) {
		secondary = contain;
		contain = "esoteric".to_string();
	}
	let disrupt = clean_text(extract_text(document.root_element(), &FLOPS_DISRUPT_SELECTOR).unwrap_or_default());

	(
		clearance,
		contain,
		secondary,
		disrupt
	)
}

// AIM Header Scraper
async fn get_aim_header(document: &Html) -> (String, String, String, String) {
	let clearance_item = extract_class(document.root_element(), &AIM_CLEARANCE_SELECTOR).unwrap_or_default();
	let clearance = match clearance_item.as_str() {
		"one" => "LEVEL 1",
		"two" => "LEVEL 2",
		"three" => "LEVEL 3",
		"four" => "LEVEL 4",
		"five" => "LEVEL 5",
		"six" => "LEVEL 6",
		_ => "",
	}.to_string();
	let mut contain = clean_text(extract_text(document.root_element(), &AIM_CONTAIN_SELECTOR).unwrap_or_default());
	let mut secondary = String::new();

	if !is_valid_containment_class(&contain) {
		secondary = contain;
		contain = "esoteric".to_string();
	}
	let disrupt = clean_text(extract_text(document.root_element(), &AIM_DISRUPT_SELECTOR).unwrap_or_default());

	(
		clearance,
		contain,
		secondary,
		disrupt
	)
}

// Searches the page for the ACS Bar & ACS Hybrid Bar
// If found, selects and scrapes specific elements
// If not found, resorts to Text Strings scraping
async fn fetch_acs_data(scp_number: u16, mut name: Option<&str>, scp_url: &str) -> Result<Option<ACS>> {
	log::info!("Fetching data from: {}", scp_url);
	let document = request_page(scp_url).await?;

	if let Some(document) = document {
		let has_anom_bar = document.select(&ACS_BAR_SELECTOR).next().is_some();
		let has_hybrid_anom_bar = document.select(&ACS_HYBRID_BAR_SELECTOR).next().is_some();
		let has_flops_header = document.select(&FLOPS_HEADER_SELECTOR).next().is_some();
		let has_aim_header = document.select(&AIM_HEADER_SELECTOR).next().is_some();

		let scp_name: String;

		if scp_number != 0 && scp_number != 1 {
			scp_name = get_scp_name(scp_number).await?;
			name = Some(&scp_name);
		}
		
		let number = if scp_number <= 99 {
			format!("SCP-{:03}", scp_number)
		} else {
			format!("SCP-{}", scp_number)
		};		

		let mut clearance = String::new();
		let mut contain = String::new();
		let mut secondary = String::new();
		let mut disrupt = String::new();
		let mut risk = String::new();

		if has_hybrid_anom_bar {
			(clearance, contain, secondary, disrupt, risk) = get_acs_hybrid_bar(&document).await;
		} else if has_anom_bar {
			(clearance, contain, secondary, disrupt, risk) = get_acs_bar(&document).await;
		} else if has_flops_header {
			(clearance, contain, secondary, disrupt) = get_flops_header(&document).await;
		} else if has_aim_header {
			(clearance, contain, secondary, disrupt) = get_aim_header(&document).await;
		} else if !has_hybrid_anom_bar && !has_anom_bar && !has_flops_header && !has_aim_header {
			if let Some((c, s, d, r)) = backup_acs_function(&document).await {
				clearance = String::new();
				contain = c;
				secondary = s;
				disrupt = d;
				risk = r;
			} else {
				return Ok(None);
			}
		}

		
		
		Ok(Some(create_acs(
			name.unwrap_or("").to_string(),
			number.clone(),
			clearance,
			contain,
			secondary,
			disrupt,
			risk,
			scp_url.to_string(),
			false,
		)))
	} else {
		log::info!("Page not found: {}", scp_url);
		Ok(None)
	}
}

// Compare ACS Backlinks and add to Database if not included
async fn fetch_and_update_entry(number: u16, name: &str, url: &str, fragment: bool) -> Result<serde_json::Value> {
	log::info!("Fetching data from: {}", url);
	match fetch_acs_data(number, Some(name), url).await {
		Ok(Some(acs_data)) => {
			log::info!("Successfully fetched ACS Bar Data from: {}", url);
			let new_entry = serde_json::json!({
				"name": acs_data.name,
				"number": acs_data.number,
				"clearance": acs_data.clearance,
				"contain": acs_data.contain,
				"secondary": acs_data.secondary,
				"disrupt": acs_data.disrupt,
				"risk": acs_data.risk,
				"url": acs_data.url,
				"fragment": fragment
			});
			Ok(new_entry)
		},
		Ok(None) => Err(anyhow!("f: fetch_and_update_entry | Failed to fetch ACS data for: {}", url)),
		Err(e) => Err(anyhow!("f: fetch_and_update_entry | Error fetching ACS data for {}: {}", url, e))
	}
}

async fn cross_compare_and_update(limit: u16) -> Result<()> {
	let acs_bar_backlinks_data = fs::read_to_string("output/acs_bar_backlinks.json").await.expect("Unable to read acs_bar_backlinks.json");
	let acs_database_data = fs::read_to_string("output/acs_database.json").await.expect("Unable to read acs_database.json");

	let acs_bar_backlinks: Vec<Value> = serde_json::from_str(&acs_bar_backlinks_data).expect("Error parsing acs_bar_backlinks.json");
	let mut acs_database: Vec<Value> = serde_json::from_str(&acs_database_data).expect("Error parsing acs_database.json");

	let semaphore = Arc::new(Semaphore::new(limit.into()));

	let total_entries = acs_bar_backlinks.len() as u64;
	let matches = Arc::new(AtomicU64::new(0));
	let pb = ProgressBar::new_spinner();
	pb.set_style(
		ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta})")
			.expect("Failed to set progress bar style.")
			.progress_chars("##-")
	);

	let matches_clone = Arc::clone(&matches);
	let message = format!("Cross comparing ACS Bar Backlinks to ACS Database - Matches: {}", matches_clone.load(Ordering::Relaxed));
	pb.set_message(message);
	pb.set_length(total_entries);

	let new_entries: Vec<(Value, u64, u64)> = acs_bar_backlinks.into_iter().filter_map(|link_item| {
		let raw_number = link_item["number"].as_u64().unwrap_or(0) as u16;
		log::info!("Reading data for: SCP-{}", raw_number);

		let number = if raw_number <= 99 {
			format!("SCP-{:03}", raw_number)
		} else {
			format!("SCP-{}", raw_number)
		};
		if acs_database.iter().any(|db_item| db_item["number"].as_str().unwrap_or_default() == number) {
			None
		} else {
			Some((link_item, 1))
		}
	}).map(|(link_item, progress)| {
		let semaphore = Arc::clone(&semaphore);
		let pb = pb.clone();
		let matches_clone = Arc::clone(&matches);
		Box::pin(async move {
			log::info!("Reading url data for: {}", link_item);
			let _permit = semaphore.acquire().await.expect("Failed to acquire semaphore");
			let fragment = link_item["fragment"].as_bool().unwrap_or_default();
			match fetch_and_update_entry(link_item["number"].as_u64().unwrap_or(0) as u16, link_item["name"].as_str().unwrap_or_default(), link_item["url"].as_str().unwrap_or_default(), fragment).await {
				Ok(data) => {
					pb.inc(1);
					matches_clone.fetch_add(1, Ordering::Relaxed);
					let message = format!("Cross comparing ACS Bar Backlinks to ACS Database - Matches: {}", matches_clone.load(Ordering::Relaxed));
					pb.set_message(message);
					tokio::time::sleep(Duration::from_millis(1000)).await;
					Some((data, progress, matches_clone.load(Ordering::Relaxed)))
				},
				Err(e) => {
					error!("f: cross_compare_and_update | Error fetching ACS data for {}: {}", link_item, e);
					pb.inc(1);
					None
				}
			}
		})
	}).collect::<futures::stream::FuturesUnordered<_>>()
		.collect::<Vec<Option<(Value, u64, u64)>>>()
		.await
		.into_iter()
		.filter_map(|x| x)
		.collect();

	let finish_message = format!("Done! - Matches: {}", matches.load(Ordering::Relaxed));
	pb.finish_with_message(finish_message);

	acs_database.extend(new_entries.into_iter().filter_map(|x| Some(x)).map(|(val, _, _)| val));

	fs::write("output/acs_database.json", serde_json::to_string_pretty(&acs_database)?).await?;

	Ok(())
}

// Main Function
#[tokio::main]
async fn main() -> Result<()> {

	if let Err(_) = pretty_env_logger::try_init() {
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
		match tokio::task::spawn_blocking(|| backlinks::fetch_backlinks()).await {
			Ok(Ok(_)) => log::info!("Completed fetch_backlinks successfully."),
			Ok(Err(e)) => log::error!("Error in fetch_backlinks: {:?}", e),
			Err(e) => log::error!("Task aborted due to panic: {:?}", e),
		}
	}
	
	if args.scraper {
		let total = (range.end - range.start + 1) as u16;
	
		let progress_bar = ProgressBar::new_spinner();
		progress_bar.set_style(ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} ({eta})")
			.expect("Failed to set progress bar style.")
			.progress_chars("##-")
		);
		progress_bar.set_message("Fetching ACS data");
		progress_bar.set_length(total.into());

		let semaphore = Arc::new(Semaphore::new(limit.into()));

		let acs_data: Vec<ACS> = (start..=end)
		.map(|scp_number| {
			let scp_url_string = if scp_number <= 99 {
				format!("https://scp-wiki.wikidot.com/scp-{:03}", scp_number)
			} else {
				format!("https://scp-wiki.wikidot.com/scp-{}", scp_number)
			};
			let pb = progress_bar.clone();
			let semaphore = Arc::clone(&semaphore);
			Box::pin(async move {
				let _permit = semaphore.acquire().await.map_err(|e| {
					error!("Failed to acquire semaphore permit for {}: {}", scp_number, e);
					e
				}).ok()?;
				let mut retries = 0;
				let mut result = fetch_acs_data(scp_number, None, &scp_url_string).await;
				while result.is_err() && retries < args.retries.into() {
						retries += 1;
						tokio::time::sleep(Duration::from_secs(2 * retries)).await;
						result = fetch_acs_data(scp_number, None, &scp_url_string).await;
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
						error!("f: main > scraper | Error fetching ACS data for {}: {}", scp_number, e);
						pb.inc(1);
						None
					}
				}
			})
		})
		.collect::<futures::stream::FuturesUnordered<_>>()
		.collect::<Vec<Option<ACS>>>()
		.await
		.into_iter()
		.filter_map(|x| x)
		.collect();
			
		progress_bar.finish_with_message("Done");

		write_json(&acs_data, "output/acs_database.json").await?;		
	}

	if args.cross {
		cross_compare_and_update(args.limit).await?;
	}
	
	Ok(())
}