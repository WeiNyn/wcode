//! Session groups — one directory per ROOT session, holding the root's
//! transcript plus one transcript per worker, so a team survives a crash and
//! `--resume` rebuilds the whole team (Item 16; design: `docs/session-groups.md`,
//! decisions D1–D5).
//!
//! Layout under today's `session_dir()` (`crates/wcode-cli/src/repl.rs`,
//! `session_dir`):
//!
//! ```text
//! <base>/
//!   <millis>_<id8>/              # one group dir per ROOT session
//!     root.jsonl                 # the root's transcript (today's flat file, moved in)
//!     manifest.json              # team topology — fast path only (D5)
//!     members/
//!       w1.jsonl                 # one transcript per worker (SessionEntry, unchanged)
//!   <millis>_<id8>.jsonl …       # legacy flat files: listed, treated as bare roots
//! ```
//!
//! Write path: the root session is created inside a fresh group dir
//! ([`create_root_session`]); `SessionFactory::spawn` persists each worker into
//! `members/<name>.jsonl` (the file stem is the member's address stem — the
//! mapping D5's scan relies on).
//!
//! Resume path: [`open_group`] + [`rebuild_team`] — scan `members/` (D5), open
//! each transcript (torn-tail tolerant `Session::open`), seed the worker with
//! its full history (D1), rebuild the worker exactly like a live spawn, and
//! re-establish registry `register`/`set_owner`/`set_model` + the phonebook
//! name edges. A corrupt/missing member is reported, never fatal (§4.3).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wcode_harness::message::AgentMessage;
use wcode_harness::protocol::SessionId;
use wcode_harness::session::Session;

use crate::agents::{Orchestrator, WorkerSpec};

/// A session group: one directory per root session.
///
/// `dir` is `<session_dir>/<millis>_<id8>` — the root transcript's *old* flat
/// name minus `.jsonl`, so a chronological listing stays lexicographic
/// (identical to `list_sessions` today, `crates/wcode-cli/src/repl.rs`).
#[derive(Clone, Debug)]
pub struct SessionGroup {
    /// `<base>/<millis>_<id8>` — the group directory.
    pub dir: PathBuf,
    /// `<dir>/root.jsonl` — the root session's transcript.
    pub root_path: PathBuf,
    /// `<dir>/members/` — one `<name>.jsonl` per worker.
    pub members_dir: PathBuf,
}

impl SessionGroup {
    /// The group id = the dir's file name (the picker's resume key, handed
    /// back verbatim to `--resume`).
    pub fn id(&self) -> String {
        self.dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The root transcript path.
    pub fn root(&self) -> &Path {
        &self.root_path
    }

    /// The members directory.
    pub fn members(&self) -> &Path {
        &self.members_dir
    }

    /// `<dir>/manifest.json` — team topology fast path (D5).
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }

    /// `<members>/<name>.jsonl` — the transcript for member `name`.
    pub fn member_path(&self, name: &str) -> PathBuf {
        self.members_dir.join(format!("{name}.jsonl"))
    }

    /// Reassemble the group from its `members/` dir alone — the write path
    /// (`SessionFactory::spawn`) only knows the members dir it was handed, not
    /// the containing group dir.
    pub fn from_members_dir(members_dir: &Path) -> SessionGroup {
        let dir = members_dir.parent().unwrap_or(members_dir).to_path_buf();
        SessionGroup {
            root_path: dir.join("root.jsonl"),
            members_dir: members_dir.to_path_buf(),
            dir,
        }
    }
}

/// One resumable entry in the session dir: a session **group** (root + members)
/// or a **legacy** flat `.jsonl` file (a pre-group bare root). Member files
/// under `members/` are never listed.
#[derive(Clone, Debug)]
pub enum GroupEntry {
    /// A `<millis>_<id8>` directory.
    Group(SessionGroup),
    /// A `<millis>_<id8>.jsonl` file — resumed as a bare root (no team).
    Legacy { path: PathBuf },
}

impl GroupEntry {
    /// The path the `/resume` picker hands back verbatim and `--resume` consumes
    /// (a group dir or a flat file).
    pub fn path(&self) -> &Path {
        match self {
            GroupEntry::Group(g) => &g.dir,
            GroupEntry::Legacy { path } => path,
        }
    }

    /// The id shown in the picker label (`<millis>_<id8>`, no `.jsonl`).
    pub fn id(&self) -> &str {
        let name = self
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        name.strip_suffix(".jsonl").unwrap_or(name)
    }
}

// ---------------------------------------------------------------------------
// Write path — layout + named session files
// ---------------------------------------------------------------------------

/// A fresh `<millis>_<8hex>` group id — the same *shape* `Session::create_with_cwd`
/// gives a root file (its name minus `.jsonl`), so listing stays chronological/
/// lexicographic. No new dependency: millis from the clock, the eight hex chars
/// from the sub-second nanoseconds.
fn fresh_group_name() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}_{:08x}", now.as_millis(), now.subsec_nanos())
}

