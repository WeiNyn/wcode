//! # Workspace concurrency — the `WorkspaceHooks` digest policy (item 18).
//!
//! See `docs/workspace-concurrency.md`. One responsibility: **digest-CAS
//! plumbing**.
//!
//! - `transform_tool_input` — auto-attach the session's last-read whole-file
//!   digest to `write`/`edit`/`edits`/`replace`.
//! - `after_tool_call` — capture the digest a `read` emitted into the cache.
//!
//! Verification itself is **not** here: each tool re-reads its target anyway
//! (`edit.rs`/`replace.rs`/`edits.rs` `read_to_string`, `write.rs`'s `old` read)
//! and compares that content's digest to `expected_digest` (Decision 5.1:
//! stateless, per-call). This hook only *plumbs the value*.
//!
//! ## Why a hook, not the kernel (design §4)
//!
//! The kernel already exposes both seams: `Hooks::transform_tool_input` (the
//! only arg-mutating seam, run before `before_tool_call`) and
//! `Hooks::after_tool_call` (invoked with the *transformed* call and its
//! output, so it can harvest the read digest).
//!
//! ## Per-session state — one instance PER agent
//!
//! The cache is session-local, so a FRESH `WorkspaceHooks` is built for every
//! agent **inside `build_agent`** (which drives `AgentConfig.hooks`) and inside
//! `SessionFactory::worker_config_with`. This covers the initial agent, every
//! REPL in-process rebuild (`/new`/`/resume` re-call `build_agent`), and every
//! worker — with no per-session leak. It must NOT live in the shared
//! `hooks` / `WorkerTemplate.hooks` set.
//!
//! ## Friction
//!
//! - `after_tool_call` receives **no `working_dir`** (only `id`/`name`/
//!   `arguments`), so the read digest must ride the `read` output text and the
//!   cache must be keyed on the *caller-supplied* `path` string, not an absolute
//!   path.
//! - `HooksSet` is fixed at `Agent` construction, so this hook cannot be
//!   swapped mid-session.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use wcode_harness::hooks::{Hooks, ToolCall};
use wcode_harness::tool::ToolOutput;

/// The tool names whose args this hook may inject `expected_digest` into.
/// Decision 5.3: **all four** mutators, uniform.
const MUTATOR_TOOLS: &[&str] = &["write", "edit", "edits", "replace"];

/// The top-level args key the digest is injected under — for every mutator,
/// including `edits` (the field lives on `EditsArgs`, NOT on the nested
/// `EditOp`). A5.
pub(crate) const EXPECTED_DIGEST_KEY: &str = "expected_digest";

/// The cache key for a caller-supplied path — the raw `path` string normalized
/// lexically so `f.txt`, `./f.txt`, and `sub/../f.txt` collide (they address the
/// same file). `crate::tools::normalize` returns a `PathBuf`, so convert with
/// `to_string_lossy().into_owned()` (the cache is `HashMap<String, String>`). No
/// `working_dir` is available in `after_tool_call`, so this stays purely lexical
/// (never resolved to absolute).
pub(crate) fn cache_key(raw_path: &str) -> String {
    crate::tools::normalize(Path::new(raw_path))
        .to_string_lossy()
        .into_owned()
}

/// The header line `read` prints in anchored mode, carrying the whole-file
/// digest so `after_tool_call` can harvest it. MUST stay in sync with
/// [`parse_digest_header`] and with `crate::tools::read`'s emit site.
///
/// Contract: contains the 12 lowercase hex chars of
/// [`anchor::file_digest`](crate::tools::anchor::file_digest) and **never** the
/// `anchor::ANCHOR_SEP` glyph, so a consumer that scans for `ANCHOR│` skips it.
pub(crate) fn digest_header(path: &str, digest: &str) -> String {
    format!("# {path} digest {digest}\n")
}

