//! GitHub: open issues and pull requests assigned to you, or awaiting your review.

use flowday_core::model::SourceItem;
use serde_json::Value;

pub fn parse(search: &Value) -> Vec<SourceItem> {
    search["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let url = i["html_url"].as_str()?;
            // https://github.com/owner/repo/issues/12 -> owner/repo#12
            let mut parts = url.trim_start_matches("https://github.com/").split('/');
            let (owner, repo, _, num) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            let kind = if i.get("pull_request").is_some() { "PR" } else { "issue" };
            let body = i["body"].as_str().unwrap_or_default();
            Some(SourceItem {
                source: "github".into(),
                source_id: format!("{owner}/{repo}#{num}"),
                title: i["title"].as_str()?.to_string(),
                url: url.to_string(),
                notes: format!("{owner}/{repo} {kind} #{num}\n\n{}", body.chars().take(1000).collect::<String>()),
                deadline: None,
            })
        })
        .collect()
}

fn search(token: &str, q: &str) -> Result<Value, String> {
    ureq::get(&format!("https://api.github.com/search/issues?per_page=100&q={}", crate::enc(q)))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "flowday")
        .call()
        .map_err(|e| crate::http_error("GitHub", e))?
        .into_json()
        .map_err(|e| e.to_string())
}

pub fn pull(token: &str) -> Result<Vec<flowday_core::model::SourceItem>, String> {
    let mut items = parse(&search(token, "is:open assignee:@me archived:false")?);
    for item in parse(&search(token, "is:open is:pr review-requested:@me archived:false")?) {
        if !items.iter().any(|i| i.source_id == item.source_id) {
            items.push(item);
        }
    }
    Ok(items)
}

/// The token from the GitHub CLI, when no token was saved.
pub fn gh_cli_token() -> Option<String> {
    let out = std::process::Command::new("gh").args(["auth", "token"]).output().ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !token.is_empty()).then_some(token)
}
