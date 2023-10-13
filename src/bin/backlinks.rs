use anyhow::Result;
use once_cell::sync::Lazy;
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use indicatif::{ProgressBar, ProgressStyle};
use regex::{Regex, RegexSet};
use rand::Rng;
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use scraper::{Html, Selector};
use std::{sync::Arc, fs::OpenOptions, io::{self, Read, Seek}};
use titlecase;
use log::{info, error};
use tokio::{fs, task, sync::Semaphore};

#[derive(Serialize, Deserialize)]
struct SCPInfo {
	number: u16,
	name: String,
}

static SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(30));
static SCP_NUM_RE: Lazy<Regex> = Lazy::new(|| {
		Regex::new(r"(?i)\bscp-([0-9]{1,4})$").unwrap()
});

async fn append_json_to_file(json: &Value, file_path: &str) -> Result<()> {
  let json = json.clone();
  let file_path = file_path.to_owned();

  task::spawn_blocking(move || {
    let mut file = OpenOptions::new()
      .read(true)
      .write(true)
      .create(true)
      .open(&file_path)?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    // Parse the existing file contents into a JSON value
    let mut existing_json = match serde_json::from_str(&contents) {
      Ok(json) => json,
      Err(_) => serde_json::Value::Array(Vec::new()), // If parsing fails, assume an empty array
    };

    // Append the new data to the existing JSON array
    if let Some(array) = existing_json.as_array_mut() {
      if let Some(new_array) = json.as_array() {
        array.extend_from_slice(new_array);
      } else {
        array.push(json.clone());
      }
    }

    // Seek to the beginning of the file before writing
    file.seek(io::SeekFrom::Start(0))?;
    file.set_len(0)?;  // Truncate the file

    // Write the updated JSON array back to the file
    serde_json::to_writer_pretty(io::BufWriter::new(&file), &existing_json)?;

    Ok::<(), anyhow::Error>(())
  }).await??;

  Ok(())
}

async fn request_page(url: &str) -> Result<Html> {
	let body = reqwest::get(url).await?.text().await?;
	Ok(Html::parse_document(&body)) 
}

async fn get_scp_name(number: u16) -> Result<Option<String>> {
	let json_data = fs::read_to_string("output/scp_names.json").await?;
	let scp_names_vec: Vec<SCPInfo> = serde_json::from_str(&json_data)?;

	let scp_name = scp_names_vec.iter().find(|&scp| scp.number == number)
		.map(|scp| scp.name.to_owned());

	Ok(scp_name)
}

fn extract_scp_number(scp_str: &str) -> Option<u16> {
	let cap = SCP_NUM_RE.captures(scp_str)?;
	let number = cap[1].parse::<u16>().ok()?;
	Some(number)
}

