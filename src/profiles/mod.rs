use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub profile: String,
    pub target: String,
    pub adapter: String, // e.g., "duckdb", "postgres", "snowflake"
}

impl Connection {
    /// Returns the dbt adapter package name (e.g., "dbt-duckdb")
    pub fn adapter_package(&self) -> String {
        format!("dbt-{}", self.adapter)
    }
}

impl std::fmt::Display for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{} ({})", self.profile, self.target, self.adapter)
    }
}

#[derive(Debug, Deserialize)]
struct ProfilesYaml {
    #[serde(flatten)]
    profiles: HashMap<String, Profile>,
}

#[derive(Debug, Deserialize)]
struct Profile {
    #[allow(dead_code)]
    target: Option<String>,
    outputs: HashMap<String, Output>,
}

#[derive(Debug, Deserialize)]
struct Output {
    #[serde(rename = "type")]
    db_type: Option<String>,
    // Other fields we might want later:
    // host, port, user, dbname, schema, etc.
}

fn get_profiles_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    Ok(home.join(".dbt").join("profiles.yml"))
}

pub fn load_profiles() -> Result<Vec<Connection>> {
    let path = get_profiles_path()?;
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read profiles.yml at {:?}", path))?;

    let parsed: ProfilesYaml =
        serde_yaml::from_str(&contents).context("Failed to parse profiles.yml")?;

    let mut connections = Vec::new();

    for (profile_name, profile) in parsed.profiles {
        // Skip config key if present
        if profile_name == "config" {
            continue;
        }

        for (target_name, output) in profile.outputs {
            let adapter = output.db_type.unwrap_or_else(|| "unknown".to_string());
            connections.push(Connection {
                profile: profile_name.clone(),
                target: target_name,
                adapter,
            });
        }
    }

    connections.sort_by(|a, b| {
        a.profile
            .cmp(&b.profile)
            .then_with(|| a.target.cmp(&b.target))
    });

    Ok(connections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_profiles() {
        let yaml = r#"
jaffle_shop:
  target: dev
  outputs:
    dev:
      type: postgres
      host: localhost
    prod:
      type: postgres
      host: prod.example.com

another_project:
  target: dev
  outputs:
    dev:
      type: snowflake
      account: xyz
"#;

        let parsed: ProfilesYaml = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.profiles.contains_key("jaffle_shop"));
        assert!(parsed.profiles.contains_key("another_project"));
    }
}
