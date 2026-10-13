use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub category: String,
    pub file_path: PathBuf,
}

#[derive(Clone)]
pub struct SkillsIndex {
    skills: BTreeMap<String, SkillMeta>,
    skills_dir: PathBuf,
}

impl SkillsIndex {
    pub fn new<P: AsRef<Path>>(skills_dir: P) -> Self {
        let dir = skills_dir.as_ref().to_path_buf();
        let mut index = Self {
            skills: BTreeMap::new(),
            skills_dir: dir,
        };
        index.reload();
        index
    }

    pub fn reload(&mut self) {
        self.skills.clear();
        if !self.skills_dir.exists() {
            return;
        }

        for entry in WalkDir::new(&self.skills_dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() && entry.file_name() == "SKILL.md" {
                if let Ok(meta) = Self::parse_skill_file(entry.path(), &self.skills_dir) {
                    self.skills.insert(meta.name.clone(), meta);
                }
            }
        }
    }

    fn parse_skill_file(path: &Path, root: &Path) -> Result<SkillMeta> {
        let content = fs::read_to_string(path)?;
        let (frontmatter_str, _) = Self::split_frontmatter(&content)?;
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(&frontmatter_str)?;

        let name = yaml_val
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            });

        let description = yaml_val
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();

        let rel = path.strip_prefix(root).unwrap_or(path);
        let category = if let Some(parent) = rel.parent() {
            if let Some(first_comp) = parent.components().next() {
                first_comp.as_os_str().to_string_lossy().to_string()
            } else {
                "general".to_string()
            }
        } else {
            "general".to_string()
        };

        Ok(SkillMeta {
            name,
            description,
            category,
            file_path: path.to_path_buf(),
        })
    }

    fn split_frontmatter(content: &str) -> Result<(String, String)> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return Ok(("".to_string(), content.to_string()));
        }
        let after_first = &trimmed[3..];
        if let Some(second_pos) = after_first.find("---") {
            let fm = &after_first[..second_pos].trim();
            let body = &after_first[second_pos + 3..].trim();
            Ok((fm.to_string(), body.to_string()))
        } else {
            Ok(("".to_string(), content.to_string()))
        }
    }

    pub fn build_prompt_xml(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut cat_map: BTreeMap<String, Vec<&SkillMeta>> = BTreeMap::new();
        for meta in self.skills.values() {
            cat_map.entry(meta.category.clone()).or_default().push(meta);
        }

        let mut out = String::from("<available_skills>\n");
        for (cat, list) in cat_map {
            out.push_str(&format!("  {}:\n", cat));
            for s in list {
                let desc = if s.description.len() > 70 {
                    format!("{}...", &s.description[..67])
                } else {
                    s.description.clone()
                };
                out.push_str(&format!("    - {}: {}\n", s.name, desc));
            }
        }
        out.push_str("</available_skills>");
        out
    }

    pub fn get_skill_content(&self, name: &str, file_path: Option<&str>) -> Result<String> {
        let meta = self
            .skills
            .get(name)
            .with_context(|| format!("Skill '{}' not found", name))?;

        if let Some(sub_path) = file_path {
            let base_dir = meta.file_path.parent().unwrap_or(&self.skills_dir);
            let target_path = base_dir.join(sub_path);
            if !target_path.exists() {
                anyhow::bail!("Linked file '{}' not found in skill '{}'", sub_path, name);
            }
            Ok(fs::read_to_string(target_path)?)
        } else {
            Ok(fs::read_to_string(&meta.file_path)?)
        }
    }

    pub fn list_skills_summary(&self) -> String {
        let mut lines = Vec::new();
        for s in self.skills.values() {
            lines.push(format!("• {}: {}", s.name, s.description));
        }
        lines.join("\n")
    }
}
