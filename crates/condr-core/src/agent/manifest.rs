//! Manifest-driven agent state detection.
//!
//! Ported from herdr `src/detect/manifest.rs` (commit 5158adab, Apache-2.0) without the
//! remote manifest download, the `explain` diagnostics and the reload API. Each agent
//! ships a TOML manifest of rules; a rule names a screen `region`, a `state`, a
//! `priority` and a gate of matchers. The highest-priority matching rule wins; no
//! match on a known agent means `Idle`. A local override at
//! `<config dir>/agent-detection/<id>.toml` replaces the bundled manifest.

use std::path::PathBuf;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use super::{AgentDetection, AgentKind, AgentState};

/// The screen snapshot plus the OSC strings the terminal retained. Empty OSC strings
/// simply match nothing.
#[derive(Debug, Clone, Copy)]
pub struct DetectionInput<'a> {
    pub screen: &'a str,
    pub osc_title: &'a str,
    pub osc_progress: &'a str,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct AgentManifest {
    id: String,
    #[serde(rename = "version")]
    _version: Option<String>,
    min_engine_version: Option<u32>,
    #[serde(rename = "updated_at")]
    _updated_at: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    rules: Vec<ManifestRule>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct ManifestRule {
    id: String,
    state: Option<ManifestState>,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default)]
    visible_idle: bool,
    #[serde(default)]
    visible_blocker: bool,
    #[serde(default)]
    visible_working: bool,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<ManifestGate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
struct ManifestGate {
    #[serde(default)]
    all: Vec<ManifestGate>,
    #[serde(default)]
    any: Vec<ManifestGate>,
    #[serde(default, rename = "not")]
    not_gate: Vec<ManifestGate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ManifestState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl From<ManifestState> for AgentState {
    fn from(value: ManifestState) -> Self {
        match value {
            ManifestState::Idle => AgentState::Idle,
            ManifestState::Working => AgentState::Working,
            ManifestState::Blocked => AgentState::Blocked,
            ManifestState::Unknown => AgentState::Unknown,
        }
    }
}

fn default_region() -> String {
    "whole_recent".to_string()
}

#[derive(Debug)]
struct CompiledGate {
    all: Vec<CompiledGate>,
    any: Vec<CompiledGate>,
    not_gate: Vec<CompiledGate>,
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

#[derive(Debug)]
struct LoadedManifest {
    rules: Vec<(ManifestRule, CompiledGate)>,
}

/// The manifests herdr ships, keyed by agent id. Kept verbatim so they can be synced
/// with upstream; see `manifests/README.md`.
const BUNDLED_MANIFESTS: &[(&str, &str)] = &[
    ("amp", include_str!("manifests/amp.toml")),
    ("agy", include_str!("manifests/antigravity.toml")),
    ("claude", include_str!("manifests/claude.toml")),
    ("cline", include_str!("manifests/cline.toml")),
    ("codex", include_str!("manifests/codex.toml")),
    ("cursor", include_str!("manifests/cursor.toml")),
    ("devin", include_str!("manifests/devin.toml")),
    ("droid", include_str!("manifests/droid.toml")),
    ("gemini", include_str!("manifests/gemini.toml")),
    ("grok", include_str!("manifests/grok.toml")),
    ("hermes", include_str!("manifests/hermes.toml")),
    ("kilo", include_str!("manifests/kilo.toml")),
    ("kimi", include_str!("manifests/kimi.toml")),
    ("kiro", include_str!("manifests/kiro.toml")),
    ("maki", include_str!("manifests/maki.toml")),
    ("muse", include_str!("manifests/muse.toml")),
    ("opencode", include_str!("manifests/opencode.toml")),
    ("pi", include_str!("manifests/pi.toml")),
    ("qodercli", include_str!("manifests/qodercli.toml")),
    ("qwen", include_str!("manifests/qwen.toml")),
    ("copilot", include_str!("manifests/github-copilot.toml")),
];

/// The newest manifest feature this engine understands; a manifest asking for more
/// is rejected rather than silently misread.
const MANIFEST_ENGINE_VERSION: u32 = 3;
const TOP_NON_EMPTY_LINES_ENGINE_VERSION: u32 = 3;
const MAX_TOP_REGION_LINE_COUNT: usize = u16::MAX as usize;

const MAX_RULES_PER_MANIFEST: usize = 128;
const MAX_GATE_DEPTH: usize = 8;
const MAX_TOTAL_GATES: usize = 512;
const MAX_MATCHERS_PER_GATE: usize = 32;
const MAX_TOTAL_MATCHERS: usize = 1024;
const MAX_MATCHER_CHARS: usize = 512;

static MANIFESTS: OnceLock<Vec<(AgentKind, Option<LoadedManifest>)>> = OnceLock::new();

fn manifests() -> &'static [(AgentKind, Option<LoadedManifest>)] {
    MANIFESTS.get_or_init(|| {
        AgentKind::SCREEN_MANIFEST_AGENTS
            .into_iter()
            .map(|agent| (agent, load_manifest(agent)))
            .collect()
    })
}

fn loaded_manifest(agent: AgentKind) -> Option<&'static LoadedManifest> {
    manifests()
        .iter()
        .find(|(candidate, _)| *candidate == agent)
        .and_then(|(_, loaded)| loaded.as_ref())
}

