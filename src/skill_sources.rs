use crate::models::{SkillSearchResult, SkillSourceConfig};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use futures_util::future::{join_all, BoxFuture};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

pub trait SkillSourceAdapter: Send + Sync {
    fn source_id(&self) -> &str;
    fn search<'a>(&'a self, keyword: &'a str, limit: usize) -> BoxFuture<'a, Result<Vec<SkillSearchResult>>>;
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceSearchOutcome {
    pub source_id: String,
    pub results: Vec<SkillSearchResult>,
    pub error: Option<String>,
}

pub struct GitHubSkillSource {
    source: SkillSourceConfig,
    client: Client,
}

impl GitHubSkillSource {
    pub fn new(source: SkillSourceConfig) -> Result<Self> {
        Ok(Self { source: source.clone(), client: source_client(source.api_key.as_deref())? })
    }
}

impl SkillSourceAdapter for GitHubSkillSource {
    fn source_id(&self) -> &str { &self.source.id }

    fn search<'a>(&'a self, keyword: &'a str, limit: usize) -> BoxFuture<'a, Result<Vec<SkillSearchResult>>> {
        Box::pin(async move {
            let mut url = Url::parse(&format!("{}/search/code", self.source.url.trim_end_matches('/')))
                .context("GitHub 搜索地址无效")?;
            url.query_pairs_mut()
                .append_pair("q", &format!("SKILL.md {} in:path", keyword.trim()))
                .append_pair("per_page", &limit.min(100).to_string());
            let response = self.client.get(url).header("Accept", "application/vnd.github+json").send().await?;
            if !response.status().is_success() {
                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    bail!("GitHub 搜索需要认证，请在来源设置中填写 GitHub Personal Access Token");
                }
                bail!("GitHub 搜索请求失败: HTTP {}", response.status());
            }
            let payload: GitHubCodeSearchResponse = response.json().await?;
            Ok(payload.items.into_iter().filter_map(|item| github_result(&self.source.id, item)).collect())
        })
    }
}

pub struct SkillHubSource {
    source: SkillSourceConfig,
    client: Client,
}

impl SkillHubSource {
    pub fn new(source: SkillSourceConfig) -> Result<Self> {
        Ok(Self { source: source.clone(), client: source_client(source.api_key.as_deref())? })
    }
}

impl SkillSourceAdapter for SkillHubSource {
    fn source_id(&self) -> &str { &self.source.id }

    fn search<'a>(&'a self, keyword: &'a str, limit: usize) -> BoxFuture<'a, Result<Vec<SkillSearchResult>>> {
        Box::pin(async move {
            let mut url = Url::parse(&format!("{}/api/skills", self.source.url.trim_end_matches('/')))
                .context("SkillHub 搜索地址无效")?;
            url.query_pairs_mut()
                .append_pair("page", "1")
                .append_pair("pageSize", &limit.min(100).to_string())
                .append_pair("sortBy", "score")
                .append_pair("order", "desc")
                .append_pair("keyword", keyword.trim());
            let response = self.client.get(url).send().await?;
            if !response.status().is_success() {
                bail!("SkillHub 搜索请求失败: HTTP {}", response.status());
            }
            let payload: SkillHubSearchResponse = response.json().await?;
            if payload.code != 0 {
                bail!("SkillHub 搜索失败: {}", payload.message);
            }
            Ok(payload.data.skills.into_iter().map(|item| skillhub_result(&self.source, item)).collect())
        })
    }
}

pub struct CustomIndexSource {
    source: SkillSourceConfig,
    client: Client,
}

impl CustomIndexSource {
    pub fn new(source: SkillSourceConfig) -> Result<Self> {
        Ok(Self { source: source.clone(), client: source_client(source.api_key.as_deref())? })
    }
}

impl SkillSourceAdapter for CustomIndexSource {
    fn source_id(&self) -> &str { &self.source.id }

    fn search<'a>(&'a self, keyword: &'a str, limit: usize) -> BoxFuture<'a, Result<Vec<SkillSearchResult>>> {
        Box::pin(async move {
            let response = self.client.get(&self.source.url).send().await?;
            if !response.status().is_success() {
                bail!("自定义索引请求失败: HTTP {}", response.status());
            }
            let payload: CustomIndexResponse = response.json().await?;
            let needle = keyword.trim().to_lowercase();
            Ok(payload.skills.into_iter()
                .filter(|item| needle.is_empty() || format!("{} {}", item.name, item.description).to_lowercase().contains(&needle))
                .take(limit)
                .map(|item| custom_index_result(&self.source.id, item))
                .collect())
        })
    }
}

pub fn adapters_for_sources(sources: &[SkillSourceConfig]) -> Result<Vec<Box<dyn SkillSourceAdapter>>> {
    sources.iter().filter(|source| source.enabled).map(|source| match source.source_type {
        crate::models::SkillSourceType::Github => Ok(Box::new(GitHubSkillSource::new(source.clone())?) as Box<dyn SkillSourceAdapter>),
        crate::models::SkillSourceType::Skillhub => Ok(Box::new(SkillHubSource::new(source.clone())?) as Box<dyn SkillSourceAdapter>),
        crate::models::SkillSourceType::CustomIndex => Ok(Box::new(CustomIndexSource::new(source.clone())?) as Box<dyn SkillSourceAdapter>),
    }).collect()
}