/// Parse the digest out of [`digest_header`]'s line, if `line` is one.
/// `None` for any other line (an anchor line, a page note, raw text).
///
/// Splits on the LAST `" digest "` (`rsplit_once`) so a path that itself
/// contains the substring `" digest "` cannot mis-split. Contract: total over
/// all `&str`; never panics.
pub(crate) fn parse_digest_header(line: &str) -> Option<String> {
    line.strip_prefix("# ")
        .and_then(|rest| rest.rsplit_once(" digest "))
        .map(|(_, digest)| digest.to_string())
}

/// The whole-file-CAS refusal, mirroring `E_STALE_ANCHOR`'s shape (D1).
///
/// The three existing `E_STALE_ANCHOR` literals are left untouched; this is the
/// one shared helper for the **digest** case. Returns the exact string the four
/// mutators return as an `is_error: true` `ToolOutput` (writing nothing).
pub(crate) fn stale_digest(path: &str, expected: &str, actual: &str) -> String {
    format!(
        "[E_STALE_DIGEST] {path} changed since your read (expected {expected}, found {actual}) — re-read and retry."
    )
}

/// The workspace digest policy (design §4): auto-attach last-read digests.
/// Built per agent; toggled by `[workspace] digest_cas` (default ON).
pub struct WorkspaceHooks {
    /// Last whole-file digest seen per caller-supplied path key (Decision 5.1:
    /// the cache is *plumbing*; verification is per-call, stateless).
    /// Key = [`cache_key`] (`tools::normalize(path).to_string_lossy()`).
    /// `std::sync::Mutex` because both `Hooks` methods take `&self` and the
    /// critical section is a single map op (never held across an `await`).
    last_read: Mutex<HashMap<String, String>>,
    /// `[workspace] digest_cas`. When `false`, both methods short-circuit — the
    /// toggle is HONORED here (one source of truth), not a silent no-op.
    digest_cas: bool,
}

impl WorkspaceHooks {
    /// Build the policy hook. `digest_cas` comes from `cfg.workspace.digest_cas`
    /// (default `true`). Constructed fresh per agent (see the module doc).
    pub fn new(digest_cas: bool) -> Self {
        Self {
            last_read: Mutex::new(HashMap::new()),
            digest_cas,
        }
    }

    /// Extract the mutation target path from a call's arguments, per tool:
    /// `write`/`edit`/`replace` → `arguments["path"]`; `edits` →
    /// `arguments["edits"][0]["path"]` (ops must share one file —
    /// `E_MIXED_FILES`). Returns the RAW path string; callers key on
    /// [`cache_key`].
    fn target_path(&self, name: &str, args: &serde_json::Value) -> Option<String> {
        match name {
            "write" | "edit" | "replace" => args
                .get("path")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            "edits" => args
                .get("edits")
                .and_then(|e| e.as_array())
                .and_then(|ops| ops.first())
                .and_then(|op| op.get("path"))
                .and_then(|p| p.as_str())
                .map(str::to_string),
            _ => None,
        }
    }

    /// The digest to inject, if the cache has one for the call's target path
    /// **and** the caller did not already supply `arguments["expected_digest"]`.
    /// `None` ⇒ leave the call untouched (unguarded write).
    fn expected_digest_for(&self, name: &str, args: &serde_json::Value) -> Option<String> {
        // A present `expected_digest` (even a wrong one) is an explicit override.
        if args.get(EXPECTED_DIGEST_KEY).is_some() {
            return None;
        }
        let path = self.target_path(name, args)?;
        self.last_read
            .lock()
            .unwrap()
            .get(&cache_key(&path))
            .cloned()
    }

    /// Record a successful `read`'s digest (`after_tool_call`), keyed by
    /// [`cache_key`] of the read's `path`.
    fn note_read(&self, args: &serde_json::Value, out: &ToolOutput) {
        let Some(path) = args.get("path").and_then(|p| p.as_str()) else {
            return;
        };
        let Some(digest) = out.output.lines().next().and_then(parse_digest_header) else {
            return;
        };
        self.last_read
            .lock()
            .unwrap()
            .insert(cache_key(path), digest);
    }
}

