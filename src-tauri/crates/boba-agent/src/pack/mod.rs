//! The Boba agent pack, rendered per run (FR-18, NFR-21, architecture §5.5).
//!
//! `agent-pack/system.md` plus one Agent Skill (`SKILL.md`) per task are the
//! prompts. They stay readable files in the repo (NFR-21) and are embedded at
//! build time, so a contributor edits a prompt without touching Rust. For each
//! run the pack is rendered into the run's workspace as the instruction file the
//! CLI actually reads — `CLAUDE.md`, `AGENTS.md` or `GEMINI.md` — with the run's
//! `target_language` filled in and its provenance recorded (AC1, AC2).

use std::io;
use std::path::{Path, PathBuf};

use boba_app::ports::agent::AgentTaskKind;

use crate::drivers::CliKind;

/// The template variable the pack fills in: the run's content language
/// (BCP-47, AD-17).
pub const TARGET_LANGUAGE: &str = "{{target_language}}";

/// The pack's system prompt (architecture §5.5).
const SYSTEM_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../agent-pack/system.md"
));

/// Every skill the pack ships, by directory name and embedded source. Adding a
/// skill is adding a line here; [`version`] hashes them all, so adding one is
/// also a new pack version.
const SKILLS: [(&str, &str); 9] = [
    (
        "answer_questions",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/answer_questions/SKILL.md"
        )),
    ),
    (
        "classify_emails",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/classify_emails/SKILL.md"
        )),
    ),
    (
        "extract_postings",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/extract_postings/SKILL.md"
        )),
    ),
    (
        "narrate_scores",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/narrate_scores/SKILL.md"
        )),
    ),
    (
        "profile_gaps",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/profile_gaps/SKILL.md"
        )),
    ),
    (
        "profile_import",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/profile_import/SKILL.md"
        )),
    ),
    (
        "review_application",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/review_application/SKILL.md"
        )),
    ),
    (
        "tailor_application",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/tailor_application/SKILL.md"
        )),
    ),
    (
        "translate_profile",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../agent-pack/skills/translate_profile/SKILL.md"
        )),
    ),
];

/// The file a run's CLI reads as its instructions (architecture §5.5).
pub const fn instruction_file(cli: CliKind) -> &'static str {
    match cli {
        CliKind::ClaudeCode => "CLAUDE.md",
        CliKind::GeminiCli => "GEMINI.md",
        CliKind::Codex | CliKind::OpenCode | CliKind::Aider => "AGENTS.md",
    }
}

/// The task's Agent Skill, embedded from `agent-pack/skills/<task id>/SKILL.md`.
pub fn skill(task: &str) -> &'static str {
    SKILLS
        .iter()
        .find(|(name, _)| *name == task)
        .map(|(_, text)| *text)
        .unwrap_or_else(|| panic!("no SKILL.md for `{task}`"))
}

/// A run's rendered pack: the file the CLI reads, and its contents. Not a file
/// on disk yet — [`write_into`] puts it in the run's workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPack {
    pub file_name: &'static str,
    pub contents: String,
}

/// Render `system.md` plus `task`'s skill for a run on `cli`, with the run's
/// content language filled in and its provenance recorded (AC1, AC2).
pub fn render(
    task: AgentTaskKind,
    cli: CliKind,
    model: Option<&str>,
    target_language: &str,
) -> RenderedPack {
    let mut contents = String::new();
    contents.push_str(&fill(SYSTEM_MD, target_language));
    contents.push_str("\n\n---\n\n");
    contents.push_str(&fill(skill(task.id()), target_language));
    contents.push_str(&provenance(cli, model, target_language));
    RenderedPack {
        file_name: instruction_file(cli),
        contents,
    }
}

/// Fill the run's content language into a prompt. `{{target_language}}` is the
/// pack's only template variable, so a plain replace is the whole renderer.
fn fill(template: &str, target_language: &str) -> String {
    template.replace(TARGET_LANGUAGE, target_language)
}

/// The footer every rendered pack carries: the pack version plus the CLI and
/// model the run is on, so a stored artifact traces back to this exact pack and
/// engine (AC2). An HTML comment: machine-readable, and never shown to the user.
fn provenance(cli: CliKind, model: Option<&str>, target_language: &str) -> String {
    format!(
        "\n<!-- boba agent pack {} | cli {} | model {} | target_language {} -->\n",
        version(),
        cli.id(),
        model.unwrap_or("unknown"),
        target_language
    )
}

/// Render the pack into a run's workspace (AC1). The runtime calls this when it
/// prepares a run, so the CLI finds its own instruction file there.
pub fn write_into(
    dir: &Path,
    task: AgentTaskKind,
    cli: CliKind,
    model: Option<&str>,
    target_language: &str,
) -> io::Result<PathBuf> {
    let rendered = render(task, cli, model, target_language);
    let path = dir.join(rendered.file_name);
    std::fs::write(&path, rendered.contents)?;
    Ok(path)
}

