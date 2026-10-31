//! **A conversation's identity, and how to ask for one** (W040 §13.3).
//!
//! # What this module is
//!
//! The two types that name **a conversation** — one independent ROOT runtime in
//! this process — and **how to build one**. Locked design:
//! `docs/work/W040-multi-root-conversations.md` §13.2 items 1–2 (the §13 pivot
//! supersedes that brief's original §4.1/§4.2).
//!
//! They live in `wcode-protocol` — not `wcode-tui`, not `wcode-cli` — for one
//! reason, and it is the *whole* feasibility question the human asked: a
//! conversation must be nameable by **three** parties that cannot all depend on
//! each other:
//!
//! | party | depends on | needs |
//! |---|---|---|
//! | the TUI (W040) | `wcode-protocol` | `ConversationId` — BESIDE the spec, not in it (B3) |
//! | the CLI (W040) | `wcode-protocol`, `wcode-tui` | mints ids, builds roots |
//! | `wcode-serve` (W041) | `wcode-protocol` **never** `wcode-tui` | `ConversationId` in a URL, `RootSpec` to spawn |
//!
//! `wcode-serve` must not link `ratatui` (`crates/wcode-tui/Cargo.toml`), and
//! `wcode-tui` must not link `wcode-cli`. `wcode-protocol` is the only crate
//! both can see, so the id + the build request belong here — this is the ONE
//! type this item promotes into a published crate (§13.3's "unchanged" is about
//! the *wire*: no `Frame` field, no `PROTOCOL_VERSION` bump; see [`ConversationId`]).
//!
//! # What this module deliberately does NOT contain
//!
//! - **Nothing serialized on the wire.** No `Frame` field, no `Request`/
//!   `AgentEvent` variant. `PROTOCOL_VERSION` stays `1`
//!   (`crates/wcode-harness/src/protocol.rs:34` `pub const PROTOCOL_VERSION: u32 = 1;`).
//! - **No factory.** `RootSpec` is a *request*; building a root needs
//!   `Arc<Background>`, `Agent`, `Orchestrator` — all `wcode-cli` types
//!   (`crates/wcode-cli/src/repl.rs:388` `pub fn build_agent(`). The factory is
//!   injected by the composition root, exactly as `DefineHandler` is
//!   (`crates/wcode-protocol/src/server.rs:68` `pub type DefineHandler =` — "the
//!   composition root injects one, since this crate cannot name a session
//!   factory"). That precedent is §13.2 item 2's own citation.
//! - **No container.** The TUI's tab view-model is `wcode-tui::conversations`;
//!   the CLI's live-root registry is `wcode-cli`'s. Only the *keys* live here.
//!
//! # Integration points (re-verified this session)
//!
//! - `crates/wcode-tui/src/lib.rs:257` `pub struct SurfaceSpec {` — gains **no**
//!   field (first-layer B3; `crates/wcode-tui/src/conversations.rs`'s module
//!   header is authoritative): the tag rides BESIDE the spec, as
//!   `(ConversationId, SurfaceSpec)` on the member feed, because
//!   `SurfaceSpec::info()` (`lib.rs:268`-region) drops it at `lib.rs:581`
//!   (`app.add_surface(spec.info());`).
//! - `crates/wcode-protocol/src/server.rs:41` `pub type Root = (SessionId, SessionHandle);`
//!   — one root *is* one conversation's served tuple today; `RootSpec` is its
//!   build-time twin.
//! - `crates/wcode-protocol/src/registry.rs:114` `pub fn register(` — a live
//!   root registers here; `:202` `pub fn remove(` un-registers on close.

use std::path::PathBuf;