#[async_trait::async_trait]
impl Hooks for WorkspaceHooks {
    /// Auto-attach `expected_digest` to a mutator call whose args lack one
    /// (Decision 5.2: harness-side plumbing; Decision 5.1: the value came from
    /// the last `read`). A present `expected_digest` is an explicit override and
    /// is left alone. A5: insert at the TOP-LEVEL `arguments["expected_digest"]`
    /// for all four tools (never inside `edits[]`).
    ///
    /// Contract: never errors, never blocks; a cache miss is a no-op (the write
    /// proceeds unguarded, exactly as today). Short-circuits when `!digest_cas`.
    async fn transform_tool_input(&self, call: &mut ToolCall) {
        if !self.digest_cas || !MUTATOR_TOOLS.contains(&call.name.as_str()) {
            return;
        }
        let Some(digest) = self.expected_digest_for(&call.name, &call.arguments) else {
            return;
        };
        if let Some(obj) = call.arguments.as_object_mut() {
            obj.insert(
                EXPECTED_DIGEST_KEY.to_string(),
                serde_json::Value::String(digest),
            );
        }
    }

    /// Harvest a `read`'s digest into the cache so the next mutation of that
    /// path is armed. `out.is_error` ⇒ skip; `!digest_cas` ⇒ skip.
    async fn after_tool_call(&self, call: &ToolCall, out: &mut ToolOutput) {
        if !self.digest_cas || call.name != "read" || out.is_error {
            return;
        }
        self.note_read(&call.arguments, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "t1".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[tokio::test]
    async fn transform_tool_input_attaches_last_read_digest() {
        let h = WorkspaceHooks::new(true);
        h.last_read
            .lock()
            .unwrap()
            .insert(cache_key("f.txt"), "abc123".into());
        let mut c = call("write", serde_json::json!({"path": "f.txt", "content": "x"}));
        h.transform_tool_input(&mut c).await;
        assert_eq!(c.arguments[EXPECTED_DIGEST_KEY], "abc123");
    }

    #[tokio::test]
    async fn explicit_expected_digest_wins_over_cache() {
        let h = WorkspaceHooks::new(true);
        h.last_read
            .lock()
            .unwrap()
            .insert(cache_key("f.txt"), "cached".into());
        let mut c = call(
            "edit",
            serde_json::json!({"path": "f.txt", "expected_digest": "explicit"}),
        );
        h.transform_tool_input(&mut c).await;
        assert_eq!(c.arguments[EXPECTED_DIGEST_KEY], "explicit");
    }

    #[tokio::test]
    async fn edits_digest_is_top_level_not_per_op() {
        // A5: the batch is one file, so one TOP-LEVEL key — never per-op.
        let h = WorkspaceHooks::new(true);
        h.last_read
            .lock()
            .unwrap()
            .insert(cache_key("f.txt"), "batch".into());
        let mut c = call(
            "edits",
            serde_json::json!({"edits": [{"path": "f.txt", "from": "a", "replacement": "b"}]}),
        );
        h.transform_tool_input(&mut c).await;
        assert_eq!(c.arguments[EXPECTED_DIGEST_KEY], "batch");
        assert!(c.arguments["edits"][0].get(EXPECTED_DIGEST_KEY).is_none());
    }

    #[test]
    fn cache_key_is_lexically_normalized() {
        assert_eq!(cache_key("./f.txt"), cache_key("f.txt"));
        assert_eq!(cache_key("sub/../f.txt"), cache_key("f.txt"));
    }

    #[tokio::test]
    async fn failed_read_does_not_arm_the_cache() {
        let h = WorkspaceHooks::new(true);
        let c = call("read", serde_json::json!({"path": "f.txt"}));
        let mut out = ToolOutput {
            output: "# f.txt digest abc123\n".into(),
            is_error: true,
            ..ToolOutput::default()
        };
        h.after_tool_call(&c, &mut out).await;
        assert!(h.last_read.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn note_read_parses_the_header_digest() {
        let h = WorkspaceHooks::new(true);
        let c = call("read", serde_json::json!({"path": "f.txt"}));
        let mut out = ToolOutput {
            output: "# f.txt digest deadbeef0000\nAB1CD│line\n".into(),
            ..ToolOutput::default()
        };
        h.after_tool_call(&c, &mut out).await;
        assert_eq!(
            h.last_read.lock().unwrap().get(&cache_key("f.txt")),
            Some(&"deadbeef0000".to_string())
        );

        // #6: a path containing the literal " digest " still parses — the split
        // is on the LAST separator.
        assert_eq!(
            parse_digest_header("# a digest b digest deadbeef0000"),
            Some("deadbeef0000".to_string())
        );
    }

    #[tokio::test]
    async fn disabled_hook_is_inert() {
        let h = WorkspaceHooks::new(false);
        h.last_read
            .lock()
            .unwrap()
            .insert(cache_key("f.txt"), "abc".into());
        let mut c = call("write", serde_json::json!({"path": "f.txt"}));
        h.transform_tool_input(&mut c).await;
        assert!(c.arguments.get(EXPECTED_DIGEST_KEY).is_none());

        let rc = call("read", serde_json::json!({"path": "g.txt"}));
        let mut out = ToolOutput {
            output: "# g.txt digest zzz\n".into(),
            ..ToolOutput::default()
        };
        h.after_tool_call(&rc, &mut out).await;
        assert!(h.last_read.lock().unwrap().get(&cache_key("g.txt")).is_none());
    }

    #[test]
    fn header_roundtrips_and_is_not_an_anchor_line() {
        let header = digest_header("src/f.txt", "0123456789ab");
        assert_eq!(header, "# src/f.txt digest 0123456789ab\n");
        assert_eq!(
            parse_digest_header(header.trim_end()),
            Some("0123456789ab".to_string())
        );
        // The header must not look like an anchor line to a SEP-scanning reader.
        assert!(!header.contains(crate::tools::anchor::ANCHOR_SEP));
        assert_eq!(parse_digest_header("AB1CD│line"), None);
        assert_eq!(parse_digest_header("… (3 more lines)"), None);
    }

    #[test]
    fn stale_digest_message_names_both_digests_and_the_reread_hint() {
        let msg = stale_digest("f.txt", "aaaa", "bbbb");
        assert!(msg.starts_with("[E_STALE_DIGEST] f.txt"), "{msg}");
        assert!(msg.contains("expected aaaa"), "{msg}");
        assert!(msg.contains("found bbbb"), "{msg}");
        assert!(msg.contains("re-read"), "{msg}");
    }
}

/// Item 19.1 — the end-to-end chain with **no fake seams**: the real `Read`
/// tool emits the digest header, the real hook caches it and attaches it to a
/// mutator call, and the real `Write` tool verifies it. This covers the wiring
/// *between* the pieces (header text ⇄ parser, `arguments` JSON ⇄ tool args),
/// not just each piece in isolation.
#[cfg(test)]
mod chain {
    use super::*;
    use wcode_harness::tool::TypedTool;

    use crate::tools::anchor;
    use crate::tools::read::{Read, ReadArgs};
    use crate::tools::test_ctx;
    use crate::tools::edit::{Edit, EditArgs};
    use crate::tools::write::{Write, WriteArgs};

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wcode_harness::hooks::HooksSet;

    /// A minimal local hook (the harness's `RecordingHooks` is crate-local):
    /// records that it ran and leaves an observable marker, so a composed pass
    /// is checkable. `after_tool_call` appends to the END so it never disturbs
    /// the digest header at `output`'s first line.
    #[derive(Default)]
    struct Recorder {
        transforms: AtomicUsize,
        outputs: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Hooks for Recorder {
        async fn transform_tool_input(&self, call: &mut ToolCall) {
            self.transforms.fetch_add(1, Ordering::SeqCst);
            if let Some(obj) = call.arguments.as_object_mut() {
                obj.insert("recorder".into(), serde_json::json!(true));
            }
        }

        async fn after_tool_call(&self, _call: &ToolCall, out: &mut ToolOutput) {
            self.outputs.fetch_add(1, Ordering::SeqCst);
            out.output.push('!');
        }
    }

    fn read_call(path: &str) -> ToolCall {
        ToolCall {
            id: "r1".into(),
            name: "read".into(),
            arguments: serde_json::json!({ "path": path }),
        }
    }

    fn write_call(path: &str, content: &str) -> ToolCall {
        ToolCall {
            id: "w1".into(),
            name: "write".into(),
            arguments: serde_json::json!({ "path": path, "content": content }),
        }
    }

    #[tokio::test]
    async fn read_arms_the_hook_and_the_real_write_is_cas_guarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "original\n").unwrap();
        let (ctx, _rx) = test_ctx(dir.path());

        // 1. The real `read` emits the digest header as its FIRST line.
        let mut out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: Some(false),
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            parse_digest_header(out.output.lines().next().unwrap()),
            Some(anchor::file_digest(b"original\n")),
            "the read header carries the file's digest"
        );

        // 2. `after_tool_call` caches that digest under the read's path.
        let h = WorkspaceHooks::new(true);
        h.after_tool_call(&read_call("f.txt"), &mut out).await;
        assert!(
            h.last_read.lock().unwrap().contains_key(&cache_key("f.txt")),
            "the read armed the cache"
        );

        // 3. A mutator call with NO `expected_digest` gets it injected top-level.
        let mut w = write_call("f.txt", "NEW");
        h.transform_tool_input(&mut w).await;
        assert_eq!(
            w.arguments[EXPECTED_DIGEST_KEY],
            anchor::file_digest(b"original\n")
        );

        // 4. The real `write` verifies against the live file and succeeds.
        let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let wargs: WriteArgs = serde_json::from_value(w.arguments.clone()).unwrap();
        let out = Write::new(lock.clone()).execute(wargs, &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "NEW");

        // 5. A peer rewrites the file out-of-band: the armed digest is now stale.
        std::fs::write(&path, "PEER").unwrap();
        let mut w2 = write_call("f.txt", "AGAIN");
        h.transform_tool_input(&mut w2).await;
        assert_eq!(
            w2.arguments[EXPECTED_DIGEST_KEY],
            anchor::file_digest(b"original\n"),
            "the stale digest is still attached"
        );
        let wargs2: WriteArgs = serde_json::from_value(w2.arguments.clone()).unwrap();
        let out = Write::new(lock).execute(wargs2, &ctx).await;
        assert!(out.is_error, "a stale write must refuse: {}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        // Non-destructive: the peer's content is byte-for-byte untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "PEER");
    }

    #[tokio::test]
    async fn a_disabled_hook_injects_nothing_into_the_real_call() {
        let h = WorkspaceHooks::new(false);
        h.last_read
            .lock()
            .unwrap()
            .insert(cache_key("f.txt"), "abc123".into());
        let mut w = write_call("f.txt", "NEW");
        h.transform_tool_input(&mut w).await;
        assert!(w.arguments.get(EXPECTED_DIGEST_KEY).is_none());
    }

    /// Item 20.1 — the same chain through the real `edit` tool: proves anchor
    /// resolution still runs *after* a passing CAS check, and that a stale base
    /// is refused before anything is written.
    #[tokio::test]
    async fn read_arms_the_hook_and_the_real_edit_is_cas_guarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "a = 1\nb = 2\n").unwrap();
        let (ctx, _rx) = test_ctx(dir.path());

        // The real `read` (anchored) emits the header; the hook caches its digest.
        let mut out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: Some(false),
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let h = WorkspaceHooks::new(true);
        h.after_tool_call(&read_call("f.txt"), &mut out).await;

        // An `edit` call (no `expected_digest`) gets it attached top-level.
        let mut e = ToolCall {
            id: "e1".into(),
            name: "edit".into(),
            arguments: serde_json::json!({
                "path": "f.txt",
                "from": anchor::anchor("a = 1"),
                "replacement": "a = 10",
            }),
        };
        h.transform_tool_input(&mut e).await;
        assert_eq!(
            e.arguments[EXPECTED_DIGEST_KEY],
            anchor::file_digest(b"a = 1\nb = 2\n")
        );

        // The real `edit` passes the CAS check and still resolves the anchor.
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let eargs: EditArgs = serde_json::from_value(e.arguments.clone()).unwrap();
        let out = Edit::new(lock.clone()).execute(eargs, &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 10\nb = 2\n");

        // A peer rewrites the file: the armed digest is stale → refuse, no write.
        std::fs::write(&path, "peer\n").unwrap();
        let mut e2 = ToolCall {
            id: "e2".into(),
            name: "edit".into(),
            arguments: serde_json::json!({
                "path": "f.txt",
                "from": anchor::anchor("peer"),
                "replacement": "x",
            }),
        };
        h.transform_tool_input(&mut e2).await;
        assert_eq!(
            e2.arguments[EXPECTED_DIGEST_KEY],
            anchor::file_digest(b"a = 1\nb = 2\n"),
            "the stale digest is still attached"
        );
        let eargs2: EditArgs = serde_json::from_value(e2.arguments.clone()).unwrap();
        let out = Edit::new(lock).execute(eargs2, &ctx).await;
        assert!(out.is_error, "a stale edit must refuse: {}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "peer\n");
    }

    /// Item 20.2 — the digest policy composed with another hook in a real
    /// `HooksSet`, driven the way the loop drives it (both seams run every hook
    /// in insertion order). Covers the ordering seam, not just the hook alone.
    #[tokio::test]
    async fn the_real_hook_set_composes_in_order() {
        let rec = Arc::new(Recorder::default());
        let set = HooksSet::from_iter([
            rec.clone() as Arc<dyn Hooks>,
            Arc::new(WorkspaceHooks::new(true)) as Arc<dyn Hooks>,
        ]);

        // transform: the recorder runs first (marker), then the digest hook.
        let mut w = write_call("f.txt", "NEW");
        set.transform_tool_input(&mut w).await;
        assert_eq!(rec.transforms.load(Ordering::SeqCst), 1);
        assert_eq!(w.arguments["recorder"], true);
        assert!(
            w.arguments.get(EXPECTED_DIGEST_KEY).is_none(),
            "an empty cache attaches nothing"
        );

        // Both hooks run on after_tool_call; the recorder leaves its marker...
        let mut read_out = ToolOutput {
            output: "# f.txt digest cafebabe0000\nAB1CD│line\n".into(),
            ..ToolOutput::default()
        };
        set.after_tool_call(&read_call("f.txt"), &mut read_out).await;
        assert_eq!(rec.outputs.load(Ordering::SeqCst), 1);
        assert!(read_out.output.ends_with('!'), "{}", read_out.output);

        // ...and the composed transform now attaches the digest it cached.
        let mut w2 = write_call("f.txt", "NEW");
        set.transform_tool_input(&mut w2).await;
        assert_eq!(w2.arguments[EXPECTED_DIGEST_KEY], "cafebabe0000");
    }

    /// Item 20.3 — a `plain: true` read has no header, so it arms nothing.
    #[tokio::test]
    async fn a_plain_read_arms_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello\nworld\n").unwrap();
        let (ctx, _rx) = test_ctx(dir.path());

        let mut out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: Some(true),
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        // The first line is raw `cat -n`, not a digest header...
        assert_eq!(parse_digest_header(out.output.lines().next().unwrap()), None);
        // ...so after_tool_call arms nothing for that path.
        let h = WorkspaceHooks::new(true);
        h.after_tool_call(&read_call("f.txt"), &mut out).await;
        assert!(h.last_read.lock().unwrap().get(&cache_key("f.txt")).is_none());
    }
}
