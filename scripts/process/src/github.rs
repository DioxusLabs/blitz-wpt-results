use serde::Deserialize;
use serde_derive::{Serialize};

pub struct GithubClient {
    auth_header: String,
}

impl GithubClient {
    pub fn new(token: &str) -> Self {
        Self {
            auth_header: format!("Bearer {token}"),
        }
    }

    pub fn get(&self, url: &str) -> http::Response<ureq::Body> {
        ureq::get(url)
            .header("authorization", &self.auth_header)
            .call()
            .unwrap()
    }

    pub fn get_bytes(&self, url: &str) -> Vec<u8> {
        ureq::get(url)
            .header("authorization", &self.auth_header)
            .call()
            .unwrap()
            .into_body()
            .read_to_vec()
            .unwrap()
    }

    pub fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> T {
        self.get(&format!("https://api.github.com{url}"))
            .into_body()
            .read_json()
            .unwrap()
    }

    pub fn list_artifacts(&self, page: usize) -> ArtifactResponse {
        self.get_json::<ArtifactResponse>(&format!(
            "/repos/dioxuslabs/blitz/actions/artifacts?per_page=100&page={page}"
        ))
    }
}

#[derive(Serialize, Deserialize)]
pub struct ArtifactResponse {
    pub total_count: u64,
    pub artifacts: Vec<Artifact>,
}

#[derive(Serialize, Deserialize)]
pub struct Artifact {
    pub archive_download_url: String,
    pub created_at: String,
    pub digest: String,
    pub expired: bool,
    pub expires_at: String,
    pub id: i64,
    pub name: String,
    pub node_id: String,
    pub size_in_bytes: i64,
    pub updated_at: String,
    pub url: String,
    pub workflow_run: WorkflowRun,
}

#[derive(Serialize, Deserialize)]
pub struct WorkflowRun {
    pub head_branch: String,
    pub head_repository_id: i64,
    pub head_sha: String,
    pub id: i64,
    pub repository_id: i64,
}