/// Classifies one screen for `agent`. Known agents whose manifest matches nothing are
/// `Idle`; an agent identified only by process (herdr drives those through lifecycle
/// hooks) stays `Unknown` rather than claiming an Idle nobody observed. The `visible_*`
/// flags only survive when they agree with the state.
pub fn detect(agent: AgentKind, input: DetectionInput<'_>) -> AgentDetection {
    let Some(loaded) = loaded_manifest(agent) else {
        return AgentDetection {
            state: AgentState::Unknown,
            ..AgentDetection::idle_fallback()
        };
    };
    let mut matched: Option<&ManifestRule> = None;
    for (rule, gate) in &loaded.rules {
        if !gate_matches(gate, region(input, &rule.region)) {
            continue;
        }
        match matched {
            Some(previous) if previous.priority >= rule.priority => {}
            _ => matched = Some(rule),
        }
    }
    let Some(rule) = matched else {
        return AgentDetection::idle_fallback();
    };
    let state = rule
        .state
        .map(AgentState::from)
        .unwrap_or(AgentState::Unknown);
    AgentDetection {
        state,
        skip_state_update: rule.skip_state_update,
        visible_idle: rule.visible_idle && state == AgentState::Idle,
        visible_blocker: rule.visible_blocker && state == AgentState::Blocked,
        visible_working: rule.visible_working && state == AgentState::Working,
    }
}

fn load_manifest(agent: AgentKind) -> Option<LoadedManifest> {
    let id = agent.id();
    let bundled = BUNDLED_MANIFESTS
        .iter()
        .find(|(manifest_id, _)| *manifest_id == id)
        .map(|(_, content)| {
            parse_manifest(content)
                .and_then(|manifest| compile_manifest(&manifest))
                .unwrap_or_else(|error| panic!("bundled {id} manifest is invalid: {error}"))
        })?;
    let Some(path) = override_path(agent) else {
        return Some(bundled);
    };
    if !path.exists() {
        return Some(bundled);
    }
    let overridden = std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|content| parse_manifest(&content))
        .and_then(|manifest| {
            if manifest_matches_agent(&manifest, agent) {
                compile_manifest(&manifest)
            } else {
                Err(format!("manifest id {} does not match {id}", manifest.id))
            }
        });
    match overridden {
        Ok(loaded) => Some(loaded),
        Err(error) => {
            eprintln!(
                "condr: ignoring agent detection override {}: {error}",
                path.display()
            );
            Some(bundled)
        }
    }
}

fn override_path(agent: AgentKind) -> Option<PathBuf> {
    crate::config_directory().map(|root| {
        root.join("agent-detection")
            .join(format!("{}.toml", agent.id()))
    })
}

