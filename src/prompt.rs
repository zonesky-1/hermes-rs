use crate::skills::SkillsIndex;
use chrono::Local;
use std::fs;
use std::path::Path;

pub struct PromptBuilder {
    soul_content: String,
    skills_index: SkillsIndex,
    home_dir: String,
    cwd: String,
}

impl PromptBuilder {
    pub fn new(soul_content: String, skills_index: SkillsIndex, home_dir: String, cwd: String) -> Self {
        Self {
            soul_content,
            skills_index,
            home_dir,
            cwd,
        }
    }

    pub fn build(&self) -> String {
        let mut sections = Vec::new();

        // ── Tier 1: Stable Prefix ──
        let tier1 = self.build_tier1_stable();
        sections.push(tier1);

        // ── Tier 2: Context Tier ──
        if let Some(tier2) = self.build_tier2_context() {
            sections.push(tier2);
        }

        // ── Tier 3: Volatile Tier ──
        let tier3 = self.build_tier3_volatile();
        sections.push(tier3);

        sections.join("\n\n")
    }

    fn build_tier1_stable(&self) -> String {
        let default_rules = r#"# Finishing the job
When the user asks you to build, run, or verify something, the deliverable is a working artifact backed by real tool output — not a description of one. Do not stop after writing a stub, a plan, or a single command. Keep working until you have actually exercised the code or produced the requested result, then report what real execution returned.

# Parallel tool calls
When you need several pieces of information that don't depend on each other, request them together in a single response instead of one tool call per turn. Independent reads, searches, web fetches, and read-only commands should be batched.

# Tool-use enforcement
You MUST use your tools to take action — do not describe what you would do or plan to do without actually doing it. When you say you will perform an action (e.g. 'I will run the tests', 'Let me check the file'), you MUST immediately make the corresponding tool call in the same response. Never end your turn with a promise of future action — execute it now."#;

        format!("{}\n\n{}", self.soul_content.trim(), default_rules)
    }

    fn build_tier2_context(&self) -> Option<String> {
        let cwd_path = Path::new(&self.cwd);
        let mut context_parts = Vec::new();

        // Check for AGENTS.md
        let agents_path = cwd_path.join("AGENTS.md");
        if agents_path.is_file() {
            if let Ok(content) = fs::read_to_string(&agents_path) {
                let truncated = if content.len() > 25000 {
                    format!("{}\n... [TRUNCATED]", &content[..25000])
                } else {
                    content
                };
                context_parts.push(format!("## AGENTS.md\n\n{}", truncated));
            }
        }

        if context_parts.is_empty() {
            None
        } else {
            Some(format!("# Project Context\n\n{}", context_parts.join("\n\n")))
        }
    }

    fn build_tier3_volatile(&self) -> String {
        let mut parts = Vec::new();

        // 1. Available Skills
        let skills_xml = self.skills_index.build_prompt_xml();
        if !skills_xml.is_empty() {
            parts.push(format!("## Skills\nBefore replying, scan the skills below. If a skill matches or is relevant, use skill_view(name) to load it.\n\n{}", skills_xml));
        }

        // 2. Persistent Memory (MEMORY.md and USER.md)
        let memory_dir = Path::new(&self.home_dir).join(".hermes").join("memories");
        let memory_file = memory_dir.join("MEMORY.md");
        let user_file = memory_dir.join("USER.md");

        let mut mem_blocks = Vec::new();
        if let Ok(mem) = fs::read_to_string(&memory_file) {
            if !mem.trim().is_empty() {
                mem_blocks.push(format!("### Project & Long-term Memory\n{}", mem.trim()));
            }
        }
        if let Ok(user) = fs::read_to_string(&user_file) {
            if !user.trim().is_empty() {
                mem_blocks.push(format!("### User Profile & Preferences\n{}", user.trim()));
            }
        }
        if !mem_blocks.is_empty() {
            parts.push(format!("## Memory\n{}", mem_blocks.join("\n\n")));
        }

        // 3. Dynamic Date & Time
        let now = Local::now();
        let date_str = now.format("%A, %B %d, %Y (%Z, %:z)").to_string();
        parts.push(format!("Today's date: {}", date_str));

        // 4. Runtime Environment
        parts.push(format!(
            "# Hermes runtime environment\n\nHost: Linux (x86_64)\nUser home directory: {}\nCurrent working directory: {}\nScratch directory: {}/.hermes/cache/scratch",
            self.home_dir, self.cwd, self.home_dir
        ));

        parts.join("\n\n")
    }
}
