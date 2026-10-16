//! Notion: pages in the databases you choose, skipping ones marked done (a checked
//! checkbox, or a status/select named Done, Complete or Completed).

use chrono::NaiveDate;
use flowday_core::model::SourceItem;
use serde_json::{json, Value};

fn is_done(props: &serde_json::Map<String, Value>) -> bool {
    props.iter().any(|(name, p)| {
        let done_name = |v: &Value| {
            v["name"].as_str().is_some_and(|n| matches!(n.to_lowercase().as_str(), "done" | "complete" | "completed" | "archived"))
        };
        match p["type"].as_str() {
            Some("checkbox") => p["checkbox"] == true && name.to_lowercase().contains("done"),
            Some("status") => done_name(&p["status"]),
            Some("select") => done_name(&p["select"]),
            _ => false,
        }
    })
}

pub fn parse(resp: &Value) -> Vec<SourceItem> {
    resp["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|page| page["archived"] != true && page["in_trash"] != true)
        .filter_map(|page| {
            let props = page["properties"].as_object()?;
            if is_done(props) {
                return None;
            }
            let title: String = props
                .values()
                .find(|p| p["type"] == "title")?["title"]
                .as_array()?
                .iter()
                .filter_map(|t| t["plain_text"].as_str())
                .collect();
            if title.trim().is_empty() {
                return None;
            }
            let deadline = props
                .values()
                .find(|p| p["type"] == "date")
                .and_then(|p| p["date"]["start"].as_str())
                .and_then(|s| NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok());
            Some(SourceItem {
                source: "notion".into(),
                source_id: page["id"].as_str()?.to_string(),
                title: title.trim().to_string(),
                url: page["url"].as_str().unwrap_or_default().to_string(),
                notes: String::new(),
                deadline,
            })
        })
        .collect()
}

/// Accepts a bare id or a database URL (`…/Tasks-<32 hex>?v=…`).
pub fn database_id(s: &str) -> &str {
    let seg = s.trim().rsplit('/').next().unwrap_or(s).split('?').next().unwrap_or(s);
    if seg.len() > 32 && seg.is_char_boundary(seg.len() - 32) {
        &seg[seg.len() - 32..]
    } else {
        seg
    }
}

pub fn pull(token: &str, databases: &[String]) -> Result<Vec<SourceItem>, String> {
    let mut items = Vec::new();
    for db in databases {
        let db = database_id(db);
        let resp: Value = ureq::post(&format!("https://api.notion.com/v1/databases/{db}/query"))
            .set("Authorization", &format!("Bearer {token}"))
            .set("Notion-Version", "2022-06-28")
            .send_json(json!({ "page_size": 100 }))
            .map_err(|e| crate::http_error("Notion", e))?
            .into_json()
            .map_err(|e| e.to_string())?;
        items.extend(parse(&resp));
    }
    Ok(items)
}