fn manifest_matches_agent(manifest: &AgentManifest, agent: AgentKind) -> bool {
    let id = agent.id();
    manifest.id == id
        || manifest.aliases.iter().any(|alias| alias == id)
        || AgentKind::parse_label(&manifest.id) == Some(agent)
        || manifest
            .aliases
            .iter()
            .any(|alias| AgentKind::parse_label(alias) == Some(agent))
}

fn parse_manifest(content: &str) -> Result<AgentManifest, String> {
    let manifest = toml::from_str::<AgentManifest>(content).map_err(|error| error.to_string())?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &AgentManifest) -> Result<(), String> {
    if let Some(required) = manifest.min_engine_version
        && required > MANIFEST_ENGINE_VERSION
    {
        return Err(format!(
            "manifest requires engine {required}, current engine is {MANIFEST_ENGINE_VERSION}"
        ));
    }
    if manifest.rules.is_empty() {
        return Err("manifest must contain at least one rule".to_string());
    }
    if manifest.rules.len() > MAX_RULES_PER_MANIFEST {
        return Err(format!(
            "manifest contains {} rules, max is {MAX_RULES_PER_MANIFEST}",
            manifest.rules.len()
        ));
    }

    let mut complexity = ManifestComplexity::default();
    for rule in &manifest.rules {
        if rule.id.trim().is_empty() {
            return Err("manifest rule id must not be empty".to_string());
        }
        if rule.skip_state_update {
            if rule.state != Some(ManifestState::Unknown) {
                return Err(format!(
                    "rule {} uses skip_state_update without state = \"unknown\"",
                    rule.id
                ));
            }
            if rule.visible_idle || rule.visible_blocker || rule.visible_working {
                return Err(format!(
                    "rule {} uses skip_state_update with visible state evidence",
                    rule.id
                ));
            }
        }
        validate_region_name(&rule.region)
            .map_err(|error| format!("rule {} uses invalid region: {error}", rule.id))?;
        if rule.region.trim().starts_with("top_non_empty_lines(")
            && manifest
                .min_engine_version
                .is_some_and(|version| version < TOP_NON_EMPTY_LINES_ENGINE_VERSION)
        {
            return Err(format!(
                "rule {} uses top_non_empty_lines but min_engine_version is below {}",
                rule.id, TOP_NON_EMPTY_LINES_ENGINE_VERSION
            ));
        }
        validate_gate(&gate_from_rule(rule), "rule", 0, &mut complexity)
            .map_err(|error| format!("rule {} has invalid matcher gates: {error}", rule.id))?;
    }
    Ok(())
}

#[derive(Default)]
struct ManifestComplexity {
    total_gates: usize,
    total_matchers: usize,
}

fn validate_gate(
    gate: &ManifestGate,
    context: &str,
    depth: usize,
    complexity: &mut ManifestComplexity,
) -> Result<(), String> {
    if depth > MAX_GATE_DEPTH {
        return Err(format!("{context} exceeds max gate depth {MAX_GATE_DEPTH}"));
    }
    complexity.total_gates += 1;
    if complexity.total_gates > MAX_TOTAL_GATES {
        return Err(format!("manifest exceeds max gate count {MAX_TOTAL_GATES}"));
    }
    validate_matcher_limits(gate, context, complexity)?;
    if !gate_has_positive_matcher(gate) {
        return Err(format!("{context} must contain a positive matcher"));
    }
    validate_regex_patterns(&gate.regex, context, "regex")?;
    validate_regex_patterns(&gate.line_regex, context, "line_regex")?;
    for nested in &gate.all {
        validate_gate(nested, "all gate", depth + 1, complexity)?;
    }
    for nested in &gate.any {
        validate_gate(nested, "any gate", depth + 1, complexity)?;
    }
    for nested in &gate.not_gate {
        if !gate_has_any_matcher(nested) {
            return Err(format!("{context} contains an empty not gate"));
        }
        validate_not_gate(nested, depth + 1, complexity)?;
    }
    Ok(())
}

