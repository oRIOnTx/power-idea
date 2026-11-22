//! Claude Code transcripts (`~/.claude/projects/<project>/<session>.jsonl`)
//! reduced to the facts the metrics need. Unknown lines and fields are
//! skipped, so format changes degrade gracefully instead of failing.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message the user typed (not tool results or IDE-injected text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub chars: usize,
    /// "no", "revert", "not like that"… — the agent had to be corrected.
    pub correction: bool,
    /// Mentions a file or path (`src/x.rs`, `app/models`).
    pub mentions_paths: bool,
    /// States what "done" means (tests pass, should, must, criteria…).
    pub has_criteria: bool,
}

/// One tool call and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    /// Shell command or a short fingerprint of the input (for loop detection).
    pub input: String,
    pub error: bool,
    /// The call followed a given prompt (index into `prompts`).
    pub after_prompt: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    /// Working folder of the session (the project).
    pub project: String,
    /// Unix seconds.
    pub started: i64,
    pub ended: i64,
    /// Model used for most replies.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Claude Code's own cost estimate (`cost-state`), USD.
    pub cost_usd: f64,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub prompts: Vec<Prompt>,
    pub tools: Vec<ToolCall>,
    pub interrupts: usize,
    pub compactions: usize,
}

impl Session {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }

    pub fn duration_secs(&self) -> i64 {
        (self.ended - self.started).max(0)
    }
}

/// `2026-11-22T10:30:05.123Z` → Unix seconds (UTC offsets other than Z are ignored).
pub fn parse_timestamp(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.trim_end_matches('Z').split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = t
        .next()
        .and_then(|s| s.split('.').next()?.parse::<i64>().ok())
        .unwrap_or(0);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + min * 60 + sec)
}

const CORRECTION_MARKERS: &[&str] = &[
    "не так",
    "ні,",
    "ні ",
    "відкоти",
    "поверни як було",
    "я ж казав",
    "я казав",
    "неправильно",
    "знову не",
    "no,",
    "no ",
    "not like that",
    "revert",
    "undo",
    "wrong",
    "that's not",
    "i said",
    "again,",
    "still not",
    "doesn't work",
    "не працює",
    "все ще",
];
const CRITERIA_MARKERS: &[&str] = &[
    "test",
    "тест",
    "should",
    "must",
    "expect",
    "done when",
    "критері",
    "має ",
    "повинн",
    "щоб ",
    "so that",
];

fn is_path_like(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '.' && c != '_');
    word.contains('/') && word.len() > 2 || {
        let ext = word.rsplit_once('.').map(|(_, e)| e);
        ext.is_some_and(|e| {
            matches!(
                e,
                "rs" | "rb"
                    | "py"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "go"
                    | "md"
                    | "toml"
                    | "yml"
                    | "json"
                    | "sql"
                    | "erb"
            )
        })
    }
}

pub fn classify_prompt(text: &str) -> Prompt {
    let lower = text.to_lowercase();
    let start: String = lower.chars().take(80).collect();
    Prompt {
        chars: text.chars().count(),
        correction: CORRECTION_MARKERS
            .iter()
            .any(|m| start.starts_with(m) || start.contains(&format!(" {m}"))),
        mentions_paths: text.split_whitespace().any(is_path_like),
        has_criteria: CRITERIA_MARKERS.iter().any(|m| lower.contains(m)),
    }
}