pub async fn search_sources(adapters: &[Box<dyn SkillSourceAdapter>], keyword: &str, limit: usize) -> Vec<SourceSearchOutcome> {
    let outcomes = join_all(adapters.iter().map(|adapter| async move {
        match adapter.search(keyword, limit).await {
            Ok(results) => SourceSearchOutcome { source_id: adapter.source_id().to_string(), results, error: None },
            Err(error) => SourceSearchOutcome { source_id: adapter.source_id().to_string(), results: Vec::new(), error: Some(error.to_string()) },
        }
    })).await;
    deduplicate_outcomes(outcomes)
}

fn deduplicate_outcomes(mut outcomes: Vec<SourceSearchOutcome>) -> Vec<SourceSearchOutcome> {
    let mut seen = HashSet::new();
    for outcome in &mut outcomes {
        outcome.results.retain(|result| seen.insert((result.source_id.clone(), result.external_id.clone())));
    }
    outcomes
}

fn source_client(api_key: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder().timeout(SEARCH_TIMEOUT).user_agent("TokenHub Skill Repository");
    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key))
            .context("技能来源 API Key 格式无效")?);
        builder = builder.default_headers(headers);
    }
    builder.build().context("创建技能来源 HTTP 客户端失败")
}

#[derive(Deserialize)]
struct GitHubCodeSearchResponse { items: Vec<GitHubCodeSearchItem> }

#[derive(Deserialize)]
struct GitHubCodeSearchItem {
    path: String,
    html_url: String,
    repository: GitHubRepository,
}

#[derive(Deserialize)]
struct GitHubRepository {
    full_name: String,
    html_url: String,
    #[serde(default)] description: Option<String>,
    #[serde(default)] updated_at: Option<DateTime<Utc>>,
    #[serde(default)] stargazers_count: Option<u64>,
    #[serde(default)] license: Option<GitHubLicense>,
}

#[derive(Deserialize)]
struct GitHubLicense { #[serde(default)] spdx_id: Option<String> }

fn github_result(source_id: &str, item: GitHubCodeSearchItem) -> Option<SkillSearchResult> {
    let path = PathParts::from_path(&item.path)?;
    let skill_directory = path.parent_path()?;
    let name = skill_directory.rsplit('/').next()?.to_string();
    Some(SkillSearchResult {
        source_id: source_id.to_string(),
        external_id: format!("{}:{}", item.repository.full_name, skill_directory),
        name,
        description: item.repository.description.unwrap_or_default(),
        author: item.repository.full_name.split('/').next().map(str::to_string),
        updated_at: item.repository.updated_at,
        license: item.repository.license.and_then(|license| license.spdx_id),
        popularity: item.repository.stargazers_count,
        version: None,
        source_url: item.html_url,
        download_locator: format!("{}/tree/HEAD/{}", item.repository.html_url, skill_directory),
    })
}

struct PathParts<'a> { path: &'a str }

impl<'a> PathParts<'a> {
    fn from_path(path: &'a str) -> Option<Self> {
        (!path.is_empty() && !path.starts_with('/') && path.split('/').all(|segment| segment != "..")).then_some(Self { path })
    }

    fn parent_path(&self) -> Option<&str> { self.path.rsplit_once('/').map(|(parent, _)| parent) }
}

#[derive(Deserialize)]
struct SkillHubSearchResponse {
    code: i32,
    message: String,
    data: SkillHubSearchData,
}

#[derive(Deserialize)]
struct SkillHubSearchData { skills: Vec<SkillHubSkill> }

#[derive(Deserialize)]
struct SkillHubSkill {
    slug: String,
    name: String,
    #[serde(default)] description: String,
    #[serde(default)] description_zh: String,
    #[serde(default)] version: Option<String>,
    #[serde(default)] downloads: Option<u64>,
    #[serde(default)] installs: Option<u64>,
    #[serde(default)] stars: Option<u64>,
    #[serde(default)] updated_at: Option<i64>,
}

fn skillhub_result(source: &SkillSourceConfig, item: SkillHubSkill) -> SkillSearchResult {
    let description = if item.description_zh.is_empty() { item.description } else { item.description_zh };
    let popularity = item.downloads.unwrap_or(0).saturating_add(item.installs.unwrap_or(0)).saturating_add(item.stars.unwrap_or(0));
    SkillSearchResult {
        source_id: source.id.clone(),
        external_id: item.slug.clone(),
        name: item.name,
        description,
        author: None,
        updated_at: item.updated_at.and_then(millis_to_datetime),
        license: None,
        popularity: Some(popularity),
        version: item.version,
        source_url: format!("https://www.skillhub.cn/skills/{}", item.slug),
        download_locator: format!("{}/api/v1/download?slug={}", source.url.trim_end_matches('/'), item.slug),
    }
}

