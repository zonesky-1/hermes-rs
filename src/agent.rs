use anyhow::Result;
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::config::Profile;
use crate::llm::{LlmClient, Message};
use crate::prompt::PromptBuilder;
use crate::skills::SkillsIndex;
use crate::storage::SessionDb;
use crate::tools::{execute_tool, get_available_tools};

pub type ToolNotifyCallback = Box<dyn Fn(String, String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct Agent {
    pub profile: Profile,
    pub client: LlmClient,
    pub messages: Vec<Message>,
    pub tools_schema: Vec<Value>,
    pub skills_index: SkillsIndex,
    pub session_db: Option<SessionDb>,
    pub session_id: String,
    pub system_prompt: String,
}

impl Agent {
    pub fn new(profile: Profile, model_override: Option<String>, session_id: Option<String>) -> Result<Self> {
        let model = model_override.unwrap_or_else(|| profile.model.clone());
        let client = LlmClient::new(
            profile.base_url.clone(),
            profile.api_key.clone(),
            model.clone(),
            profile.proxy.clone(),
        )?;

        let sid = session_id.unwrap_or_else(|| {
            format!("session_{}", chrono::Utc::now().timestamp_millis())
        });

        // 1. Resolve Skills Directory
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        let profile_skills = profile.path.join("skills");
        let global_skills = Path::new(&home_dir).join(".hermes").join("skills");
        let fallback_skills = Path::new(&home_dir).join(".hermes").join("profiles").join("redmess").join("skills");
        let skills_dir = if profile_skills.exists() {
            profile_skills
        } else if global_skills.exists() {
            global_skills
        } else {
            fallback_skills
        };
        let skills_index = SkillsIndex::new(&skills_dir);

        // 2. Build 3-Tier System Prompt (Stable -> Context -> Volatile)
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/root".to_string());
        let prompt_builder = PromptBuilder::new(
            profile.system_prompt.clone(),
            skills_index.clone(),
            home_dir,
            cwd,
        );
        let full_system_prompt = prompt_builder.build();

        // 3. Connect to SQLite SessionDB
        let db_path = profile.path.join("state.db");
        let session_db = match SessionDb::open(&db_path) {
            Ok(db) => {
                let _ = db.ensure_session(&sid, "telegram", &model);
                Some(db)
            }
            Err(e) => {
                eprintln!("⚠️ Warning: Failed to open state.db at {:?}: {}", db_path, e);
                None
            }
        };

        // 4. Initial System Message
        let mut messages = Vec::new();
        messages.push(Message {
            role: "system".to_string(),
            content: Some(full_system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        });

        // 5. Convert available tools into OpenAI JSON schema format
        let tools_schema: Vec<Value> = get_available_tools()
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();

        Ok(Self {
            profile,
            client,
            messages,
            tools_schema,
            skills_index,
            session_db,
            session_id: sid,
            system_prompt: full_system_prompt,
        })
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.messages.push(Message {
            role: "system".to_string(),
            content: Some(self.system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
        if let Some(ref db) = self.session_db {
            let _ = db.clear_messages(&self.session_id);
        }
    }

    pub fn switch_model(&mut self, new_model: &str) {
        self.client.set_model(new_model.to_string());
        if let Some(ref db) = self.session_db {
            let _ = db.ensure_session(&self.session_id, "telegram", new_model);
        }
    }

    pub fn current_model(&self) -> &str {
        &self.client.model
    }

    pub async fn run_turn(&mut self, user_input: &str, max_steps: usize) -> Result<String> {
        self.run_turn_internal(user_input, max_steps, None).await
    }

    pub async fn run_turn_with_notify(
        &mut self,
        user_input: &str,
        max_steps: usize,
        notify_cb: Option<&ToolNotifyCallback>,
    ) -> Result<String> {
        self.run_turn_internal(user_input, max_steps, notify_cb).await
    }

    async fn run_turn_internal(
        &mut self,
        user_input: &str,
        max_steps: usize,
        notify_cb: Option<&ToolNotifyCallback>,
    ) -> Result<String> {
        println!("\n👤 User: {}", user_input);
        self.messages.push(Message {
            role: "user".to_string(),
            content: Some(user_input.to_string()),
            tool_calls: None,
            tool_call_id: None,
        });

        if let Some(ref db) = self.session_db {
            let _ = db.append_message(&self.session_id, "user", Some(user_input), None, None, None, None);
        }

        let mut step = 0;
        loop {
            step += 1;
            if step > max_steps {
                return Ok("Reached maximum autonomous steps without finishing.".to_string());
            }

            // Call LLM
            let response = self.client.chat_complete(&self.messages, Some(&self.tools_schema)).await?;

            // Check if model called any tools
            if let Some(tool_calls) = response.tool_calls.clone() {
                if !tool_calls.is_empty() {
                    // Record in DB & memory
                    let tc_json = serde_json::to_string(&tool_calls).unwrap_or_default();
                    if let Some(ref db) = self.session_db {
                        let _ = db.append_message(
                            &self.session_id,
                            "assistant",
                            response.content.as_deref(),
                            None,
                            Some(&tc_json),
                            None,
                            None,
                        );
                    }
                    self.messages.push(response);

                    for tc in tool_calls {
                        let fn_name = &tc.function.name;
                        let fn_args: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(json!({}));

                        println!("⚡ [Tool Call] {} -> {}", fn_name, fn_args);

                        if let Some(cb) = notify_cb {
                            let desc = format!("🛠️ Running tool: `{}`", fn_name);
                            cb(fn_name.clone(), desc).await;
                        }

                        let tool_result = match execute_tool(fn_name, &fn_args, Some(&self.skills_index)).await {
                            Ok(res) => res,
                            Err(e) => format!("Error executing tool: {}", e),
                        };

                        println!("📥 [Tool Result] {} chars output", tool_result.len());

                        if let Some(ref db) = self.session_db {
                            let _ = db.append_message(
                                &self.session_id,
                                "tool",
                                Some(&tool_result),
                                Some(&tc.id),
                                None,
                                Some(fn_name),
                                None,
                            );
                        }

                        // Append tool result message
                        self.messages.push(Message {
                            role: "tool".to_string(),
                            content: Some(tool_result),
                            tool_calls: None,
                            tool_call_id: Some(tc.id),
                        });
                    }
                    continue; // Loop back to let the LLM evaluate tool results
                }
            }

            // If no tool calls, this is the final assistant response
            let final_content = response.content.clone().unwrap_or_default();
            println!("\n🤖 Hermes: {}\n", final_content);

            if let Some(ref db) = self.session_db {
                let _ = db.append_message(
                    &self.session_id,
                    "assistant",
                    Some(&final_content),
                    None,
                    None,
                    None,
                    None,
                );
            }

            self.messages.push(response);
            return Ok(final_content);
        }
    }
}