/// **A conversation's identity** — opaque, host-minted, monotone, stable for the
/// life of the process.
///
/// MINTED BY THE HOST (the CLI today, `wcode-serve` in W041), never by a client:
/// the same value is the key the TUI groups surfaces by
/// (`wcode_tui::conversations::Conversations`), the key the CLI's live-root
/// registry uses (`wcode_cli`'s `Roots`), and — W041 — the path segment in
/// `POST /api/conversation/{id}/request`. A client that invented its own id
/// could not address a conversation it did not create.
///
/// # Contracts
///
/// - **Distinct.** Two live conversations never share an id; the host's
///   `Roots::insert` refuses a duplicate (a `debug_assert!` + `Result`, not a
///   silent overwrite — the `Orchestrator::spawn_worker` precedent,
///   `crates/wcode-cli/src/agents.rs:742`), and `Roots::new` reserves seq `0`
///   for the startup conversation (`first()`) so the first mint is `c1`.
/// - **Not a `SessionId`.** A conversation is a *root and its team*; a session is
///   one agent. The root's `SessionId` is `agent:orchestrator`
///   (`crates/wcode-cli/src/agents.rs:780` `pub fn id(&self) -> &SessionId`) or
///   the literal `agent:root` when there is no orchestrator
///   (`crates/wcode-cli/src/main.rs:2040` `unwrap_or_else(|| SessionId::agent("root"))`)
///   — i.e. **not unique across conversations**. Routing stays one-level
///   (`SessionId`); this id is the *grouping* key. See "the one flat surface-id
///   key" in the
///   W040 sketch report: the CLI must mint unique SURFACE ids per conversation
///   for the one-level routing to stay correct.
/// - **`Display` is short and loggable** (`c3`), never the session id.
/// - **`serde`**: `Serialize + Deserialize`, `#[serde(transparent)]`, so W041 can
///   put it in a URL/JSON without inventing a second spelling. W040 serializes
///   it nowhere — the derive is for the *next* consumer and costs nothing (it is
///   NOT a `Frame` field, so `PROTOCOL_VERSION` is untouched).
///
/// # Errors / Panics
///
/// None. Construction cannot fail; an unparsable string is the caller's problem
/// (`ConversationId` is not parsed from untrusted input until W041, which must
/// validate a path segment before building one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ConversationId(u64);

impl ConversationId {
    /// Wrap a host-minted sequence number. `0` is the startup conversation — the
    /// one `wcode` opens — so a single-conversation process always has id 0 and
    /// every "the only conversation" test can name it.
    pub fn new(seq: u64) -> Self {
        Self(seq)
    }

    /// The sequence number (the CLI's mint source: `next += 1` per root).
    pub fn seq(self) -> u64 {
        self.0
    }

    /// The startup conversation. Named so no call site writes `ConversationId::new(0)`
    /// and drifts from the CLI's mint rule.
    pub fn first() -> Self {
        Self(0)
    }
}