/// The pack's version: a hash of the embedded prompt text, so any edit to a
/// prompt is a new version and a stored artifact can be traced back to the exact
/// prompts that produced it (AC2, NFR-21).
pub fn version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        let mut hash = fnv1a(FNV_OFFSET, SYSTEM_MD.as_bytes());
        for (_, text) in SKILLS {
            hash = fnv1a(hash, text.as_bytes());
        }
        format!("{hash:016x}")
    })
}

/// FNV-1a, 64-bit: small, dependency-free and stable across builds.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks;

    #[test]
    fn the_pack_is_rendered_into_the_clis_native_instruction_file() {
        for (cli, file) in [
            (CliKind::ClaudeCode, "CLAUDE.md"),
            (CliKind::GeminiCli, "GEMINI.md"),
            (CliKind::Codex, "AGENTS.md"),
            (CliKind::OpenCode, "AGENTS.md"),
            (CliKind::Aider, "AGENTS.md"),
        ] {
            let rendered = render(AgentTaskKind::ExtractPostings, cli, Some("sonnet"), "en");
            assert_eq!(rendered.file_name, file, "{}", cli.display_name());
            assert!(
                rendered.contents.contains("Boba Job Find"),
                "{} reads no system prompt: {}",
                cli.display_name(),
                rendered.contents
            );
            assert!(
                rendered.contents.contains("# extract_postings"),
                "{} got no skill: {}",
                cli.display_name(),
                rendered.contents
            );
            // Only this task's skill: a leak of another task's prompts would
            // tell the agent to produce things this run cannot even save.
            assert!(
                !rendered.contents.contains("# narrate_scores"),
                "{} leaked another task's skill",
                cli.display_name()
            );
        }
        for cli in CliKind::ALL {
            assert!(!instruction_file(cli).is_empty(), "{}", cli.id());
        }
    }

    #[test]
    fn the_runs_target_language_is_filled_in() {
        // The template really carries the placeholder, so a prompt edit that
        // drops it fails here instead of shipping an unfilled pack.
        assert!(SYSTEM_MD.contains(TARGET_LANGUAGE));

        let rendered = render(
            AgentTaskKind::NarrateScores,
            CliKind::ClaudeCode,
            None,
            "es-MX",
        );
        assert!(
            !rendered.contents.contains(TARGET_LANGUAGE),
            "the placeholder survived: {}",
            rendered.contents
        );
        assert!(
            rendered.contents.contains("`es-MX`"),
            "the language did not arrive: {}",
            rendered.contents
        );
    }

    #[test]
    fn a_rendered_pack_stores_the_version_the_cli_and_the_model() {
        let rendered = render(
            AgentTaskKind::TailorApplication,
            CliKind::ClaudeCode,
            Some("opus"),
            "en",
        );
        assert!(
            rendered.contents.contains(version()),
            "{}",
            rendered.contents
        );
        assert!(
            rendered.contents.contains("cli claude_code"),
            "{}",
            rendered.contents
        );
        assert!(
            rendered.contents.contains("model opus"),
            "{}",
            rendered.contents
        );

        // A run with no model still records the field rather than dropping it.
        let unknown = render(AgentTaskKind::TailorApplication, CliKind::Aider, None, "en");
        assert!(
            unknown.contents.contains("model unknown"),
            "{}",
            unknown.contents
        );
        assert!(
            unknown.contents.contains("cli aider"),
            "{}",
            unknown.contents
        );
    }

    #[test]
    fn the_version_is_a_stable_content_hash() {
        assert_eq!(version().len(), 16, "{:?}", version());
        assert!(version().chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(version(), version());
        // Same bytes, same hash; different bytes, different hash.
        assert_eq!(fnv1a(FNV_OFFSET, b"pack"), fnv1a(FNV_OFFSET, b"pack"));
        assert_ne!(fnv1a(FNV_OFFSET, b"pack"), fnv1a(FNV_OFFSET, b"packs"));
    }

    #[test]
    fn writing_the_pack_lands_in_the_runs_workspace() {
        let dir = std::env::temp_dir().join(format!("boba-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = write_into(
            &dir,
            AgentTaskKind::ProfileImport,
            CliKind::GeminiCli,
            Some("flash"),
            "es",
        )
        .expect("write the pack");
        assert_eq!(path, dir.join("GEMINI.md"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            render(
                AgentTaskKind::ProfileImport,
                CliKind::GeminiCli,
                Some("flash"),
                "es"
            )
            .contents
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn every_task_in_the_registry_has_a_skill() {
        for task in tasks::TASKS {
            assert!(
                SKILLS.iter().any(|(name, _)| *name == task),
                "{task} has no SKILL.md"
            );
            assert!(skill(task).contains(task), "{task}");
        }
        assert!(SKILLS.iter().all(|(_, text)| !text.trim().is_empty()));
    }
}