fn validate_not_gate(
    gate: &ManifestGate,
    depth: usize,
    complexity: &mut ManifestComplexity,
) -> Result<(), String> {
    if depth > MAX_GATE_DEPTH {
        return Err(format!("not gate exceeds max gate depth {MAX_GATE_DEPTH}"));
    }
    complexity.total_gates += 1;
    if complexity.total_gates > MAX_TOTAL_GATES {
        return Err(format!("manifest exceeds max gate count {MAX_TOTAL_GATES}"));
    }
    validate_matcher_limits(gate, "not gate", complexity)?;
    if !gate_has_any_matcher(gate) {
        return Err("not gate must contain a matcher".to_string());
    }
    validate_regex_patterns(&gate.regex, "not gate", "regex")?;
    validate_regex_patterns(&gate.line_regex, "not gate", "line_regex")?;
    for nested in &gate.all {
        validate_gate(nested, "not all gate", depth + 1, complexity)?;
    }
    for nested in &gate.any {
        validate_gate(nested, "not any gate", depth + 1, complexity)?;
    }
    for nested in &gate.not_gate {
        validate_not_gate(nested, depth + 1, complexity)?;
    }
    Ok(())
}

fn validate_matcher_limits(
    gate: &ManifestGate,
    context: &str,
    complexity: &mut ManifestComplexity,
) -> Result<(), String> {
    let matcher_count = gate.contains.len() + gate.regex.len() + gate.line_regex.len();
    if matcher_count > MAX_MATCHERS_PER_GATE {
        return Err(format!(
            "{context} has {matcher_count} direct matchers, max is {MAX_MATCHERS_PER_GATE}"
        ));
    }
    complexity.total_matchers += matcher_count;
    if complexity.total_matchers > MAX_TOTAL_MATCHERS {
        return Err(format!(
            "manifest exceeds max matcher count {MAX_TOTAL_MATCHERS}"
        ));
    }
    for value in gate
        .contains
        .iter()
        .chain(gate.regex.iter())
        .chain(gate.line_regex.iter())
    {
        if value.chars().count() > MAX_MATCHER_CHARS {
            return Err(format!(
                "{context} matcher exceeds max length {MAX_MATCHER_CHARS}"
            ));
        }
    }
    Ok(())
}

fn validate_regex_patterns(patterns: &[String], context: &str, field: &str) -> Result<(), String> {
    for pattern in patterns {
        Regex::new(pattern).map_err(|error| {
            format!("{context} contains invalid {field} pattern {pattern:?}: {error}")
        })?;
    }
    Ok(())
}

fn gate_has_positive_matcher(gate: &ManifestGate) -> bool {
    !gate.contains.is_empty()
        || !gate.regex.is_empty()
        || !gate.line_regex.is_empty()
        || !gate.all.is_empty()
        || !gate.any.is_empty()
}

fn gate_has_any_matcher(gate: &ManifestGate) -> bool {
    gate_has_positive_matcher(gate) || !gate.not_gate.is_empty()
}

fn validate_region_name(spec: &str) -> Result<(), String> {
    let trimmed = spec.trim();
    match trimmed {
        "whole_recent"
        | "after_last_prompt_marker"
        | "before_current_prompt_marker"
        | "whole_recent_without_current_prompt_marker"
        | "current_prompt_block_marker"
        | "after_current_prompt_block_marker"
        | "prompt_box_body"
        | "above_prompt_box"
        | "last_non_empty_above_prompt_box"
        | "after_last_horizontal_rule"
        | "osc_title"
        | "osc_progress" => Ok(()),
        _ if region_count(trimmed, "bottom_lines").is_some()
            || region_count(trimmed, "bottom_non_empty_lines").is_some()
            || top_region_count(trimmed).is_some() =>
        {
            Ok(())
        }
        _ => Err(trimmed.to_string()),
    }
}

