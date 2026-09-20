use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::message::{AgentMessage, Usage};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Header {
        version: u32,
        id: String,
        cwd: String,
        created: String, // RFC3339
    },
    Message {
        id: String,
        /// Reserved for future tree/fork sessions; the kernel is linear today
        /// and always records `None`.
        parent_id: Option<String>,
        message: AgentMessage,
    },
    ModelChange {
        id: String,
        model: String,
    },
    EffortChange {
        id: String,
        /// None = effort cleared (send nothing).
        effort: Option<String>,
    },
    /// A compaction boundary: `summary` stands in for every `Message` before
    /// `first_kept_message` (an ordinal into the `Message` sequence).
    Compaction {
        id: String,
        summary: String,
        first_kept_message: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    #[serde(other)]
    Unknown,
}

pub struct Session {
    path: Option<PathBuf>,
    entries: Vec<SessionEntry>,
}

impl Session {
    pub fn create(dir: &Path) -> io::Result<Session> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Session::create_with_cwd(dir, &cwd)
    }

    /// Like [`create`], but records `cwd` in the session header instead of the
    /// process working directory. An agent whose tools run against a custom
    /// `working_dir` (see `AgentConfig.working_dir`) should stamp that same
    /// directory here so a resumed session's header agrees with where the tools
    /// actually ran — otherwise the header's `cwd` describes a directory the
    /// tools never touched.
    pub fn create_with_cwd(dir: &Path, cwd: &Path) -> io::Result<Session> {
        let id = uuid::Uuid::new_v4();
        let name = format!(
            "{}_{}.jsonl",
            Utc::now().timestamp_millis(),
            &id.simple().to_string()[..8]
        );
        Self::open_new(dir, name, id, cwd)
    }

    /// Like [`create`], but the file is exactly `<dir>/<name>.jsonl` — the
    /// deterministic tree session groups need (`root.jsonl`, `members/<name>.jsonl`),
    /// where a `{millis}_{uuid8}` name would not map back to an address. Header
    /// content identical to [`create_with_cwd`]; only the file name is caller-
    /// supplied.
    pub fn create_named(dir: &Path, name: &str) -> io::Result<Session> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Session::create_named_with_cwd(dir, name, &cwd)
    }

    /// [`create_named`] with an explicit `cwd` (see [`create_with_cwd`]).
    pub fn create_named_with_cwd(dir: &Path, name: &str, cwd: &Path) -> io::Result<Session> {
        Self::open_new(dir, format!("{name}.jsonl"), uuid::Uuid::new_v4(), cwd)
    }

    /// The shared constructor: fresh dirs, a header stamped first, one file.
    fn open_new(dir: &Path, filename: String, id: uuid::Uuid, cwd: &Path) -> io::Result<Session> {
        fs::create_dir_all(dir)?;
        let path = dir.join(filename);
        let mut session = Session {
            path: Some(path.clone()),
            entries: Vec::new(),
        };
        let header = SessionEntry::Header {
            version: 1,
            id: id.to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
            created: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        };
        session.append(header)?;
        Ok(session)
    }

    pub fn open(path: &Path) -> io::Result<Session> {
        let raw = fs::read_to_string(path)?;
        let lines: Vec<&str> = raw.lines().collect();
        let last_non_empty = lines.iter().rposition(|l| !l.trim().is_empty());
        let mut entries = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionEntry>(line) {
                Ok(e) => entries.push(e),
                Err(e) => {
                    if Some(i) == last_non_empty {
                        break; // torn final write; rest is blank
                    }
                    return Err(io::Error::new(io::ErrorKind::InvalidData, e));
                }
            }
        }
        Ok(Session {
            path: Some(path.to_path_buf()),
            entries,
        })
    }

    pub fn in_memory() -> Session {
        Session {
            path: None,
            entries: Vec::new(),
        }
    }

    pub fn append(&mut self, e: SessionEntry) -> io::Result<()> {
        if let Some(path) = &self.path {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            let line = serde_json::to_string(&e)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            f.write_all(format!("{line}\n").as_bytes())?;
            f.flush()?;
        }
        self.entries.push(e);
        Ok(())
    }

    pub fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }

    pub fn messages(&self) -> Vec<AgentMessage> {
        // The last compaction governs: everything before its kept boundary is
        // represented by the summary; everything after is kept verbatim.
        let boundary = self.entries.iter().rev().find_map(|e| match e {
            SessionEntry::Compaction {
                summary,
                first_kept_message,
                ..
            } => Some((summary.clone(), *first_kept_message)),
            _ => None,
        });
        let (summary, skip) = match boundary {
            Some((s, k)) => (Some(s), k),
            None => (None, 0),
        };

        let mut out = Vec::new();
        if let Some(s) = summary {
            out.push(AgentMessage::summary(s));
        }
        let mut ordinal = 0usize;
        for entry in &self.entries {
            if let SessionEntry::Message { message, .. } = entry {
                if ordinal >= skip {
                    out.push(message.clone());
                }
                ordinal += 1;
            }
        }
        out
    }

    /// Number of persisted `Message` entries — the ordinal space that
    /// [`SessionEntry::Compaction::first_kept_message`] indexes into.
    pub fn message_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, SessionEntry::Message { .. }))
            .count()
    }

    /// Record a compaction: `summary` stands in for every `Message` before the
    /// most recent `kept_messages`, which stay. The boundary ordinal is derived
    /// from the current message count, so it stays correct as messages append.
    pub fn record_compaction(
        &mut self,
        summary: &str,
        kept_messages: usize,
        tokens_before: Option<u64>,
        usage: Option<Usage>,
    ) -> io::Result<()> {
        let first_kept_message = self.message_count().saturating_sub(kept_messages);
        self.append(SessionEntry::Compaction {
            id: uuid::Uuid::new_v4().to_string(),
            summary: summary.to_string(),
            first_kept_message,
            tokens_before,
            usage,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn model(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|e| match e {
            SessionEntry::ModelChange { model, .. } => Some(model.clone()),
            _ => None,
        })
    }

    pub fn effort(&self) -> Option<Option<String>> {
        self.entries.iter().rev().find_map(|e| match e {
            SessionEntry::EffortChange { effort, .. } => Some(effort.clone()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{ContentBlock, StopReason};
    use serde_json::json;

    fn msg(text: &str) -> SessionEntry {
        SessionEntry::Message {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            message: AgentMessage::user_text(text),
        }
    }

    #[test]
    fn create_writes_header_with_timestamped_name() {
        let dir = tempfile::tempdir().unwrap();
        let s = Session::create(dir.path()).unwrap();
        let name = s.path().unwrap().file_name().unwrap().to_str().unwrap();
        let (ts, rest) = name.split_once('_').unwrap();
        assert!(ts.parse::<i64>().is_ok());
        let (hex, ext) = rest.split_once('.').unwrap();
        assert_eq!(ext, "jsonl");
        assert_eq!(hex.len(), 8);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(s.entries().len(), 1);
        match &s.entries()[0] {
            SessionEntry::Header {
                version, id, cwd, ..
            } => {
                assert_eq!(*version, 1);
                assert!(!id.is_empty());
                assert!(!cwd.is_empty());
            }
            _ => panic!("first entry not header"),
        }
    }

    #[test]
    fn create_with_cwd_stamps_header_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = std::path::Path::new("/custom/workdir");
        let s = Session::create_with_cwd(dir.path(), cwd).unwrap();
        match &s.entries()[0] {
            SessionEntry::Header { cwd, .. } => assert_eq!(cwd, "/custom/workdir"),
            _ => panic!("first entry not header"),
        }
    }

    #[test]
    fn create_named_writes_to_the_named_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = Session::create_named_with_cwd(dir.path(), "root", std::path::Path::new("/wd"))
            .unwrap();
        assert_eq!(root.path().unwrap().file_name().unwrap(), "root.jsonl");
        match &root.entries()[0] {
            SessionEntry::Header { version, cwd, .. } => {
                assert_eq!(*version, 1);
                assert_eq!(cwd, "/wd");
            }
            _ => panic!("first entry not header"),
        }

        // The bare `create_named` stamps the process cwd, and the file name is
        // exactly `<name>.jsonl` — the deterministic tree session groups need.
        let member = Session::create_named(dir.path(), "w1").unwrap();
        assert_eq!(member.path().unwrap().file_name().unwrap(), "w1.jsonl");

        // Reopen round-trips: an append comes back, so group transcripts are
        // ordinary kernel sessions with caller-supplied names.
        let mut reopened = Session::open(&dir.path().join("root.jsonl")).unwrap();
        reopened
            .append(SessionEntry::ModelChange {
                id: "m1".into(),
                model: "m".into(),
            })
            .unwrap();
        assert_eq!(reopened.entries().len(), 2);
        assert_eq!(
            Session::open(&dir.path().join("root.jsonl"))
                .unwrap()
                .model()
                .as_deref(),
            Some("m")
        );
    }

    #[test]
    fn create_append_open_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::create(dir.path()).unwrap();
        s.append(msg("hello")).unwrap();
        s.append(SessionEntry::ModelChange {
            id: "m1".into(),
            model: "claude-x".into(),
        })
        .unwrap();
        s.append(SessionEntry::Message {
            id: "a2".into(),
            parent_id: Some("a1".into()),
            message: AgentMessage::Assistant {
                content: vec![ContentBlock::Text {
                    text: "reply".into(),
                }],
                stop_reason: StopReason::Stop,
                usage: None,
                model: Some("claude-x".into()),
            },
        })
        .unwrap();

        let path = s.path().unwrap().to_path_buf();
        let reopened = Session::open(&path).unwrap();
        assert_eq!(reopened.entries(), s.entries());
        assert_eq!(reopened.messages().len(), 2);
        assert_eq!(reopened.model().as_deref(), Some("claude-x"));
    }

    #[test]
    fn torn_final_line_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(
            &path,
            format!(
                "{}\n{}",
                json!({"type":"message","id":"1","parent_id":null,"message":{"role":"user","content":[{"type":"text","text":"q"}]}}),
                "{\"type\":\"mess" // torn
            ),
        )
        .unwrap();
        let s = Session::open(&path).unwrap();
        assert_eq!(s.entries().len(), 1);
        assert_eq!(s.messages().len(), 1);
    }

    #[test]
    fn torn_line_followed_by_blank_lines_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(
            &path,
            format!(
                "{}\n{{\"type\":\"mess\n\n",
                json!({"type":"message","id":"1","parent_id":null,"message":{"role":"user","content":[{"type":"text","text":"q"}]}})
            ),
        )
        .unwrap();
        let s = Session::open(&path).unwrap();
        assert_eq!(s.entries().len(), 1);
        assert_eq!(s.messages().len(), 1);
    }

    #[test]
    fn torn_earlier_line_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(
            &path,
            "{\"type\":\"mess\n{\"type\":\"model_change\",\"id\":\"m\",\"model\":\"x\"}\n",
        )
        .unwrap();
        assert!(Session::open(&path).is_err());
    }

    #[test]
    fn unknown_entries_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                json!({"type":"header","version":1,"id":"x","cwd":"/","created":"2026-01-01T00:00:00Z"}),
                json!({"type":"future_thing","data":42}),
                json!({"type":"message","id":"1","parent_id":null,"message":{"role":"user","content":[{"type":"text","text":"q"}]}})
            ),
        )
        .unwrap();
        let s = Session::open(&path).unwrap();
        assert_eq!(s.entries().len(), 3);
        assert_eq!(s.entries()[1], SessionEntry::Unknown);
        assert_eq!(s.messages().len(), 1);
    }

    #[test]
    fn model_latest_wins() {
        let mut s = Session::in_memory();
        assert_eq!(s.model(), None);
        s.append(SessionEntry::ModelChange {
            id: "1".into(),
            model: "old".into(),
        })
        .unwrap();
        s.append(SessionEntry::ModelChange {
            id: "2".into(),
            model: "new".into(),
        })
        .unwrap();
        assert_eq!(s.model().as_deref(), Some("new"));
    }

    #[test]
    fn effort_latest_wins_and_none_means_cleared() {
        let mut s = Session::in_memory();
        assert_eq!(s.effort(), None);
        s.append(SessionEntry::EffortChange {
            id: "1".into(),
            effort: Some("high".into()),
        })
        .unwrap();
        assert_eq!(s.effort(), Some(Some("high".into())));
        s.append(SessionEntry::EffortChange {
            id: "2".into(),
            effort: None,
        })
        .unwrap();
        assert_eq!(s.effort(), Some(None));
    }

    #[test]
    fn effort_roundtrips_through_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::create(dir.path()).unwrap();
        s.append(SessionEntry::EffortChange {
            id: "e1".into(),
            effort: Some("low".into()),
        })
        .unwrap();
        let path = s.path().unwrap().to_path_buf();
        let reopened = Session::open(&path).unwrap();
        assert_eq!(reopened.effort(), Some(Some("low".into())));
    }

    #[test]
    fn in_memory_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::in_memory();
        s.append(msg("hi")).unwrap();
        assert!(s.path().is_none());
        assert_eq!(s.entries().len(), 1);
        assert_eq!(s.messages().len(), 1);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;

    fn msg(text: &str) -> SessionEntry {
        SessionEntry::Message {
            id: uuid::Uuid::new_v4().to_string(),
            parent_id: None,
            message: AgentMessage::user_text(text),
        }
    }

    fn texts(s: &Session) -> Vec<String> {
        s.messages().iter().map(|m| m.as_text()).collect()
    }

    #[test]
    fn no_compaction_returns_every_message() {
        let mut s = Session::in_memory();
        s.append(msg("a")).unwrap();
        s.append(msg("b")).unwrap();
        assert_eq!(texts(&s), ["a", "b"]);
        assert_eq!(s.message_count(), 2);
    }

    #[test]
    fn compaction_rebuilds_summary_plus_kept_tail_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::create(dir.path()).unwrap();
        for t in ["m1", "m2", "m3", "m4", "m5"] {
            s.append(msg(t)).unwrap();
        }
        // Keep the newest two (m4, m5); summarize the rest.
        s.record_compaction("did 1..3", 2, Some(123), None).unwrap();

        let path = s.path().unwrap().to_path_buf();
        let mut reopened = Session::open(&path).unwrap();
        let got = texts(&reopened);
        assert_eq!(got.len(), 3, "summary + two kept");
        assert!(got[0].contains("did 1..3"), "summary leads: {got:?}");
        assert_eq!(got[1], "m4");
        assert_eq!(got[2], "m5");

        // Messages appended after the compaction are kept too.
        reopened.append(msg("m6")).unwrap();
        assert_eq!(texts(&reopened), [got[0].clone(), "m4".into(), "m5".into(), "m6".into()]);
        assert_eq!(reopened.message_count(), 6);
    }

    #[test]
    fn second_compaction_supersedes_the_first() {
        let mut s = Session::in_memory();
        for t in ["m1", "m2", "m3", "m4", "m5", "m6"] {
            s.append(msg(t)).unwrap();
        }
        s.record_compaction("first", 4, None, None).unwrap(); // keep m3..m6
        assert_eq!(
            texts(&s),
            [
                "Summary of the earlier conversation (auto-generated by compaction):\n\nfirst",
                "m3",
                "m4",
                "m5",
                "m6"
            ]
        );
        s.record_compaction("second", 2, None, None).unwrap(); // keep m5, m6
        let got = texts(&s);
        assert_eq!(got.len(), 3);
        assert!(got[0].contains("second"), "the latest summary wins: {got:?}");
        assert_eq!(got[1], "m5");
        assert_eq!(got[2], "m6");
    }
}
