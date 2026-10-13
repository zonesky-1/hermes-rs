use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::agent::Agent;
use crate::config::Profile;

pub struct TelegramGateway {
    token: String,
    client: Client,
    profile: Profile,
    sessions: Arc<Mutex<HashMap<i64, Agent>>>,
}

impl TelegramGateway {
    pub fn new(profile: Profile) -> Result<Self> {
        let token = profile
            .telegram_token
            .clone()
            .context("Profile does not have TELEGRAM_BOT_TOKEN set in .env")?;

        let client = Client::builder().timeout(Duration::from_secs(60)).build()?;

        Ok(Self {
            token,
            client,
            profile,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<()> {
        let chunks = split_message(text, 28000);
        for chunk in chunks {
            let rich_url = self.api_url("sendRichMessage");
            let rich_payload = json!({
                "chat_id": chat_id,
                "rich_message": {
                    "markdown": chunk
                }
            });

            let res = self.client.post(&rich_url).json(&rich_payload).send().await;
            let mut delivered = false;
            if let Ok(resp) = res {
                if let Ok(v) = resp.json::<Value>().await {
                    if v["ok"].as_bool() == Some(true) {
                        delivered = true;
                    }
                }
            }

            if !delivered {
                // Fallback to sendMessage Markdown
                let send_url = self.api_url("sendMessage");
                let fallback_payload = json!({
                    "chat_id": chat_id,
                    "text": chunk,
                    "parse_mode": "Markdown"
                });
                let fb_res = self.client.post(&send_url).json(&fallback_payload).send().await;
                let mut fb_ok = false;
                if let Ok(resp) = fb_res {
                    if let Ok(v) = resp.json::<Value>().await {
                        if v["ok"].as_bool() == Some(true) {
                            fb_ok = true;
                        }
                    }
                }
                if !fb_ok {
                    // Final plain fallback without parse_mode
                    let plain_payload = json!({
                        "chat_id": chat_id,
                        "text": chunk
                    });
                    let _ = self.client.post(&send_url).json(&plain_payload).send().await;
                }
            }
        }
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        println!("🚀 Starting Telegram Gateway for profile '{}'...", self.profile.name);

        // Verify token via getMe
        let me_url = self.api_url("getMe");
        let res = self.client.get(&me_url).send().await?;
        let me_json: Value = res.json().await?;
        let bot_user = me_json["result"]["username"].as_str().unwrap_or("unknown_bot");
        println!("🤖 Connected to Telegram as @{}", bot_user);

        let mut offset = 0i64;

        loop {
            let updates_url = self.api_url("getUpdates");
            let req = self.client.post(&updates_url).json(&json!({
                "offset": offset,
                "timeout": 30,
            }));

            let res = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("⚠️ Telegram network error: {}. Retrying in 3s...", e);
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let body: Value = match res.json().await {
                Ok(b) => b,
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };

            if let Some(updates) = body["result"].as_array() {
                for update in updates {
                    if let Some(update_id) = update["update_id"].as_i64() {
                        offset = update_id + 1;
                    }

                    if let Some(msg) = update.get("message") {
                        let chat_id = match msg["chat"]["id"].as_i64() {
                            Some(id) => id,
                            None => continue,
                        };

                        let user_id = msg["from"]["id"].as_i64().unwrap_or(chat_id);
                        let text = msg["text"].as_str().unwrap_or("").trim();

                        if text.is_empty() {
                            continue;
                        }

                        // Check authorization
                        if !self.profile.allow_all_users {
                            if let Some(home) = self.profile.home_channel {
                                if user_id != home {
                                    let _ = self.send_message(chat_id, "⛔ Unauthorized user.").await;
                                    continue;
                                }
                            }
                        }

                        // Handle Commands
                        if text.starts_with('/') {
                            self.handle_command(chat_id, text).await?;
                            continue;
                        }

                        // Regular Message -> Run Agent Turn
                        self.handle_user_message(chat_id, text).await?;
                    }
                }
            }
        }
    }

    async fn handle_command(&self, chat_id: i64, text: &str) -> Result<()> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let cmd = cmd.split('@').next().unwrap_or("");

        match cmd {
            "/start" | "/help" => {
                let help_text = format!(
                    "👋 **Hermes Agent (Rust Edition — Nous Parity)**\n\n\
                    • **Profile:** `{}`\n\
                    • **Base URL:** `{}`\n\n\
                    **Available Commands:**\n\
                    • `/model` — Lihat model yang aktif & daftar rekomendasi\n\
                    • `/model <nama_model>` — Ganti model runtime (misal: `/model Gemini`)\n\
                    • `/skills` — Daftar skills yang terdeteksi di sistem\n\
                    • `/status` — Cek status agent, database state.db, dan token\n\
                    • `/reset` atau `/new` — Reset riwayat percakapan sesi ini\n\
                    • `/help` — Menampilkan bantuan ini\n\n\
                    Kirim pesan apa saja untuk mulai menjalankan agent & tools autonomous!",
                    self.profile.name, self.profile.base_url
                );
                self.send_message(chat_id, &help_text).await?;
            }
            "/status" => {
                let mut sessions = self.sessions.lock().await;
                let profile = self.profile.clone();
                let agent = sessions.entry(chat_id).or_insert_with(|| {
                    Agent::new(profile, None, Some(format!("tg_{}", chat_id))).unwrap()
                });

                let msg_count = agent.messages.len();
                let cur_model = agent.current_model();

                let status_text = format!(
                    "📊 **Agent Status (Hermes Nous Parity)**\n\n\
                    • **Profile:** `{}`\n\
                    • **Active Model:** `{}`\n\
                    • **Session ID:** `{}`\n\
                    • **Active Messages:** `{}`\n\
                    • **Database Storage:** SQLite WAL (`state.db`) Active\n\
                    • **Tools Available:** `terminal`, `read_file`, `write_file`, `patch`, `search_files`, `skills_list`, `skill_view`\n\
                    • **Runtime:** Rust Native (hermes-rs v0.2.0)",
                    self.profile.name, cur_model, agent.session_id, msg_count
                );
                self.send_message(chat_id, &status_text).await?;
            }
            "/skills" => {
                let mut sessions = self.sessions.lock().await;
                let profile = self.profile.clone();
                let agent = sessions.entry(chat_id).or_insert_with(|| {
                    Agent::new(profile, None, Some(format!("tg_{}", chat_id))).unwrap()
                });

                let summary = agent.skills_index.list_skills_summary();
                let reply = if summary.is_empty() {
                    "Belum ada skill yang terpasang di direktori skills.".to_string()
                } else {
                    let head: Vec<&str> = summary.lines().take(40).collect();
                    format!(
                        "📚 **Daftar Skills Terpasang (Total {} lines):**\n\n{}\n\n_(Ketik 'skill_view <name>' atau tanya topik terkait untuk menggunakan skill)_",
                        summary.lines().count(),
                        head.join("\n")
                    )
                };
                self.send_message(chat_id, &reply).await?;
            }
            "/reset" | "/new" => {
                let mut sessions = self.sessions.lock().await;
                if let Some(agent) = sessions.get_mut(&chat_id) {
                    agent.reset();
                }
                self.send_message(chat_id, "🧹 Sesi percakapan telah di-reset ke kondisi awal.").await?;
            }
            "/model" | "/models" => {
                let mut sessions = self.sessions.lock().await;
                let profile = self.profile.clone();
                let agent = sessions.entry(chat_id).or_insert_with(|| {
                    Agent::new(profile, None, Some(format!("tg_{}", chat_id))).unwrap()
                });

                if parts.len() > 1 {
                    let new_model = parts[1];
                    agent.switch_model(new_model);
                    self.send_message(chat_id, &format!("✅ Model aktif berhasil diubah menjadi: `{}`", new_model)).await?;
                } else {
                    let cur_model = agent.current_model();
                    let remote_models = agent.client.list_models().await.unwrap_or_default();
                    let sample_remote: Vec<&str> = remote_models.iter().take(12).map(|s| s.as_str()).collect();

                    let text = format!(
                        "🤖 **Konfigurasi Model**\n\n\
                        • **Model Aktif:** `{}`\n\n\
                        **Daftar Model Rekomendasi:**\n\
                        • `Gemini` (Sangat stabil & responsif)\n\
                        • `Opencode`\n\
                        • `Claude`\n\
                        • `cbai/deepseek-v4.1-flash`\n\
                        • `cbai/glm-5.2`\n\
                        • `oc/mimo-v2.6-flash-free`\n\n\
                        **Model Lain dari Router (Total {}):**\n{}\n\n\
                        Untuk mengganti model, ketik:\n`/model <nama_model>`",
                        cur_model, remote_models.len(), sample_remote.join(", ")
                    );
                    self.send_message(chat_id, &text).await?;
                }
            }
            _ => {
                self.send_message(chat_id, "❓ Perintah tidak dikenal. Ketik /help untuk daftar perintah.").await?;
            }
        }
        Ok(())
    }

    async fn handle_user_message(&self, chat_id: i64, user_text: &str) -> Result<()> {
        let token = self.token.clone();
        let client = self.client.clone();
        let typing_handle = tokio::spawn(async move {
            let url = format!("https://api.telegram.org/bot{}/sendChatAction", token);
            let payload = json!({
                "chat_id": chat_id,
                "action": "typing",
            });
            loop {
                let _ = client.post(&url).json(&payload).send().await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });

        let mut sessions = self.sessions.lock().await;
        let profile = self.profile.clone();
        let agent = sessions.entry(chat_id).or_insert_with(|| {
            Agent::new(profile, None, Some(format!("tg_{}", chat_id))).unwrap()
        });

        let result = agent.run_turn(user_text, 15).await;

        typing_handle.abort();

        match result {
            Ok(reply) => {
                self.send_message(chat_id, &reply).await?;
            }
            Err(e) => {
                let err_msg = format!("❌ Error during execution: {}", e);
                self.send_message(chat_id, &err_msg).await?;
            }
        }

        Ok(())
    }
}

fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() + 1 > max_len {
            if !current.is_empty() {
                chunks.push(current);
                current = String::new();
            }
            if line.len() > max_len {
                let mut start = 0;
                while start < line.len() {
                    let end = (start + max_len).min(line.len());
                    chunks.push(line[start..end].to_string());
                    start = end;
                }
                continue;
            }
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}