fn gate_from_rule(rule: &ManifestRule) -> ManifestGate {
    ManifestGate {
        all: rule.all.clone(),
        any: rule.any.clone(),
        not_gate: rule.not_gate.clone(),
        contains: rule.contains.clone(),
        regex: rule.regex.clone(),
        line_regex: rule.line_regex.clone(),
    }
}

fn compile_manifest(manifest: &AgentManifest) -> Result<LoadedManifest, String> {
    let rules = manifest
        .rules
        .iter()
        .map(|rule| {
            compile_gate(&gate_from_rule(rule))
                .map(|gate| (rule.clone(), gate))
                .map_err(|error| format!("rule {} could not be compiled: {error}", rule.id))
        })
        .collect::<Result<_, _>>()?;
    Ok(LoadedManifest { rules })
}

fn compile_gate(gate: &ManifestGate) -> Result<CompiledGate, String> {
    let compile_regexes = |patterns: &[String]| {
        patterns
            .iter()
            .map(|pattern| Regex::new(pattern).map_err(|error| error.to_string()))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(CompiledGate {
        all: gate
            .all
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        any: gate
            .any
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        not_gate: gate
            .not_gate
            .iter()
            .map(compile_gate)
            .collect::<Result<_, _>>()?,
        contains: gate
            .contains
            .iter()
            .map(|needle| needle.to_lowercase())
            .collect(),
        regex: compile_regexes(&gate.regex)?,
        line_regex: compile_regexes(&gate.line_regex)?,
    })
}

fn gate_matches(gate: &CompiledGate, text: &str) -> bool {
    let lower_text = text.to_lowercase();
    compiled_gate_matches(gate, text, &lower_text)
}

fn compiled_gate_matches(gate: &CompiledGate, text: &str, lower_text: &str) -> bool {
    gate.contains
        .iter()
        .all(|needle| lower_text.contains(needle))
        && gate.regex.iter().all(|regex| regex.is_match(text))
        && gate
            .line_regex
            .iter()
            .all(|regex| text.lines().any(|line| regex.is_match(line)))
        && gate
            .all
            .iter()
            .all(|nested| compiled_gate_matches(nested, text, lower_text))
        && (gate.any.is_empty()
            || gate
                .any
                .iter()
                .any(|nested| compiled_gate_matches(nested, text, lower_text)))
        && !gate
            .not_gate
            .iter()
            .any(|nested| compiled_gate_matches(nested, text, lower_text))
}

fn region<'a>(input: DetectionInput<'a>, spec: &str) -> &'a str {
    let trimmed = spec.trim();
    // OSC regions source from their dedicated fields, not the screen.
    match trimmed {
        "osc_title" => return input.osc_title,
        "osc_progress" => return input.osc_progress,
        _ => {}
    }
    let content = input.screen;
    match trimmed {
        "whole_recent" => content,
        "after_last_prompt_marker" => after_last_prompt_marker(content),
        "before_current_prompt_marker" => before_current_prompt_marker(content),
        "whole_recent_without_current_prompt_marker" => {
            whole_recent_without_current_prompt_marker(content)
        }
        "current_prompt_block_marker" => current_prompt_block_marker(content).unwrap_or(""),
        "after_current_prompt_block_marker" => {
            after_current_prompt_block_marker(content).unwrap_or("")
        }
        "prompt_box_body" => prompt_box_body(content).unwrap_or(""),
        "above_prompt_box" => above_prompt_box(content),
        "last_non_empty_above_prompt_box" => last_non_empty_line(above_prompt_box(content)),
        "after_last_horizontal_rule" => after_last_horizontal_rule(content),
        _ => {
            if let Some(count) = region_count(trimmed, "bottom_lines") {
                return bottom_lines(content, count);
            }
            if let Some(count) = region_count(trimmed, "bottom_non_empty_lines") {
                return bottom_non_empty_lines(content, count);
            }
            if let Some(count) = top_region_count(trimmed) {
                return top_non_empty_lines(content, count);
            }
            ""
        }
    }
}

fn region_count(spec: &str, name: &str) -> Option<usize> {
    spec.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('('))
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|count| count.parse::<usize>().ok())
}