/// Create a new group under `base` (today's `session_dir()`): a fresh
/// `<base>/<millis>_<id8>/` directory plus an empty `members/`. The
/// `<millis>_<id8>` is generated in the shape `Session::create_with_cwd`'s file
/// name gives a root (minus `.jsonl`), so listing stays
/// chronological/lexicographic. `root.jsonl` is written by the caller via
/// [`create_root_session`].
pub fn create_group(base: &Path) -> io::Result<SessionGroup> {
    fs::create_dir_all(base)?;
    let dir = base.join(fresh_group_name());
    fs::create_dir(&dir)?;
    let members_dir = dir.join("members");
    fs::create_dir(&members_dir)?;
    Ok(SessionGroup {
        root_path: dir.join("root.jsonl"),
        members_dir,
        dir,
    })
}

/// The root transcript as a live `Session` persisted at `<dir>/root.jsonl`.
/// Replaces today's `Session::create(&session_dir())` (the session-create arm in
/// `crates/wcode-cli/src/main.rs`) once groups land. `cwd` is stamped in the
/// header exactly like `Session::create_with_cwd`.
pub fn create_root_session(dir: &Path, cwd: &Path) -> io::Result<Session> {
    Session::create_named_with_cwd(dir, "root", cwd)
}

/// A worker transcript persisted at `<members_dir>/<name>.jsonl`. The file stem
/// is the worker's address stem (D5's scan maps it back via [`member_id`]), so
/// validate `name` first ([`validate_member_name`]).
pub fn create_member_session(members_dir: &Path, name: &str) -> io::Result<Session> {
    validate_member_name(name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    Session::create_named(members_dir, name)
}

/// A member transcript for a group — the [`create_member_session`] convenience
/// over [`SessionGroup::member_path`].
pub fn create_group_member_session(group: &SessionGroup, name: &str) -> io::Result<Session> {
    create_member_session(group.members(), name)
}

/// Reject a member name that could not round-trip through a file name: empty,
/// contains a path separator or `..`, or ends in `.jsonl` (would make the
/// stem→address mapping ambiguous). The same gate guards spawn-time writes and
/// resume-time reads.
pub fn validate_member_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("member name is empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "member name `{name}` contains a path separator or `..`"
        ));
    }
    if name.ends_with(".jsonl") {
        return Err(format!("member name `{name}` must not end in `.jsonl`"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Discovery — listing + opening
// ---------------------------------------------------------------------------

/// List resumable entries in `dir`, **newest first**: group dirs (names shaped
/// `<millis>_<8hex>`) interleaved with legacy `.jsonl` files, both in the same
/// lexicographic-descending order `list_sessions` uses (`crates/wcode-cli/src/
/// repl.rs`), so group dirs and their old flat files sort as one sequence.
/// Member files under `members/` are never listed (they are inside a group
/// dir, not top-level). A missing `dir` yields an empty list, exactly like
/// `list_sessions`.
pub fn list_groups(dir: &Path) -> io::Result<Vec<GroupEntry>> {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        // No dir yet = no sessions; callers report it as empty.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut groups: Vec<SessionGroup> = Vec::new();
    let mut legacy: Vec<PathBuf> = Vec::new();
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if is_group_dir_name(name) {
                groups.push(SessionGroup {
                    root_path: path.join("root.jsonl"),
                    members_dir: path.join("members"),
                    dir: path,
                });
            }
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            legacy.push(path);
        }
    }
    // Q(C): a group dir and its old flat `<id>.jsonl` are the *same* root — the
    // dir is the newer, complete form, so hide the colliding legacy file.
    let group_ids: std::collections::HashSet<String> = groups
        .iter()
        .filter_map(|g| g.dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    legacy.retain(|p| {
        p.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .is_none_or(|stem| !group_ids.contains(&stem))
    });

    let mut entries: Vec<GroupEntry> = groups
        .into_iter()
        .map(GroupEntry::Group)
        .chain(legacy.into_iter().map(|path| GroupEntry::Legacy { path }))
        .collect();
    // Q(D): name-lexicographic descending, exactly like `list_sessions` — the
    // `{millis}_` prefix makes name order chronological for real session names.
    entries.sort_by(|a, b| b.id().cmp(a.id()));
    Ok(entries)
}

/// Whether `name` is a group-dir name: `<millis>_<8 hex chars>` — the shape
/// `Session::create_with_cwd` gives a root file minus `.jsonl`. Pure + unit-
/// tested so the listing rule stays explicit.
pub fn is_group_dir_name(name: &str) -> bool {
    let Some((millis, id8)) = name.split_once('_') else {
        return false;
    };
    millis.parse::<i64>().is_ok() && id8.len() == 8 && id8.chars().all(|c| c.is_ascii_hexdigit())
}

/// Open a group by its directory: `dir` must exist and hold a `root.jsonl`;
/// a missing `members/` is tolerated (an empty team — a bare root).
pub fn open_group(dir: &Path) -> io::Result<SessionGroup> {
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("not a session directory: {}", dir.display()),
        ));
    }
    let root_path = dir.join("root.jsonl");
    if !root_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no root.jsonl in {}", dir.display()),
        ));
    }
    Ok(SessionGroup {
        dir: dir.to_path_buf(),
        root_path,
        members_dir: dir.join("members"),
    })
}

