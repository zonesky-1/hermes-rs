use anyhow::{Context, Result};
use reqwest::{Client, Proxy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: usize,
    pub message: Message,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub choices: Vec<ChatChoice>,
}

#[derive(Clone)]
pub struct LlmClient {
    client: Client,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl LlmClient {
    pub fn new(base_url: String, api_key: String, model: String, proxy: Option<String>) -> Result<Self> {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(120));

        if let Some(proxy_url) = proxy {
            builder = builder.proxy(Proxy::all(proxy_url)?);
        }

        let client = builder.build()?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model,
        })
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let trimmed_base = self.base_url.trim_end_matches('/');
        let url = format!("{}/models", trimmed_base);

        let mut req = self.client.get(&url);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let res = req.send().await
            .with_context(|| format!("Failed to list models from {}", url))?;

        if !res.status().is_success() {
            anyhow::bail!("Failed to fetch models: HTTP {}", res.status());
        }

        let json_body: Value = res.json().await?;
        let mut model_ids = Vec::new();

        if let Some(data) = json_body.get("data").and_then(|d| d.as_array()) {
            for item in data {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    model_ids.push(id.to_string());
                }
            }
        }

        Ok(model_ids)
    }

    pub async fn chat_complete(
        &self,
        messages: &[Message],
        tools: Option<&[Value]>,
    ) -> Result<Message> {
        let trimmed_base = self.base_url.trim_end_matches('/');
        let url = format!("{}/chat/completions", trimmed_base);

        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0.7,
            "stream": false,
        });

        if let Some(tool_list) = tools {
            if !tool_list.is_empty() {
                payload["tools"] = json!(tool_list);
                payload["tool_choice"] = json!("auto");
            }
        }

        let mut req = self.client.post(&url)
            .json(&payload);

        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let res = req.send().await
            .with_context(|| format!("Failed to send request to {}", url))?;

        if !res.status().is_success() {
            let status = res.status();
            let err_body = res.text().await.unwrap_or_default();
            anyhow::bail!("API Error ({}): {}", status, err_body);
        }

        let resp: ChatCompletionResponse = res.json().await
            .context("Failed to parse chat completion response")?;

        let choice = resp.choices.into_iter().next()
            .context("No choice returned in completion response")?;

        Ok(choice.message)
    }
}