fn top_region_count(spec: &str) -> Option<usize> {
    let count = spec
        .strip_prefix("top_non_empty_lines")?
        .strip_prefix('(')?
        .strip_suffix(')')?;
    if count.starts_with('0') || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    count
        .parse::<usize>()
        .ok()
        .filter(|count| *count <= MAX_TOP_REGION_LINE_COUNT)
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(count);
    slice_from_line_index(content, &lines, start)
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(start_index) = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    slice_from_line_index(content, &lines, start_index)
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(end_index) = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .take(count)
        .last()
        .map(|(index, _)| index)
    else {
        return "";
    };
    let byte_offset = line_start_offset(content, &lines, end_index + 1);
    &content[..byte_offset]
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(index) = lines.iter().rposition(|line| codex_prompt_line(line)) else {
        return content;
    };
    slice_from_line_index(content, &lines, index + 1)
}

fn before_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(index) = current_codex_prompt_index(&lines) else {
        return content;
    };
    let byte_offset = lines[..index]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    &content[..byte_offset.min(content.len())]
}

fn whole_recent_without_current_prompt_marker(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    if current_codex_prompt_index(&lines).is_some() {
        ""
    } else {
        content
    }
}

fn current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let prompt_index = current_codex_prompt_index(&lines)?;
    lines[..prompt_index]
        .iter()
        .rev()
        .find(|line| codex_block_marker_line(line))
        .copied()
}

fn after_current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let prompt_index = current_codex_prompt_index(&lines)?;
    let block_index = lines[..prompt_index]
        .iter()
        .rposition(|line| codex_block_marker_line(line))?;
    Some(slice_from_line_index(content, &lines, block_index))
}

fn current_codex_prompt_index(lines: &[&str]) -> Option<usize> {
    let prompt_index = lines.iter().rposition(|line| codex_prompt_line(line))?;
    if lines[prompt_index + 1..]
        .iter()
        .any(|line| codex_block_marker_line(line))
    {
        return None;
    }
    Some(prompt_index)
}

fn codex_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn codex_block_marker_line(line: &str) -> bool {
    line.starts_with('•') || line.starts_with('■') || line.starts_with('✗') || line.starts_with('✓')
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let top = prompt_box_top_border_index(&lines)?;
    let start = line_start_offset(content, &lines, top + 1);
    let end_index = lines[top + 1..]
        .iter()
        .position(|line| is_horizontal_rule(line))
        .map(|relative| top + 1 + relative)
        .unwrap_or(lines.len());
    let end = line_start_offset(content, &lines, end_index);
    Some(&content[start.min(content.len())..end.min(content.len())])
}

fn above_prompt_box(content: &str) -> &str {
    let lines: Vec<&str> = content.lines().collect();
    let Some(top) = prompt_box_top_border_index(&lines) else {
        return content;
    };
    let end = line_start_offset(content, &lines, top);
    &content[..end.min(content.len())]
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let mut last_rule_end = 0usize;
    let mut offset = 0usize;
    for line in content.lines() {
        let next_offset = offset + line.len() + 1;
        if is_horizontal_rule(line) {
            last_rule_end = next_offset.min(content.len());
        }
        offset = next_offset;
    }
    &content[last_rule_end..]
}

fn last_non_empty_line(content: &str) -> &str {
    content
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
}

fn prompt_box_top_border_index(lines: &[&str]) -> Option<usize> {
    let mut border_count = 0;
    for index in (0..lines.len()).rev() {
        if is_horizontal_rule(lines[index]) {
            border_count += 1;
            if border_count == 2 {
                return Some(index);
            }
        }
    }
    None
}

fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let rule_chars = trimmed.chars().take_while(|&ch| ch == '─').count();
    if rule_chars == 0 {
        return false;
    }
    let rule_bytes = trimmed
        .char_indices()
        .nth(rule_chars)
        .map(|(index, _)| index)
        .unwrap_or(trimmed.len());
    let suffix = trimmed[rule_bytes..].trim_start();
    suffix.is_empty() || rule_chars >= 3
}