/// Map a resume target up to its group dir (amendment 6b): a group dir itself,
/// or a `<dir>/root.jsonl` path — what `/reload` re-execs after a group resume
/// (`session_path()` is the root transcript). `None` when `p` is a bare flat
/// file or an unrelated directory, so the caller resumes it as today.
pub fn group_dir_of(p: &Path) -> Option<PathBuf> {
    let name = p.file_name()?.to_str()?;
    if is_group_dir_name(name) && p.is_dir() {
        return Some(p.to_path_buf());
    }
    if name == "root.jsonl" {
        let dir = p.parent()?;
        if dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_group_dir_name)
            && dir.is_dir()
        {
            return Some(dir.to_path_buf());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Manifest (D5) — fast path, never the source of truth
// ---------------------------------------------------------------------------

/// `manifest.json` — team topology, written as a fast path only (D5): the
/// `members/` scan is the source of truth for *who* exists; the manifest is the
/// source for each member's launch-time [`WorkerSpec`] (which the transcript
/// does not carry).
///
/// JSON shape (1:1 with `WorkerSpec`; its members are the serializable mirror
/// `MemberRecord` — see the review sheet Q(a) for the exact-schema decision):
///
/// ```json
/// {
///   "group": "1768570000000_a1b2c3d4",
///   "members": [
///     { "name": "w1", "model": null, "role": null, "tools": null,
///       "base_url": "http://…", "api_key": "…" }
///   ]
/// }
/// ```
///
/// All keys optional except `group`; unknown keys are ignored so a future
/// schema stays readable. (The root's cwd lives only in `root.jsonl`'s header —
/// one source of truth; amendment 6c.)
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Manifest {
    /// The group id (`<millis>_<id8>`), **informational only** — written on the
    /// first record and never validated on open. The `members/` scan and the
    /// directory name are the source of truth; `.group` is not read back.
    pub group: String,
    /// Per-member launch specs. `members/` scanning decides membership; a
    /// member with no record here resumes with a default (template) spec.
    #[serde(default)]
    pub members: Vec<MemberRecord>,
}

impl Manifest {
    /// An empty manifest for a fresh group.
    pub fn new(group: String) -> Self {
        Manifest {
            group,
            members: Vec::new(),
        }
    }
}

/// The serializable mirror of [`WorkerSpec`] — same six optional fields, no
/// dependency on `WorkerSpec`'s own derives. [`Self::from_worker_spec`] /
/// [`Self::to_worker_spec`] are the write/read seam.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemberRecord {
    /// The worker's address stem (`SessionId::agent(name)`). Must equal the
    /// `<members>/<name>.jsonl` file stem the record describes.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `WorkerSpec::system` — the `# Role` text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

impl MemberRecord {
    /// The exact `WorkerSpec` that launched this member (resume record).
    pub fn from_worker_spec(spec: &WorkerSpec) -> Option<Self> {
        Some(MemberRecord {
            name: spec.name.clone()?,
            model: spec.model.clone(),
            role: spec.system.clone(),
            tools: spec.tools.clone(),
            base_url: spec.base_url.clone(),
            api_key: spec.api_key.clone(),
        })
    }

    /// Back to a spec, with `name` pinned to this record's name. The caller
    /// overrides `name` with the file stem if the two disagree (manifest stale).
    pub fn to_worker_spec(&self) -> WorkerSpec {
        WorkerSpec {
            name: Some(self.name.clone()),
            model: self.model.clone(),
            system: self.role.clone(),
            tools: self.tools.clone(),
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
        }
    }
}

/// Read the manifest; `None` when absent or unparseable (scan path — D5).
pub fn load_manifest(group: &SessionGroup) -> Option<Manifest> {
    let raw = fs::read_to_string(group.manifest_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Write the manifest (create-on-missing). Called on the write path only —
/// resume never rewrites it.
pub fn write_manifest(group: &SessionGroup, manifest: &Manifest) -> io::Result<()> {
    fs::create_dir_all(&group.dir)?;
    let file = fs::File::create(group.manifest_path())?;
    serde_json::to_writer_pretty(file, manifest).map_err(io::Error::from)
}

/// The launch spec for member `name`: the manifest record (`fast path`),
/// else [`WorkerSpec::default()`] (inherit the *current* template — see the
/// review sheet Q(b)). `name` is always pinned onto the returned spec.
pub fn member_spec(group: &SessionGroup, name: &str) -> WorkerSpec {
    match load_manifest(group).and_then(|m| m.members.into_iter().find(|r| r.name == name)) {
        Some(record) => {
            let mut spec = record.to_worker_spec();
            spec.name = Some(name.to_string());
            spec
        }
        None => {
            // A crash can leave a member file without a manifest entry; the
            // member still resumes on the current template. Make that visible,
            // since the inherited model may differ from the one it ran on.
            eprintln!("member {name}: no manifest record, inheriting template");
            WorkerSpec {
                name: Some(name.to_string()),
                ..Default::default()
            }
        }
    }
}

/// Upsert `spec` into the manifest (replace-by-name) and write it. Called by
/// `SessionFactory::spawn`'s members-dir path so the launch record exists before
/// any crash could separate file and manifest (D5 re-syncs by scan either way).
pub fn record_member(group: &SessionGroup, spec: &WorkerSpec) -> io::Result<()> {
    let mut manifest = load_manifest(group).unwrap_or_else(|| Manifest::new(group.id()));
    if let Some(record) = MemberRecord::from_worker_spec(spec) {
        match manifest.members.iter_mut().find(|r| r.name == record.name) {
            Some(existing) => *existing = record,
            None => manifest.members.push(record),
        }
    }
    write_manifest(group, &manifest)
}

// ---------------------------------------------------------------------------
// Members — the scan (source of truth, D5)
// ---------------------------------------------------------------------------

/// Every `<name>.jsonl` under `members/`, lexicographically sorted — the source
/// of truth for the member set. Non-`.jsonl` files and sub-directories are
/// skipped. A missing `members/` yields an empty list.
pub fn scan_members(group: &SessionGroup) -> io::Result<Vec<PathBuf>> {
    let read = match fs::read_dir(&group.members_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    Ok(files)
}

/// A member transcript's address: `<name>.jsonl` → `SessionId::agent(<name>)`.
pub fn member_id(path: &Path) -> SessionId {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    SessionId::agent(stem)
}

// ---------------------------------------------------------------------------
// Resume — the whole team comes back (§4, D1, D5)
// ---------------------------------------------------------------------------

/// Open the root transcript of a group: the `Session` plus its full message
/// history — the piece the `--resume` arm already does today
/// (`crates/wcode-cli/src/main.rs`, the `Some(path)` arm) for a *file*; a
/// group routes through this instead. Hard failure (missing/corrupt root) is
/// the caller's fatal, exactly as today.
pub fn load_root(group: &SessionGroup) -> io::Result<(Session, Vec<AgentMessage>)> {
    let session = Session::open(&group.root_path)?;
    let messages = session.messages();
    Ok((session, messages))
}

/// How one member came back on resume.
#[derive(Debug)]
pub enum MemberResume {
    /// The member is live again: opened, seeded (D1), rebuilt, registered,
    /// owned, named.
    ///
    /// A `messages: 0` result is the **phantom-member window**: the member file
    /// is created (`create_group_member_session`, which writes only the header)
    /// *before* the manifest is written (`record_member`), so a crash in
    /// between leaves a header-only file with no manifest entry. On resume that
    /// opens with an empty context and comes back as a benign "hello" member —
    /// distinct from a *corrupt* file, which is `Skipped { reason }`.
    Restored {
        /// The live address (`agent:<name>`), as registered.
        id: SessionId,
        /// Messages seeded into the rebuilt worker (`s.messages().len()`).
        messages: usize,
    },
    /// The transcript could not be parsed (corrupt/missing) — the rest of the
    /// team resumes regardless (§4.3). Reported to the caller, never fatal.
    Skipped {
        /// The member's short name.
        name: String,
        /// Why it did not join (an `open`/`resume_worker` error string).
        reason: String,
    },
}

/// Rebuild every local member of `group` into a live team under `orchestrator`.
///
/// Per `members/<name>.jsonl` (scan — D5):
/// 1. `Session::open` (torn-tail tolerant) → `s.messages()` = the worker's
///    **full transcript** (D1; compaction inside the transcript still applies,
///    same knob as the root).
/// 2. Recover the launch spec ([`member_spec`]): the manifest record, else a
///    default that inherits the current `orchestrator` template.
/// 3. Build the worker via [`Orchestrator::resume_worker`] — which uses the
///    orchestrator's *own* factory/phonebook (amendment 6a), so a resumed member
///    is indistinguishable from a spawned one: same name allocation,
///    `registry.register` + `set_owner` + `set_model`, surface-sink, and phonebook
///    name edge.
///
/// A member whose file parses joins `Restored`; a corrupt/missing one is
/// `Skipped` (with the reason) and the loop continues. There is no hard
/// failure here — the group's root was already opened by [`load_root`].
///
/// A header-only member file (empty transcript) still parses, so it comes back
/// as `Restored { messages: 0 }` — the crash window between member-file
/// creation and the manifest write (see [`MemberResume::Restored`]), *not* a
/// corrupt file.
///
/// `root` is the report-back target (the root's address, i.e. `orchestrator.id()`).
pub fn rebuild_team(
    group: &SessionGroup,
    orchestrator: &Orchestrator,
    root: &SessionId,
) -> Vec<MemberResume> {
    let files = match scan_members(group) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("scan members of {}: {e}", group.dir.display());
            return Vec::new();
        }
    };
    let mut resumed = Vec::new();
    for file in files {
        // The file stem *is* the member's address (D5): `members/w1.jsonl` →
        // `agent:w1`, and back via [`SessionGroup::member_path`]. A stem that
        // cannot round-trip is skipped, never fatal.
        let id = member_id(&file);
        let name = match id.as_str().strip_prefix("agent:") {
            Some(n) => n.to_string(),
            None => {
                eprintln!("skip member: unnamed file {}", file.display());
                continue;
            }
        };
        if let Err(reason) = validate_member_name(&name) {
            resumed.push(MemberResume::Skipped { name, reason });
            continue;
        }
        match Session::open(&group.member_path(&name)) {
            Ok(session) => {
                let context = session.messages();
                let spec = member_spec(group, &name);
                match orchestrator.resume_worker(root, spec, Some(session), context.clone()) {
                    Ok(worker) => resumed.push(MemberResume::Restored {
                        id: worker.id,
                        messages: context.len(),
                    }),
                    Err(reason) => resumed.push(MemberResume::Skipped { name, reason }),
                }
            }
            Err(e) => resumed.push(MemberResume::Skipped {
                name,
                reason: e.to_string(),
            }),
        }
    }
    resumed
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;

    use wcode_harness::actor::{SessionActor, SessionHandle};
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::event::{AgentEvent, LlmStreamEvent};
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::message::StopReason;
    use wcode_harness::protocol::Request;
    use wcode_harness::session::SessionEntry;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};
    use wcode_protocol::Registry;

    use crate::agents::WorkerTemplate;
    use crate::config::ToolsConfig;

    /// A group with a real on-disk layout in `base/<id>` (dir + members/),
    /// with no `root.jsonl` unless the test writes one.
    fn make_group(base: &Path, id: &str) -> SessionGroup {
        let dir = base.join(id);
        fs::create_dir_all(dir.join("members")).unwrap();
        SessionGroup {
            root_path: dir.join("root.jsonl"),
            members_dir: dir.join("members"),
            dir,
        }
    }

    fn user_message(text: &str) -> SessionEntry {
        SessionEntry::Message {
            id: "m".into(),
            parent_id: None,
            message: AgentMessage::user_text(text),
        }
    }

    /// A model that never streams — enough to prove spawn/registration, since
    /// the `Notify` messages we deliver start no turn.
    fn stub_stream() -> StreamFn {
        Arc::new(|_ctx, _sys, _tools, _opts| Box::pin(futures::stream::empty()) as LlmStream)
    }

    /// A model that answers every run with one text delta.
    fn answering_stream() -> StreamFn {
        Arc::new(|_ctx, _sys, _tools, _opts| {
            Box::pin(futures::stream::iter(vec![
                LlmStreamEvent::TextDelta("the answer is 56".into()),
                LlmStreamEvent::Done {
                    stop_reason: StopReason::Stop,
                    usage: None,
                },
            ])) as LlmStream
        })
    }

    fn template_with(members_dir: Option<PathBuf>, stream_fn: StreamFn) -> WorkerTemplate {
        WorkerTemplate {
            system: "sys".into(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            tools: ToolsConfig::default(),
            compaction: CompactionPolicy::default(),
            working_dir: std::env::temp_dir(),
            members_dir,
            digest_cas: true,
            sessions_dir: crate::repl::session_dir(),
        }
    }

    fn orchestrator(members_dir: Option<PathBuf>) -> Orchestrator {
        Orchestrator::new(Registry::new(), template_with(members_dir, stub_stream()))
    }

    /// A root session handle to receive a worker's report-back.
    fn root_session() -> SessionHandle {
        SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts::default(),
            stream_fn: stub_stream(),
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: CompactionPolicy::default(),
        }))
    }

    // ---- layout & discovery ----

    #[test]
    fn create_group_lays_out_dir_and_members() {
        let base = tempfile::tempdir().unwrap();
        let group = create_group(base.path()).unwrap();

        assert!(group.dir.is_dir());
        assert!(group.dir.join("members").is_dir());
        assert!(!group.root_path.exists(), "root.jsonl is the caller's");
        assert_eq!(group.members_dir, group.dir.join("members"));

        let name = group.dir.file_name().unwrap().to_str().unwrap();
        assert!(
            is_group_dir_name(name),
            "generated name has the group shape: {name}"
        );
        assert_eq!(group.id(), name);
    }

    #[test]
    fn is_group_dir_name_accepts_millis_hex_and_rejects_the_rest() {
        assert!(is_group_dir_name("1768570000000_a1b2c3d4"));
        for bad in [
            "1768570000000.jsonl",          // no `_`
            "1768570000000_a1b2c3d4.jsonl", // the flat-file form, not the dir
            "1768570000000_a1b2c3d4g",      // non-hex id8
            "1768570000000_a1b2c3",         // id8 wrong length
            "root",                         // no `_`
            "no-us",
            "abc_a1b2c3d4",   // non-millis left of the `_`
            "1768570000000_", // empty id8
        ] {
            assert!(!is_group_dir_name(bad), "{bad}");
        }
    }

    #[test]
    fn list_groups_mixes_groups_and_legacy_newest_first() {
        let base = tempfile::tempdir().unwrap();
        make_group(base.path(), "100_a1b2c3d4");
        make_group(base.path(), "300_d4e5f6a7");
        fs::write(base.path().join("200_bbbbbbbb.jsonl"), "").unwrap();
        fs::write(base.path().join("150_cccccccc.jsonl"), "").unwrap();
        fs::create_dir(base.path().join("stuff")).unwrap();
        // A member file inside a group is never a top-level entry.
        fs::write(
            base.path()
                .join("300_d4e5f6a7")
                .join("members")
                .join("w1.jsonl"),
            "",
        )
        .unwrap();

        let list = list_groups(base.path()).unwrap();
        let ids: Vec<&str> = list.iter().map(|e| e.id()).collect();
        // Q(D): name-lexicographic descending, exactly like `list_sessions`.
        assert_eq!(
            ids,
            [
                "300_d4e5f6a7",
                "200_bbbbbbbb",
                "150_cccccccc",
                "100_a1b2c3d4"
            ]
        );
        assert!(list.iter().all(|e| e.id() != "w1"), "member files hidden");

        let groups: Vec<&str> = list
            .iter()
            .filter(|e| matches!(e, GroupEntry::Group(_)))
            .map(|e| e.id())
            .collect();
        assert_eq!(groups, ["300_d4e5f6a7", "100_a1b2c3d4"]);
        assert!(matches!(list[1], GroupEntry::Legacy { .. }), "200_ is flat");
    }

    #[test]
    fn list_groups_hides_a_legacy_file_colliding_with_a_group_dir() {
        let base = tempfile::tempdir().unwrap();
        make_group(base.path(), "100_a1b2c3d4");
        // The same root, left behind as a flat file during a move.
        fs::write(base.path().join("100_a1b2c3d4.jsonl"), "").unwrap();

        let list = list_groups(base.path()).unwrap();
        assert_eq!(
            list.len(),
            1,
            "the dir wins; the colliding flat file is hidden"
        );
        assert!(
            matches!(list[0], GroupEntry::Group(_)),
            "the surviving entry is the group"
        );
    }

    #[test]
    fn list_groups_survives_a_missing_session_dir() {
        let base = tempfile::tempdir().unwrap();
        let missing = base.path().join("nope");
        assert!(list_groups(&missing).unwrap().is_empty());
    }

    #[test]
    fn open_group_requires_root_jsonl_and_tolerates_missing_members() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_b");

        // No root.jsonl yet → NotFound.
        let err = open_group(&group.dir).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        // A directory that is not a session dir → NotFound too.
        let err = open_group(&base.path().join("nope")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        create_root_session(&group.dir, Path::new("/wd")).unwrap();
        fs::remove_dir_all(&group.members_dir).unwrap();
        let opened = open_group(&group.dir).unwrap();
        assert_eq!(opened.root_path, group.root_path);
        assert!(
            scan_members(&opened).unwrap().is_empty(),
            "missing members/ = empty team"
        );
    }

    #[test]
    fn group_dir_of_maps_a_group_dir_and_root_jsonl_only() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "1768570000000_a1b2c3d4");

        // (1) The group directory itself maps to itself.
        assert_eq!(group_dir_of(&group.dir).as_deref(), Some(group.dir.as_path()));

        // (2) `<groupdir>/root.jsonl` (the resume / `/reload` target) maps back up.
        let root = group.dir.join("root.jsonl");
        fs::write(&root, "").unwrap();
        assert_eq!(group_dir_of(&root).as_deref(), Some(group.dir.as_path()));

        // (3) A bare flat file (a legacy session) is not a group.
        let flat = base.path().join("1768570000000_a1b2c3d4.jsonl");
        fs::write(&flat, "").unwrap();
        assert_eq!(group_dir_of(&flat), None);

        // (3b) An unrelated directory is not a group (`is_group_dir_name` fails)…
        let plain = base.path().join("stuff");
        fs::create_dir(&plain).unwrap();
        assert_eq!(group_dir_of(&plain), None);
        // …and a `root.jsonl` inside it maps nowhere either.
        let other = plain.join("root.jsonl");
        fs::write(&other, "").unwrap();
        assert_eq!(group_dir_of(&other), None);
    }

    // ---- member records & the manifest fast path ----

    #[test]
    fn create_root_and_member_sessions_roundtrip_through_open() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        let root = create_root_session(&group.dir, Path::new("/wd")).unwrap();
        assert_eq!(root.path().unwrap(), group.root_path);
        assert_eq!(
            create_group_member_session(&group, "w1")
                .unwrap()
                .path()
                .unwrap(),
            group.member_path("w1")
        );

        // Appending to the member round-trips on reopen (SessionEntry unchanged).
        let mut member = Session::open(&group.member_path("w1")).unwrap();
        member.append(user_message("hello")).unwrap();
        let reopened = Session::open(&group.member_path("w1")).unwrap();
        assert_eq!(reopened.messages().len(), 1);
        assert_eq!(reopened.messages()[0].as_text(), "hello");

        // The root's header is intact.
        let root_again = Session::open(&group.root_path).unwrap();
        assert!(matches!(
            root_again.entries()[0],
            SessionEntry::Header { .. }
        ));
    }

    #[test]
    fn validate_member_name_rejects_slash_dotdot_and_jsonl_suffix() {
        assert!(validate_member_name("w1").is_ok());
        assert!(validate_member_name("reviewer-2").is_ok());
        for bad in ["a/b", "a\\b", "..", "", "x.jsonl", "../w1"] {
            assert!(validate_member_name(bad).is_err(), "{bad}");
        }
        // create_member_session enforces the same gate before touching disk.
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");
        assert!(create_member_session(&group.members_dir, "a/b").is_err());
    }

    #[test]
    fn scan_members_lists_only_jsonl_sorted() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");
        fs::write(group.member_path("w2"), "").unwrap();
        fs::write(group.member_path("w1"), "").unwrap();
        fs::write(group.members_dir.join("notes.txt"), "").unwrap();

        let names: Vec<String> = scan_members(&group)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["w1.jsonl", "w2.jsonl"]);
    }

    #[test]
    fn member_id_derives_session_id_from_file_stem() {
        assert_eq!(
            member_id(Path::new("members/w1.jsonl")),
            SessionId::agent("w1")
        );
        assert_eq!(
            member_id(&PathBuf::from("/g/members/reviewer.jsonl")),
            SessionId::agent("reviewer")
        );
    }

    #[test]
    fn manifest_roundtrips_member_specs() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        let full = WorkerSpec {
            name: Some("w1".into()),
            model: Some("m".into()),
            system: Some("role text".into()),
            tools: Some(vec!["read".into()]),
            base_url: Some("http://x".into()),
            api_key: Some("k".into()),
        };
        let record = MemberRecord::from_worker_spec(&full).unwrap();
        assert_eq!(record.role.as_deref(), Some("role text"), "system → role");

        let mut manifest = Manifest::new(group.id());
        manifest.members.push(record);
        write_manifest(&group, &manifest).unwrap();

        let loaded = load_manifest(&group).unwrap();
        assert_eq!(loaded.group, group.id());
        let back = loaded.members[0].to_worker_spec();
        assert_eq!(back.name, full.name);
        assert_eq!(back.model, full.model);
        assert_eq!(back.system, full.system);
        assert_eq!(back.tools, full.tools);
        assert_eq!(back.base_url, full.base_url);
        assert_eq!(back.api_key, full.api_key);

        // A nameless spec cannot be recorded (a record is always named).
        assert!(MemberRecord::from_worker_spec(&WorkerSpec::default()).is_none());
        // Absent manifest → None (scan path).
        assert!(load_manifest(&make_group(base.path(), "200_b")).is_none());
    }

    #[test]
    fn member_spec_falls_back_to_default_when_manifest_entry_missing() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        // No manifest at all: a default spec, name pinned, no overrides.
        let bare = member_spec(&group, "w1");
        assert_eq!(bare.name.as_deref(), Some("w1"));
        assert_eq!(bare.model, None);
        assert_eq!(bare.system, None);

        // A record under a different name does not satisfy `w1`.
        record_member(
            &group,
            &WorkerSpec {
                name: Some("stale".into()),
                model: Some("m".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(member_spec(&group, "w1").model, None);

        // With the matching record, its fields come back (name pinned to stem).
        record_member(
            &group,
            &WorkerSpec {
                name: Some("w1".into()),
                model: Some("m".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let got = member_spec(&group, "w1");
        assert_eq!(got.name.as_deref(), Some("w1"));
        assert_eq!(got.model.as_deref(), Some("m"));
    }

    #[test]
    fn record_member_upserts_instead_of_duplicating() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");
        record_member(
            &group,
            &WorkerSpec {
                name: Some("w1".into()),
                model: Some("old".into()),
                ..Default::default()
            },
        )
        .unwrap();
        record_member(
            &group,
            &WorkerSpec {
                name: Some("w1".into()),
                model: Some("new".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let manifest = load_manifest(&group).unwrap();
        assert_eq!(manifest.members.len(), 1, "replace-by-name, not append");
        assert_eq!(manifest.members[0].model.as_deref(), Some("new"));
        assert_eq!(manifest.group, group.id());
    }

    // ---- resume ----

    #[tokio::test]
    async fn torn_tail_member_transcript_is_tolerated_on_resume() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");
        let mut member = create_member_session(&group.members_dir, "w1").unwrap();
        member.append(user_message("prior task")).unwrap();
        append_torn_line(&group.member_path("w1"), "{\"type\":\"mess");

        let orch = orchestrator(Some(group.members_dir.clone()));
        let resumed = rebuild_team(&group, &orch, orch.id());
        assert_eq!(resumed.len(), 1);
        match &resumed[0] {
            MemberResume::Restored { id, messages } => {
                assert_eq!(id.as_str(), "agent:w1");
                assert_eq!(*messages, 1, "the intact message survives");
            }
            other => panic!("torn tail must not be fatal: {other:?}"),
        }
    }

    #[tokio::test]
    async fn corrupt_mid_file_member_is_skipped_not_fatal() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        // w1: header + message, then a torn line that is NOT last → open errors.
        let mut w1 = create_member_session(&group.members_dir, "w1").unwrap();
        w1.append(user_message("hi")).unwrap();
        append_torn_line(&group.member_path("w1"), "{\"type\":\"bogus");
        append_line(
            &group.member_path("w1"),
            "{\"type\":\"model_change\",\"id\":\"m\",\"model\":\"x\"}",
        );
        // w2: a clean, empty member.
        create_member_session(&group.members_dir, "w2").unwrap();

        let orch = orchestrator(Some(group.members_dir.clone()));
        let resumed = rebuild_team(&group, &orch, orch.id());
        assert_eq!(resumed.len(), 2);
        let skipped: Vec<&str> = resumed
            .iter()
            .filter_map(|r| match r {
                MemberResume::Skipped { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, ["w1"]);
        let restored: Vec<&str> = resumed
            .iter()
            .filter_map(|r| match r {
                MemberResume::Restored { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(restored, ["agent:w2"]);
        assert!(
            orch.registry().contains(&SessionId::agent("w2")),
            "the healthy member is fully registered"
        );
    }

    #[tokio::test]
    async fn rebuild_team_closes_registry_owners_and_names() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        // First life: two durable workers (transcripts + manifest on disk).
        let first = orchestrator(Some(group.members_dir.clone()));
        first
            .spawn_worker(WorkerSpec {
                name: Some("w1".into()),
                model: Some("m-w1".into()),
                system: Some("role".into()),
                ..Default::default()
            })
            .unwrap();
        first
            .spawn_worker(WorkerSpec {
                name: Some("reviewer".into()),
                ..Default::default()
            })
            .unwrap();
        drop(first); // a crash: registry + phonebook are gone

        // Second life: a fresh orchestrator rebuilds the team from the files.
        let second = orchestrator(Some(group.members_dir.clone()));
        let root = second.id().clone();
        let resumed = rebuild_team(&group, &second, &root);
        assert_eq!(resumed.len(), 2);
        let mut ids: Vec<String> = resumed
            .iter()
            .filter_map(|r| match r {
                MemberResume::Restored { id, .. } => Some(id.as_str().to_string()),
                _ => None,
            })
            .collect();
        ids.sort();
        assert_eq!(ids, ["agent:reviewer", "agent:w1"]);

        let reg = second.registry();
        let w1 = SessionId::agent("w1");
        let reviewer = SessionId::agent("reviewer");
        assert!(
            reg.contains(&w1) && reg.contains(&reviewer),
            "both registered"
        );
        assert!(reg.permitted(&w1, &root), "member may report to root");
        assert!(reg.permitted(&root, &w1), "root may address member");
        assert!(reg.permitted(&reviewer, &root) && reg.permitted(&root, &reviewer));
        assert_eq!(
            reg.model_of(&w1).as_deref(),
            Some("m-w1"),
            "set_model restored"
        );
        // The phonebook names resolve to live, addressable members.
        assert!(
            second.worker_backend("w1").is_some(),
            "phonebook closes `w1`"
        );
        assert!(
            second.worker_backend("reviewer").is_some(),
            "phonebook closes `reviewer`"
        );
        assert_eq!(
            member_spec(&group, "w1").system.as_deref(),
            Some("role"),
            "the role survived the manifest fast path"
        );
    }

    #[tokio::test]
    async fn resume_roundtrip_members_report_to_the_root() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");

        // Life 1: spawn a team and see the worker report back to the root.
        let first = Orchestrator::new(
            Registry::new(),
            template_with(Some(group.members_dir.clone()), answering_stream()),
        );
        let root1 = root_session();
        first.register_root(root1.clone());
        let w1 = first
            .spawn_worker(WorkerSpec {
                name: Some("w1".into()),
                ..Default::default()
            })
            .unwrap();
        let mut root1_rx = root1.subscribe();
        first
            .registry()
            .deliver(
                first.id(),
                &w1.id,
                Request::Wake {
                    content: "do it".into(),
                },
            )
            .unwrap();
        let first_report = tokio::time::timeout(Duration::from_secs(3), root1_rx.recv())
            .await
            .expect("the worker reports")
            .expect("open");
        assert!(
            matches!(&first_report, AgentEvent::MessageReceived { from, content }
                if from == &w1.id && content == "the answer is 56"),
            "{first_report:?}"
        );
        drop(first); // crash: registry + factory gone

        // The worker's transcript is now durable (header + its prior turns).
        let on_disk = Session::open(&group.member_path("w1")).unwrap();
        let prior = on_disk.messages().len();
        assert!(prior >= 2, "prior turns persisted: {prior}");

        // Life 2: a fresh orchestrator rebuilds the team from the group.
        let second = Orchestrator::new(
            Registry::new(),
            template_with(Some(group.members_dir.clone()), answering_stream()),
        );
        let root2 = root_session();
        second.register_root(root2.clone());
        let root2_id = second.id().clone();
        let resumed = rebuild_team(&group, &second, &root2_id);
        assert_eq!(resumed.len(), 1);
        match &resumed[0] {
            MemberResume::Restored { id, messages } => {
                assert_eq!(id.as_str(), "agent:w1");
                assert!(
                    *messages >= 2,
                    "full prior transcript seeded (D1): {messages}"
                );
            }
            other => panic!("{other:?}"),
        }

        // A new task reaches the restored worker, which reports to the new root —
        // the whole report-back edge is live again.
        let mut root2_rx = root2.subscribe();
        second
            .registry()
            .deliver(
                &root2_id,
                &SessionId::agent("w1"),
                Request::Wake {
                    content: "again".into(),
                },
            )
            .unwrap();
        let second_report = tokio::time::timeout(Duration::from_secs(3), root2_rx.recv())
            .await
            .expect("the restored worker reports")
            .expect("open");
        assert!(
            matches!(&second_report, AgentEvent::MessageReceived { from, content }
                if from == &SessionId::agent("w1") && content == "the answer is 56"),
            "{second_report:?}"
        );
    }

    #[test]
    fn load_root_extracts_model_and_effort_from_the_transcript() {
        let base = tempfile::tempdir().unwrap();
        let group = make_group(base.path(), "100_a");
        let mut root = create_root_session(&group.dir, Path::new("/wd")).unwrap();
        root.append(SessionEntry::ModelChange {
            id: "m".into(),
            model: "claude-x".into(),
        })
        .unwrap();
        root.append(SessionEntry::EffortChange {
            id: "e".into(),
            effort: Some("high".into()),
        })
        .unwrap();

        let (s, messages) = load_root(&group).unwrap();
        assert_eq!(s.model().as_deref(), Some("claude-x"));
        assert_eq!(s.effort(), Some(Some("high".into())));
        assert!(messages.is_empty());
    }

    /// Append a raw line (a torn write / a future entry) to a file.
    fn append_line(path: &Path, line: &str) {
        use std::io::Write as _;
        let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(format!("{line}\n").as_bytes()).unwrap();
    }

    fn append_torn_line(path: &Path, partial: &str) {
        append_line(path, partial);
    }
}
