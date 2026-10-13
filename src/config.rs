use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelAlias {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ModelConfig {
    pub provider: Option<String>,
    pub default: Option<String>,
    pub base_url: Option<String>,
    pub aliases: Option<HashMap<String, ModelAlias>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProfileConfigYaml {
    pub model: Option<ModelConfig>,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub path: PathBuf,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub system_prompt: String,
    pub telegram_token: Option<String>,
    pub allow_all_users: bool,
    pub home_channel: Option<i64>,
    pub proxy: Option<String>,
    pub aliases: HashMap<String, ModelAlias>,
}

impl Profile {
    pub fn load(profile_name: &str) -> Result<Self> {
        let base_dir = PathBuf::from(format!("/root/.hermes/profiles/{}", profile_name));
        if !base_dir.exists() {
            anyhow::bail!("Profile path not found: {:?}", base_dir);
        }

        // 1. Read .env
        let env_file = base_dir.join(".env");
        let mut env_map = HashMap::new();
        if env_file.exists() {
            let content = fs::read_to_string(&env_file)?;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') || trimmed.is_empty() {
                    continue;
                }
                if let Some((k, v)) = trimmed.split_once('=') {
                    env_map.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }

        // 2. Read config.yaml
        let config_file = base_dir.join("config.yaml");
        let mut model_name = env_map.get("DEFAULT_MODEL").cloned().unwrap_or_else(|| "Gemini".to_string());
        let mut base_url = env_map.get("BASE_URL").cloned().unwrap_or_else(|| "http://100.79.55.60:20127/api/v1".to_string());
        let mut aliases = HashMap::new();

        if config_file.exists() {
            let content = fs::read_to_string(&config_file)?;
            if let Ok(cfg) = serde_yaml::from_str::<ProfileConfigYaml>(&content) {
                if let Some(m) = cfg.model {
                    if let Some(def) = m.default {
                        if def != "Opencode" {
                            model_name = def;
                        }
                    }
                    if let Some(b) = m.base_url {
                        base_url = b;
                    }
                    if let Some(als) = m.aliases {
                        aliases = als;
                    }
                }
            }
        }

        // Get API Key
        let api_key = env_map
            .get("ROUTER_API_KEY")
            .or_else(|| env_map.get("NINE_ROUTER_API_KEY"))
            .or_else(|| env_map.get("OPENAI_API_KEY"))
            .cloned()
            .unwrap_or_default();

        let telegram_token = env_map.get("TELEGRAM_BOT_TOKEN").cloned();
        let allow_all_users = env_map.get("TELEGRAM_ALLOW_ALL_USERS")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(true);
        let home_channel = env_map.get("TELEGRAM_HOME_CHANNEL")
            .and_then(|v| v.parse::<i64>().ok());

        // 3. Load System Prompt (SOUL.md or instructions.md)
        let mut system_prompt = "You are Hermes Agent, a sovereign and autonomous AI agent. Always be direct, technical, and helpful.".to_string();
        let soul_file = base_dir.join("SOUL.md");
        let inst_file = base_dir.join("instructions.md");
        if soul_file.exists() {
            if let Ok(text) = fs::read_to_string(&soul_file) {
                system_prompt = text;
            }
        } else if inst_file.exists() {
            if let Ok(text) = fs::read_to_string(&inst_file) {
                system_prompt = text;
            }
        }

        // Outbound HTTP proxy (configurable via .env HTTP_PROXY or environment)
        let proxy = env_map.get("HTTP_PROXY")
            .cloned()
            .or_else(|| std::env::var("HTTP_PROXY").ok())
            .or_else(|| {
                // If running in sandbox with local tailnet proxy
                if Path::new("/root/.hermes").exists() {
                    Some("http://127.0.0.1:1055".to_string())
                } else {
                    None
                }
            });

        Ok(Profile {
            name: profile_name.to_string(),
            path: base_dir,
            model: model_name,
            base_url,
            api_key,
            system_prompt,
            telegram_token,
            allow_all_users,
            home_channel,
            proxy,
            aliases,
        })
    }
}
