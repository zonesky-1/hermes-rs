# 🦀 Hermes Agent — Rust Edition (`hermes-rs`)

Implementasi native agent runtime Hermes menggunakan bahasa pemrograman **Rust**.
Sangat ringan (~4 MB binary), hemat memori (~4-8 MB RAM per instance), dan memiliki waktu inisialisasi instan (<15 ms).

---

## 🚀 Fitur Utama
1. **Autonomous ReAct Agent Loop**:
   * Evaluasi permintaan pengguna → Menghasilkan dan memanggil `tool_calls` secara berulang sampai tugas selesai.
2. **Standard Tools Engine**:
   * `terminal`: Eksekusi perintah bash / command line secara aman dengan timeout dan penangkapan stdout/stderr/exit code.
   * `read_file`: Membaca isi file dengan penomoran baris dan offset/limit paginasi.
   * `write_file`: Menulis / membuat file lengkap (termasuk auto-create directory).
   * `patch`: Find-and-replace tepat sasaran pada file tertentu.
3. **Multi-Profile Support**:
   * Kompatibel dengan direktori konfigurasi Hermes (`/root/.hermes/profiles/<nama_profile>/`).
   * Membaca `.env`, `config.yaml`, dan `SOUL.md` / `instructions.md`.
4. **Telegram Gateway**:
   * Long-polling connection ke Telegram Bot API via `reqwest` & `tokio`.
   * Typing action indicator di background saat agent berpikir / menjalankan tools.
   * Auto message chunking (maksimal 4.000 karakter per pesan Telegram).
   * Slash commands:
     - `/model`: Melihat model aktif & daftar model yang tersedia.
     - `/model <nama_model>`: Mengganti model aktif saat runtime.
     - `/status`: Status profile, model, message history, dan tools.
     - `/reset` / `/new`: Mengosongkan riwayat sesi chat.
     - `/help`: Bantuan perintah.

---

## 📁 Struktur Kode
* `src/main.rs`: Entry point CLI & Gateway launcher (menggunakan `clap`).
* `src/config.rs`: Parser konfigurasi profile (`config.yaml`, `.env`, `SOUL.md`).
* `src/llm.rs`: OpenAI-compatible Chat Completion API client (dengan tool definitions & models catalog).
* `src/agent.rs`: Autonomous agent turn loop (ReAct loop & session memory).
* `src/tools.rs`: Implementasi tools (`terminal`, `read_file`, `write_file`, `patch`).
* `src/telegram.rs`: Telegram gateway runner, long polling, commands handler.

---

## 🔨 Cara Build & Menjalankan

### Persyaratan:
* Rust toolchain (`rustc` & `cargo` >= 1.75)
* `libssl-dev` & `pkg-config` (atau OpenSSL)

### Compile Release:
```bash
cargo build --release
cp target/release/hermes-rs /usr/local/bin/
```

### Menjalankan CLI:
```bash
# Eksekusi task satu baris:
hermes-rs --profile redmess "Jalankan tool terminal untuk cek uptime"

# Mode interactive chat (REPL):
hermes-rs --profile syadagentic --chat
```

### Menjalankan sebagai Telegram Gateway Service:
```bash
hermes-rs --profile redmess --gateway
```