async fn parse_html_to_json(html_body: Arc<Html>) -> Result<serde_json::Value> {
	let document = Arc::clone(&html_body);
	let _permit = SEMAPHORE.acquire().await;

	let link_selector = Selector::parse("ul li a:first-of-type").expect("Failed to create link Selector");
	let breadcrumb_selector = Selector::parse("#breadcrumbs > a:last-of-type").expect("Failed to create link Selector");

	let mut links: Vec<serde_json::Value> = Vec::new();
	let re = Regex::new(r" \(/\S+\)").unwrap();
	let regex_set = RegexSet::new(&[
		r"(?i)http",
		r"(?i)component",
		r"(?i)guide",
		r"(?i)author",
		r"(?i)memo",
		r"(?i)acs",
		r"(?i)personnel",
		r"(?i)icons",
		r"(?i)art:",
		r"(?i)resource",
	]).unwrap();

	let total_entries: u16 = document.select(&link_selector)
		.filter(|element| {
			let url = element.value().attr("href").unwrap_or_default();
			!regex_set.is_match(url)
		})
		.count() as u16;
	let mut fragments = 0;
	let mut normal = 0;
	let backlinks_pb = ProgressBar::new_spinner();
	backlinks_pb.set_style(
		ProgressStyle::default_bar()
			.template("{msg} {spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>7}/{len:7} ({eta})")? 
			.progress_chars("##-")
	);
	let message = format!("Fetching ACS Backlinks Info - Fragments: {} / Normal {}",  fragments, normal);
	backlinks_pb.set_message(message);
	backlinks_pb.set_length(total_entries.into());	

	for element in document.select(&link_selector) {		
		let url: String = element.value().attr("href").unwrap_or_default().to_string();
		let is_fragment = url.contains("fragment:");	
		
		if is_fragment {
			fragments += 1; 
		} else {
			normal += 1;
		}

		let message = format!("Fetching ACS Backlinks Info - Fragments: {} / Normal {}",  fragments, normal);
		backlinks_pb.set_message(message);

		let name_text = element.text().collect::<Vec<_>>().join("");
		let mut name = name_text.trim().to_string();    
		let mut number: u16 = 0;

		info!("Initial name: {}", name);

		if regex_set.is_match(&url) || regex_set.is_match(&name) {
			continue;
		}

		name = re.replace_all(&name, "").to_string();

		if SCP_NUM_RE.is_match(&url) && !is_fragment {
			if let Some(scp_number) = extract_scp_number(&url) {
				number = scp_number;
				match get_scp_name(scp_number).await {
					Ok(Some(name_from_json)) => {
						name = name_from_json;
						info!("SCP Number: {} | Name from JSON: {}", number, name);
					},
					Ok(None) => {
						info!("No name found for SCP Number: {}, skipping.", number);
						continue;
					},
					Err(e) => {
						error!("Error getting name for SCP Number: {}: {}", number, e);
						continue;
					},
				}
			}
		} else if name.len() <= 1 {
			info!("SCP URL: {} | Name <= 1: {}", url, name);
			if let Some(url_name) = url.rsplit('/').next() {
				name = url_name.replace("fragment:", "").replace("ii", "II").replace("-s", "'s").replace("-", " ");
				name = titlecase::titlecase(&name);
			}
		}

		if is_fragment {
			let page_url = format!("https://scp-wiki.wikidot.com{}/norender/true", url);
			let document = request_page(&page_url).await?;
			let mut breadcrumb = document.select(&breadcrumb_selector);
			if let Some(first_breadcrumb) = breadcrumb.next() {
				let breadcrumb_text = first_breadcrumb.text().collect::<String>();
				let breadcrumb_match = SCP_NUM_RE.is_match(&breadcrumb_text);
				info!("breadcrumb text: {} | matches SCP_RUM_RE: {}", breadcrumb_text, breadcrumb_match);
				if SCP_NUM_RE.is_match(&breadcrumb_text) {
					if let Some(scp_number) = extract_scp_number(&breadcrumb_text) {
						number = scp_number;
						match get_scp_name(scp_number).await {
							Ok(Some(name_from_json)) => {
								name = name_from_json;
								info!("SCP Number: {} | Name from JSON: {}", number, name);
							},
							Ok(None) => {
								info!("No name found for SCP Number: {}, skipping.", number);
								continue;
							},
							Err(e) => {
								error!("Error getting name for SCP Number: {}: {}", number, e);
								continue;
							},
						}
					}
				} else {
					name = breadcrumb_text
				}		
			}
		}

		if name.as_str().to_lowercase().contains("proposal") || url.as_str().to_lowercase().contains("proposal") {
			number = 1;
		}		

		info!("Final name: {}, Final number: {}", name, number);

		if !links.iter().any(|link| link["url"] == url.as_str()) {
			links.push(json!({
				"number": number,
				"name": name,
				"url": format!("https://scp-wiki.wikidot.com{}", url.as_str()),
				"fragment": is_fragment
			}));
		}

		backlinks_pb.inc(1);
	}

	backlinks_pb.finish();
	Ok(serde_json::Value::Array(links))
}

#[tokio::main]
pub async fn fetch_backlinks() -> Result<()> {

  // Initialize logger
  if let Err(_) = pretty_env_logger::try_init() {
    log::warn!("Logger is already initialized.");
  }

  // Generate a random token
  let token: String = rand::thread_rng()
    .sample_iter(&rand::distributions::Alphanumeric)
    .take(8)
    .map(char::from)
    .collect();

  info!("Created token: {}", token);

  // Set up cookies
  let mut headers = HeaderMap::new();
  headers.insert("Cookie", HeaderValue::from_str(&format!("wikidot_token7={}", token)).unwrap());
  headers.insert("User-Agent", HeaderValue::from_static("reqwest/0.11.20 (rust)"));

  info!("Created headers: {:?}", headers);

  // Set up parameters
  let page_ids = ["858310940", "1058262511", "1307058244"];

  // Make the HTTP request
  let client = Client::new();

  for page_id in &page_ids {
    let params = [
      ("page_id", *page_id),
      ("moduleName", "backlinks/BacklinksModule"),
      ("callbackIndex", "1"),
      ("wikidot_token7", &token),
    ];

    info!("Created Params for page_id {}: {:?}", page_id, params);

    let response = client.post("https://scp-wiki.wikidot.com/ajax-module-connector.php")
      .headers(headers.clone())  // Clone headers to ensure they are fresh for each request
      .form(&params)
      .send()
      .await?;

    if response.status().is_success() {
      let json: serde_json::Value = response.json().await?;
      let html: Option<Arc<Html>> = match json.get("body") {
        Some(html_body) => {
          let html_body_str = html_body.as_str().ok_or(anyhow::anyhow!("Failed to convert html_body to str"))?;
          let html = Html::parse_document(html_body_str);
          Some(Arc::new(html))
        },
        None => {
          error!("No HTML body");
          None
        }
      };
      if let Some(html) = html {
        let parsed_json = parse_html_to_json(html).await?;
        append_json_to_file(&parsed_json, "output/acs_bar_backlinks.json").await?;
      }
    } else {
      error!("Failed request or response for page_id {}: {:?}", page_id, response.status());
    }
  }
  
  Ok(())
}