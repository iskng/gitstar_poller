use serde::Deserialize;

// GitHub API response structures
#[derive(Debug, Deserialize)]
pub struct GitHubRepo {
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub stargazers_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub avatar_url: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
pub struct GitHubStarredRepo {
    pub name: String,
    pub full_name: String,
    pub html_url: String,
    pub stargazers_count: u32,
}