impl std::fmt::Display for ConversationId {
    /// `c0`, `c1`, … — the tab bar's log token and W041's URL segment.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// **What the client asks the host to build**: one root runtime, from scratch or
/// from a session group on disk.
///
/// The TUI (`Action::NewConversation`) and, later, `wcode-serve` both hand one of
/// these to the CLI's injected factory. There is deliberately **no field for the
/// model, the team, the tools or the hooks**: those are the *process's*
/// (`llm`/`cfg`/`rt` at
/// `crates/wcode-cli/src/main.rs:1931` `async fn dispatch(`), already resolved at
/// startup, and a per-tab override would be a second config surface — `AGENTS.md`:
/// "config files for behavior" is absent by design.
///
/// # Contracts
///
/// - The factory that consumes this is **pure w.r.t. process state**: it creates a
///   session group, builds one agent, spawns one actor, and returns a handle. It
///   must NOT `exit`, and must NOT print to stderr (the `load_session`/
///   `build_runtime` pair it is refactored from does both — "the exiting,
///   printing factory body").
/// - `Fresh` creates a new group dir
///   (`crates/wcode-cli/src/session_groups.rs:47` `pub struct SessionGroup {` —
///   `dir`/`root_path`/`members_dir`), so every conversation has its own
///   `root.jsonl`, `members/`, and `plan.ndjson` (`:81`
///   `pub fn plan_path(&self) -> PathBuf {`).
/// - `Resume` takes a group dir, not a file, because a resumed team must come back
///   with its root (`load_root`); a bare flat `.jsonl` is the legacy path and
///   stays reachable by resuming it as a one-member group.
///
/// # Not promised
///
/// - No ordering guarantee between two `Fresh` builds (each mints its own dir).
/// - No deduplication: `Resume(dir)` twice builds two live roots over one dir.
///   The host is expected to refuse it (`Roots::contains_session_path`) — a
///   second writer to one `root.jsonl` would interleave two transcripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootSpec {
    /// A brand-new session group + root.
    Fresh {
        /// The tab label to seed, `None` = "no seed requested".
        ///
        /// PRECEDENCE (W040 §13.6.1): `agent.title()` → this seed → the cwd's
        /// basename (`crates/wcode-cli/src/main.rs:2022` seeds `Options.cwd` from
        /// it). The `Agent` accessor comes FIRST because it is the same source the
        /// header already reads: a `/tab new` root built with `--title`, or a
        /// `Resume` that restored a `Title` entry, has a name from the first frame
        /// and must not disagree with the band above it. A label never survives
        /// long anyway: the first `AgentEvent::SessionTitle` (W031) replaces it
        /// ("the tab label's single source").
        label: Option<String>,
    },
    /// Resume an existing group dir on disk (`--resume <dir>` semantics).
    Resume {
        /// `<session_dir>/<millis>_<id8>` — the group directory
        /// (`crates/wcode-cli/src/session_groups.rs:49` `    pub dir: PathBuf,`).
        dir: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn ids_are_distinct_per_sequence_and_display_short() {
        assert_ne!(ConversationId::new(2), ConversationId::new(3));
        assert_eq!(ConversationId::new(2).seq(), 2);
        assert_eq!(ConversationId::new(2).to_string(), "c2");
        // The startup conversation, and the reservation that keeps it unique.
        assert_eq!(ConversationId::first().seq(), 0);
        assert_eq!(ConversationId::first().to_string(), "c0");
    }

    #[test]
    fn an_id_is_hashable_and_ordered_by_sequence() {
        // The TUI stores ids in a `HashMap`/`BTreeMap`; ordering must be mint
        // order, not pointer order (`Hash + Ord` both derived).
        let mut seen: HashMap<ConversationId, &str> = HashMap::new();
        seen.insert(ConversationId::new(2), "two");
        seen.insert(ConversationId::first(), "zero");
        assert_eq!(seen.get(&ConversationId::new(2)).copied(), Some("two"));
        assert_eq!(seen.len(), 2);

        // Rust 2024 allows `c0..=c2` here; spell the seqs out so the assertion is
        // about the derived `Ord`, not about a `Range` impl.
        let mut ordered: BTreeMap<ConversationId, ()> = BTreeMap::new();
        for seq in [2_u64, 0, 1] {
            ordered.insert(ConversationId::new(seq), ());
        }
        let seqs: Vec<u64> = ordered.keys().map(|id| id.seq()).collect();
        assert_eq!(seqs, vec![0, 1, 2], "ordered by mint sequence");
    }

    #[test]
    fn root_spec_fresh_and_resume_are_distinguishable() {
        let dir = PathBuf::from("/tmp/group");
        let fresh = RootSpec::Fresh { label: None };
        let resume = RootSpec::Resume { dir: dir.clone() };
        assert_ne!(fresh, resume, "the factory cannot confuse the two arms");
        // `Fresh` carries no dir, so the same path cannot make them equal.
        assert_ne!(RootSpec::Fresh { label: Some("x".into()) }, fresh);
        assert_eq!(resume.clone(), RootSpec::Resume { dir });
    }

    #[test]
    fn an_id_serializes_transparently_for_the_browser() {
        // W041's URL segment; NOT a wire type of this item (no `Frame` change).
        assert_eq!(
            serde_json::to_string(&ConversationId::new(7)).unwrap(),
            "7"
        );
        let back: ConversationId = serde_json::from_str("7").unwrap();
        assert_eq!(back, ConversationId::new(7));
    }

    /// **The module adds nothing to the wire** (W040 §13.3). This is the pin that
    /// stops a later hand from putting `ConversationId` into `Frame` "because it
    /// serializes": the version stays `1` and a `Request` frame's JSON is exactly
    /// what it was.
    #[test]
    fn the_wire_is_untouched_by_this_module() {
        use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

        assert_eq!(PROTOCOL_VERSION, 1, "no protocol bump for W040");

        let frame = Frame::new(0, SessionId::new("s1"), Request::Cancel);
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(
            json,
            r#"{"v":1,"id":0,"session":"s1","type":"cancel"}"#,
            "a Request frame is byte-identical to the pre-W040 wire"
        );
        for word in ["conversation", "RootSpec", "root_spec", " \"c"] {
            assert!(
                !json.contains(word),
                "the frame must not grow a conversation field: {json}"
            );
        }
    }
}