fn millis_to_datetime(milliseconds: i64) -> Option<DateTime<Utc>> { Utc.timestamp_millis_opt(milliseconds).single() }

#[derive(Deserialize)]
struct CustomIndexResponse { #[serde(default)] skills: Vec<CustomIndexSkill> }

#[derive(Deserialize)]
struct CustomIndexSkill {
    id: String,
    name: String,
    #[serde(default)] description: String,
    source_url: String,
    download_locator: String,
    #[serde(default)] author: Option<String>,
    #[serde(default)] license: Option<String>,
    #[serde(default)] popularity: Option<u64>,
    #[serde(default)] version: Option<String>,
    #[serde(default)] updated_at: Option<DateTime<Utc>>,
}

fn custom_index_result(source_id: &str, item: CustomIndexSkill) -> SkillSearchResult {
    SkillSearchResult {
        source_id: source_id.to_string(),
        external_id: item.id,
        name: item.name,
        description: item.description,
        author: item.author,
        updated_at: item.updated_at,
        license: item.license,
        popularity: item.popularity,
        version: item.version,
        source_url: item.source_url,
        download_locator: item.download_locator,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SkillSourceType;
    use anyhow::anyhow;

    fn source(id: &str, source_type: SkillSourceType) -> SkillSourceConfig {
        SkillSourceConfig { id: id.to_string(), name: id.to_string(), source_type, url: "https://example.test".to_string(), enabled: true, last_status: None, last_checked_at: None }
    }

    #[test]
    fn github_result_normalizes_skill_directory() {
        let result = github_result("github", GitHubCodeSearchItem {
            path: "skills/review/SKILL.md".to_string(),
            html_url: "https://github.com/acme/repo/blob/main/skills/review/SKILL.md".to_string(),
            repository: GitHubRepository { full_name: "acme/repo".to_string(), html_url: "https://github.com/acme/repo".to_string(), description: Some("Skill collection".to_string()), updated_at: None, stargazers_count: Some(12), license: Some(GitHubLicense { spdx_id: Some("MIT".to_string()) }) },
        }).unwrap();
        assert_eq!(result.name, "review");
        assert_eq!(result.external_id, "acme/repo:skills/review");
        assert_eq!(result.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn skillhub_result_prefers_chinese_description() {
        let result = skillhub_result(&source("skillhub", SkillSourceType::Skillhub), SkillHubSkill {
            slug: "review".to_string(), name: "Review".to_string(), description: "English".to_string(), description_zh: "中文介绍".to_string(), version: Some("1.0.0".to_string()), downloads: Some(2), installs: Some(3), stars: Some(5), updated_at: Some(1_700_000_000_000),
        });
        assert_eq!(result.description, "中文介绍");
        assert_eq!(result.popularity, Some(10));
        assert!(result.updated_at.is_some());
    }

    struct FakeSource { id: String, results: Vec<SkillSearchResult>, error: Option<String> }

    impl SkillSourceAdapter for FakeSource {
        fn source_id(&self) -> &str { &self.id }
        fn search<'a>(&'a self, _: &'a str, _: usize) -> BoxFuture<'a, Result<Vec<SkillSearchResult>>> {
            Box::pin(async move {
                if let Some(error) = &self.error { Err(anyhow!(error.clone())) } else { Ok(self.results.clone()) }
            })
        }
    }

    fn result(source_id: &str, id: &str) -> SkillSearchResult {
        SkillSearchResult { source_id: source_id.to_string(), external_id: id.to_string(), name: id.to_string(), description: String::new(), author: None, updated_at: None, license: None, popularity: None, version: None, source_url: format!("https://example.test/{}", id), download_locator: String::new() }
    }

    #[tokio::test]
    async fn search_isolates_source_errors_and_deduplicates_results() {
        let adapters: Vec<Box<dyn SkillSourceAdapter>> = vec![
            Box::new(FakeSource { id: "first".to_string(), results: vec![result("first", "one"), result("first", "one")], error: None }),
            Box::new(FakeSource { id: "failed".to_string(), results: Vec::new(), error: Some("offline".to_string()) }),
        ];
        let outcomes = search_sources(&adapters, "skill", 10).await;
        assert_eq!(outcomes[0].results.len(), 1);
        assert!(outcomes[0].error.is_none());
        assert_eq!(outcomes[1].error.as_deref(), Some("offline"));
    }

    #[test]
    fn custom_index_result_preserves_download_locator() {
        let result = custom_index_result("custom", CustomIndexSkill { id: "one".to_string(), name: "One".to_string(), description: "Description".to_string(), source_url: "https://example.test/one".to_string(), download_locator: "https://example.test/one.zip".to_string(), author: None, license: None, popularity: None, version: None, updated_at: None });
        assert_eq!(result.download_locator, "https://example.test/one.zip");
    }
}