fn slice_from_line_index<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    let byte_offset = line_start_offset(content, lines, index);
    &content[byte_offset.min(content.len())..]
}

fn line_start_offset(content: &str, lines: &[&str], index: usize) -> usize {
    lines[..index.min(lines.len())]
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>()
        .min(content.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_manifest_compiles_and_names_a_known_agent() {
        for (id, _) in BUNDLED_MANIFESTS {
            let agent = AgentKind::parse_label(id).unwrap_or_else(|| panic!("unknown id {id}"));
            assert!(
                AgentKind::SCREEN_MANIFEST_AGENTS.contains(&agent),
                "{id} is bundled but not a screen manifest agent"
            );
        }
        for agent in AgentKind::SCREEN_MANIFEST_AGENTS {
            assert!(
                loaded_manifest(agent).is_some(),
                "{} has no bundled manifest",
                agent.id()
            );
        }
    }

    fn screen(agent: AgentKind, screen: &str) -> AgentDetection {
        detect(
            agent,
            DetectionInput {
                screen,
                osc_title: "",
                osc_progress: "",
            },
        )
    }

    #[test]
    fn claude_states_follow_the_manifest() {
        let idle = "  Some earlier output\n────────────\n❯ \n────────────\n  ? for shortcuts";
        let detection = screen(AgentKind::Claude, idle);
        assert_eq!(detection.state, AgentState::Idle);
        assert!(detection.visible_idle);

        let working = "⏵ Thinking… (esc to interrupt · 3s)\n────────────\n❯ \n────────────";
        assert_eq!(
            screen(AgentKind::Claude, working).state,
            AgentState::Working
        );

        let blocked = "Do you want to proceed?\n❯ 1. Yes\n  2. No\n\nEsc to cancel";
        let detection = screen(AgentKind::Claude, blocked);
        assert_eq!(detection.state, AgentState::Blocked);
        assert!(detection.visible_blocker);

        let transcript = "Showing detailed transcript\nctrl+o to toggle\n? for shortcuts";
        assert!(screen(AgentKind::Claude, transcript).skip_state_update);
    }

    #[test]
    fn codex_osc_title_outranks_the_screen() {
        let screen_text = "• Working (2s · esc to interrupt)\n› ";
        assert_eq!(
            screen(AgentKind::Codex, screen_text).state,
            AgentState::Working
        );
        let with_title = detect(
            AgentKind::Codex,
            DetectionInput {
                screen: screen_text,
                osc_title: "Action Required",
                osc_progress: "",
            },
        );
        assert_eq!(with_title.state, AgentState::Blocked);
        let spinner = detect(
            AgentKind::Codex,
            DetectionInput {
                screen: "› ",
                osc_title: "⠼ condr",
                osc_progress: "",
            },
        );
        assert_eq!(spinner.state, AgentState::Working);
    }

    #[test]
    fn unmatched_known_agents_fall_back_to_idle() {
        let detection = screen(AgentKind::Gemini, "nothing recognizable here");
        assert_eq!(detection.state, AgentState::Idle);
        assert!(!detection.visible_idle);
    }

    #[test]
    fn regions_slice_the_screen_as_documented() {
        let content = "a\n\nb\nc\n";
        assert_eq!(bottom_non_empty_lines(content, 2), "b\nc\n");
        assert_eq!(bottom_lines(content, 1), "c\n");
        assert_eq!(top_non_empty_lines(content, 2), "a\n\nb\n");
        assert_eq!(after_last_horizontal_rule("x\n───\ny\n"), "y\n");
        assert_eq!(
            prompt_box_body("out\n────\n❯ hi\n────\n").unwrap(),
            "❯ hi\n"
        );
        assert_eq!(after_last_prompt_marker("• done\n› \nmore"), "more");
        assert_eq!(whole_recent_without_current_prompt_marker("• done\n› "), "");
        assert_eq!(
            whole_recent_without_current_prompt_marker("› old\n• block"),
            "› old\n• block"
        );
    }
}
