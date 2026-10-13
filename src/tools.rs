use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use walkdir::WalkDir;

use crate::skills::SkillsIndex;

pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub fn get_available_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "terminal".to_string(),
            description: "Execute a shell command in foreground and return stdout, stderr, and exit code.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Optional timeout in seconds (default: 60)."
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a text file with line numbers and pagination.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "1-based line number to start reading from."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to read."
                    }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file, completely replacing existing content.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write."
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write into the file."
                    }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "patch".to_string(),
            description: "Find-and-replace edits in files. Replaces old_string with new_string.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact text to find and replace."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement text."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences instead of requiring unique match."
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolDefinition {
            name: "search_files".to_string(),
            description: "Search file contents (regex) or find files by name glob.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern for content search, or glob pattern for file search."
                    },
                    "target": {
                        "type": "string",
                        "enum": ["content", "files"],
                        "description": "'content' searches inside file contents, 'files' searches by name."
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search in (default: current working directory)."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 50)."
                    }
                },
                "required": ["pattern"]
            }),
        },
        ToolDefinition {
            name: "skills_list".to_string(),
            description: "List available skills (name + description).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "skill_view".to_string(),
            description: "Load full content or linked files of a specific skill.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The skill name."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Optional: Path to linked file within the skill."
                    }
                },
                "required": ["name"]
            }),
        },
    ]
}

fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let head = &text[..max_chars / 2];
    let tail = &text[text.len() - (max_chars / 3)..];
    let omitted = text.len() - head.len() - tail.len();
    format!("{}\n\n... [OUTPUT TRUNCATED - {} characters omitted] ...\n\n{}", head, omitted, tail)
}

pub async fn execute_tool(
    name: &str,
    args: &Value,
    skills_index: Option<&SkillsIndex>,
) -> Result<String> {
    match name {
        "terminal" => {
            let cmd_str = args["command"].as_str().context("Missing 'command' field")?;
            let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(60);

            let fut = Command::new("bash")
                .arg("-c")
                .arg(cmd_str)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            match timeout(Duration::from_secs(timeout_secs), fut).await {
                Ok(Ok(output)) => {
                    let exit_code = output.status.code().unwrap_or(-1);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);

                    let combined = format!(
                        "Exit Code: {}\nSTDOUT:\n{}\nSTDERR:\n{}",
                        exit_code, stdout, stderr
                    );
                    Ok(truncate_output(&combined, 35_000))
                }
                Ok(Err(e)) => Ok(format!("Execution failed: {}", e)),
                Err(_) => Ok(format!("Command timed out after {} seconds.", timeout_secs)),
            }
        }

        "read_file" => {
            let path_str = args["path"].as_str().context("Missing 'path' field")?;
            let path = Path::new(path_str);
            if !path.exists() {
                return Ok(format!("Error: File not found at '{}'", path_str));
            }

            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => return Ok(format!("Error reading file: {}", e)),
            };

            let lines: Vec<&str> = content.lines().collect();
            let offset = args["offset"].as_u64().unwrap_or(1) as usize;
            let limit = args["limit"].as_u64().unwrap_or(2000) as usize;

            let start = if offset > 0 { offset - 1 } else { 0 };
            if start >= lines.len() {
                return Ok(format!("Offset {} exceeds total line count {}.", offset, lines.len()));
            }

            let end = (start + limit).min(lines.len());
            let mut formatted = String::new();
            for i in start..end {
                formatted.push_str(&format!("{}|{}\n", i + 1, lines[i]));
            }

            Ok(truncate_output(&formatted, 35_000))
        }

        "write_file" => {
            let path_str = args["path"].as_str().context("Missing 'path' field")?;
            let content = args["content"].as_str().context("Missing 'content' field")?;

            let path = Path::new(path_str);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(path, content)?;
            Ok(format!("Successfully wrote {} bytes to '{}'", content.len(), path_str))
        }

        "patch" => {
            let path_str = args["path"].as_str().context("Missing 'path' field")?;
            let old_str = args["old_string"].as_str().context("Missing 'old_string' field")?;
            let new_str = args["new_string"].as_str().context("Missing 'new_string' field")?;
            let replace_all = args["replace_all"].as_bool().unwrap_or(false);

            let path = Path::new(path_str);
            if !path.exists() {
                return Ok(format!("Error: File '{}' not found", path_str));
            }

            let content = fs::read_to_string(path)?;
            let matches = content.matches(old_str).count();

            if matches == 0 {
                return Ok(format!("Error: 'old_string' not found in '{}'", path_str));
            }
            if !replace_all && matches > 1 {
                return Ok(format!(
                    "Error: 'old_string' found {} times in '{}'. Must be unique or set replace_all=true.",
                    matches, path_str
                ));
            }

            let patched = if replace_all {
                content.replace(old_str, new_str)
            } else {
                content.replacen(old_str, new_str, 1)
            };

            fs::write(path, patched)?;
            Ok(format!("Successfully applied patch to '{}' ({} occurrence(s) replaced).", path_str, matches))
        }

        "search_files" => {
            let pattern = args["pattern"].as_str().context("Missing 'pattern' field")?;
            let target = args["target"].as_str().unwrap_or("content");
            let search_dir = args["path"].as_str().unwrap_or(".");
            let limit = args["limit"].as_u64().unwrap_or(50) as usize;

            let re = match Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return Ok(format!("Invalid regex pattern '{}': {}", pattern, e)),
            };

            let mut results = Vec::new();
            for entry in WalkDir::new(search_dir)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if results.len() >= limit {
                    break;
                }

                let p = entry.path();
                if target == "files" {
                    let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if re.is_match(file_name) {
                        results.push(p.display().to_string());
                    }
                } else if entry.file_type().is_file() {
                    // Content search
                    if let Ok(content) = fs::read_to_string(p) {
                        for (idx, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                results.push(format!("{}:{}: {}", p.display(), idx + 1, line.trim()));
                                if results.len() >= limit {
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if results.is_empty() {
                Ok(format!("No matches found for pattern '{}' in '{}'.", pattern, search_dir))
            } else {
                Ok(truncate_output(&results.join("\n"), 35_000))
            }
        }

        "skills_list" => {
            if let Some(index) = skills_index {
                Ok(index.list_skills_summary())
            } else {
                Ok("Skills index is not loaded.".to_string())
            }
        }

        "skill_view" => {
            let name = args["name"].as_str().context("Missing 'name' field")?;
            let file_path = args["file_path"].as_str();

            if let Some(index) = skills_index {
                match index.get_skill_content(name, file_path) {
                    Ok(c) => Ok(truncate_output(&c, 35_000)),
                    Err(e) => Ok(format!("Failed to load skill '{}': {}", name, e)),
                }
            } else {
                Ok("Skills index is not loaded.".to_string())
            }
        }

        _ => Ok(format!("Unknown tool: {}", name)),
    }
}