/// Text the user typed, or `None` for injected/system content.
fn prompt_text(line: &Value) -> Option<String> {
    if line.get("isMeta").and_then(Value::as_bool) == Some(true) || line.get("toolUseResult").is_some() {
        return None;
    }
    let content = line.pointer("/message/content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            if parts
                .iter()
                .any(|p| p.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return None;
            }
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let trimmed = text.trim_start();
    // Slash commands, command output and reminders are not prompts.
    (!trimmed.is_empty() && !trimmed.starts_with('<')).then_some(text)
}

fn tool_fingerprint(name: &str, input: &Value) -> String {
    if let Some(command) = input.get("command").and_then(Value::as_str) {
        return command.chars().take(200).collect();
    }
    let text = input.to_string();
    format!("{name}:{}", text.chars().take(200).collect::<String>())
}

/// The prompts the user typed, in order (interruption markers left out).
pub fn prompt_texts(lines: impl Iterator<Item = String>) -> Vec<String> {
    lines
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .filter(|value| value.get("type").and_then(Value::as_str) == Some("user"))
        .filter_map(|value| prompt_text(&value))
        .filter(|text| !text.contains("[Request interrupted by user"))
        .collect()
}

/// Parses one transcript; `None` when it has no user prompt (empty or tool-only sessions).
pub fn parse_session(lines: impl Iterator<Item = String>) -> Option<Session> {
    let mut session = Session::default();
    let mut usage: HashMap<String, [u64; 4]> = HashMap::new();
    let mut models: HashMap<String, usize> = HashMap::new();
    // tool_use id → index in `session.tools`
    let mut pending: HashMap<String, usize> = HashMap::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(ts) = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
        {
            if session.started == 0 || ts < session.started {
                session.started = ts;
            }
            session.ended = session.ended.max(ts);
        }
        if session.id.is_empty()
            && let Some(id) = value.get("sessionId").and_then(Value::as_str)
        {
            session.id = id.to_string();
        }
        if session.project.is_empty()
            && let Some(cwd) = value.get("cwd").and_then(Value::as_str)
        {
            session.project = cwd.to_string();
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(text) = prompt_text(&value) {
                    if text.contains("[Request interrupted by user") {
                        session.interrupts += 1;
                    } else {
                        session.prompts.push(classify_prompt(&text));
                    }
                }
                for part in value
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = part
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(&ix) = pending.get(id) {
                        session.tools[ix].error = part.get("is_error").and_then(Value::as_bool) == Some(true);
                    }
                }
            }
            Some("assistant") => {
                let message = value.get("message").cloned().unwrap_or(Value::Null);
                let id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if let Some(u) = message.get("usage") {
                    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    // A message is logged once per content block with the same usage: keep one copy.
                    usage.insert(
                        id,
                        [
                            n("input_tokens"),
                            n("output_tokens"),
                            n("cache_read_input_tokens"),
                            n("cache_creation_input_tokens"),
                        ],
                    );
                }
                if let Some(model) = message
                    .get("model")
                    .and_then(Value::as_str)
                    .filter(|m| !m.starts_with('<'))
                {
                    *models.entry(model.to_string()).or_default() += 1;
                }
                for part in message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let name = part
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let input = tool_fingerprint(&name, part.get("input").unwrap_or(&Value::Null));
                    let tool_id = part
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if pending.contains_key(&tool_id) {
                        continue;
                    }
                    pending.insert(tool_id, session.tools.len());
                    session.tools.push(ToolCall {
                        name,
                        input,
                        error: false,
                        after_prompt: session.prompts.len().saturating_sub(1),
                    });
                }
            }
            Some("system") if value.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                session.compactions += 1;
            }
            Some("cost-state") => {
                let f = |k: &str| value.get(k).and_then(Value::as_f64);
                session.cost_usd = f("totalCostUSD").unwrap_or(session.cost_usd);
                session.lines_added = f("totalLinesAdded").map_or(session.lines_added, |v| v as u64);
                session.lines_removed = f("totalLinesRemoved").map_or(session.lines_removed, |v| v as u64);
            }
            _ => {}
        }
    }
    for [input, output, read, write] in usage.into_values() {
        session.input_tokens += input;
        session.output_tokens += output;
        session.cache_read_tokens += read;
        session.cache_write_tokens += write;
    }
    session.model = models
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map_or_default(|(m, _)| m);
    (!session.prompts.is_empty() && !session.id.is_empty()).then_some(session)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{classify_prompt, parse_session, parse_timestamp};

    /// A small transcript: two prompts (the second a correction), three tool
    /// calls (one failing), a message logged twice, an interrupt and a cost line.
    pub(crate) const FIXTURE: &str = r#"{"type":"permission-mode","sessionId":"s1"}
{"type":"user","sessionId":"s1","cwd":"/p/shop","timestamp":"2026-11-20T10:00:00.000Z","message":{"role":"user","content":"Fix the login bug in app/models/user.rb, tests must pass"}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-20T10:00:10.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"thinking","thinking":"..."}],"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-20T10:00:11.000Z","message":{"id":"m1","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"bundle exec rspec"}}],"usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}}}
{"type":"user","sessionId":"s1","timestamp":"2026-11-20T10:00:20.000Z","toolUseResult":{},"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"1 failure"}]}}
{"type":"assistant","sessionId":"s1","timestamp":"2026-11-20T10:01:00.000Z","message":{"id":"m2","model":"claude-opus-5-5","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"app/models/user.rb"}},{"type":"tool_use","id":"t3","name":"Bash","input":{"command":"git commit -m fix"}}],"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":2000,"cache_creation_input_tokens":0}}}
{"type":"user","sessionId":"s1","timestamp":"2026-11-20T10:01:30.000Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}
{"type":"user","sessionId":"s1","timestamp":"2026-11-20T10:02:00.000Z","message":{"role":"user","content":"no, revert that commit"}}
{"type":"user","sessionId":"s1","timestamp":"2026-11-20T10:02:01.000Z","isMeta":true,"message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}
{"type":"system","subtype":"compact_boundary","sessionId":"s1"}
{"type":"cost-state","sessionId":"s1","totalCostUSD":0.42,"totalLinesAdded":12,"totalLinesRemoved":3}
not json at all
{"type":"some-future-event","sessionId":"s1","whatever":true}"#;

    pub(crate) fn fixture_lines() -> impl Iterator<Item = String> {
        FIXTURE.lines().map(str::to_string)
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2026-11-20T10:00:00.000Z"), Some(1_795_168_800));
        assert_eq!(parse_timestamp("nonsense"), None);
    }

    #[test]
    fn classifies_prompts() {
        let p = classify_prompt("Fix the login bug in app/models/user.rb, tests must pass");
        assert!(p.mentions_paths && p.has_criteria && !p.correction);
        assert!(classify_prompt("Ні, відкоти це").correction);
        assert!(classify_prompt("no, revert that").correction);
        assert!(!classify_prompt("Add a Docker panel").correction);
    }

    #[test]
    fn parses_a_transcript() {
        let s = parse_session(fixture_lines()).unwrap();
        assert_eq!(
            (s.id.as_str(), s.project.as_str(), s.model.as_str()),
            ("s1", "/p/shop", "claude-opus-5-5")
        );
        assert_eq!(s.prompts.len(), 2);
        assert!(s.prompts[1].correction);
        assert_eq!((s.interrupts, s.compactions), (1, 1));
        // m1 is logged twice but counted once.
        assert_eq!(
            (s.input_tokens, s.output_tokens, s.cache_read_tokens),
            (110, 70, 3000)
        );
        assert_eq!(s.tools.len(), 3);
        assert!(s.tools[0].error && !s.tools[1].error);
        assert_eq!(s.tools[2].input, "git commit -m fix");
        assert_eq!((s.cost_usd, s.lines_added), (0.42, 12));
        assert_eq!(s.duration_secs(), 121);
    }

    #[test]
    fn extracts_prompt_texts() {
        assert_eq!(
            super::prompt_texts(fixture_lines()),
            [
                "Fix the login bug in app/models/user.rb, tests must pass",
                "no, revert that commit"
            ]
        );
    }

    #[test]
    fn skips_sessions_without_prompts() {
        assert!(parse_session(["{\"type\":\"mode\"}".to_string()].into_iter()).is_none());
    }
}
