//! Linear: issues assigned to you that aren't completed or canceled.

use chrono::NaiveDate;
use flowday_core::model::SourceItem;
use serde_json::{json, Value};

const QUERY: &str = "query { viewer { assignedIssues(first: 100, filter: { state: { type: { nin: [\"completed\", \"canceled\"] } } }) \
{ nodes { id identifier title url dueDate description } } } }";

pub fn parse(resp: &Value) -> Vec<SourceItem> {
    resp["data"]["viewer"]["assignedIssues"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            Some(SourceItem {
                source: "linear".into(),
                source_id: n["id"].as_str()?.to_string(),
                title: format!("{} {}", n["identifier"].as_str().unwrap_or_default(), n["title"].as_str()?).trim().to_string(),
                url: n["url"].as_str().unwrap_or_default().to_string(),
                notes: n["description"].as_str().unwrap_or_default().chars().take(1000).collect(),
                deadline: n["dueDate"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()),
            })
        })
        .collect()
}

pub fn pull(key: &str) -> Result<Vec<SourceItem>, String> {
    let resp: Value = ureq::post("https://api.linear.app/graphql")
        .set("Authorization", key)
        .send_json(json!({ "query": QUERY }))
        .map_err(|e| crate::http_error("Linear", e))?
        .into_json()
        .map_err(|e| e.to_string())?;
    if let Some(err) = resp["errors"][0]["message"].as_str() {
        return Err(format!("Linear: {err}"));
    }
    Ok(parse(&resp))
}
