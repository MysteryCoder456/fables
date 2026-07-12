# notion-tui — Production Polish Design (v1.0 push)

**Date:** 2026-07-12
**Status:** Approved for implementation planning
**Predecessor:** `2026-07-05-notion-tui-design.md` (v1 spec, milestones 1–5, complete)

## 1. Purpose

Take the feature-complete v1 of notion-tui to a credible public 1.0: safe for
strangers' workspaces, first-class on Linux (x86_64 + aarch64), macOS, and
Windows, and shipped through an automated release pipeline. Sequencing is
trust-first: fix data-corrupting and silent-failure bugs before widening the
audience, then close every v1 spec-vs-implementation gap, then polish
interaction, then ship. Post-1.0 features are explicitly out of scope and get
their own brainstorm after M11.

## 2. Audit baseline (2026-07-12)

Two full audits (UI/UX and production-readiness) ground this plan. Verified
baseline: `cargo test --workspace` passes (~220 tests), clippy clean,
`cargo fmt --check` fails (389 hunks, no rustfmt.toml). Core sync semantics
(FIFO ops, conflict primitives, temp-ID rewriting, transactional store,
panic-safe terminal restore) are solid and well-tested. The findings below are
referenced by milestone; file references were verified at audit time.

Severity legend: **[T]** trust/data-safety, **[S]** spec gap, **[P]** polish,
**[I]** infrastructure.

## 3. Milestones

### M6 — Trust & data safety

Fix everything that can corrupt data, lose data, or fail silently. A thin CI
gate lands first so every subsequent milestone is protected.

1. **Thin CI gate.** GitHub Actions: `cargo test --workspace`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo fmt --check` on
   ubuntu/macos/windows runners. Requires `rustfmt.toml` capturing the
   existing style (or a one-time reformat) so the fmt gate can pass.
2. **[T] HTTP timeouts.** `notion-api/src/client.rs` uses
   `reqwest::Client::new()` — no timeout; a stalled connection freezes sync
   forever. Add request + connect timeouts and flip status to Offline on
   timeout.
3. **[T] Relation/people property corruption.** `ui/props.rs`
   `build_property_value` falls through to `Value::Null` for
   relation/people; editing (or even confirming) those fields wipes them
   remotely. Make relation/people fields read-only in the form (real pickers
   are a post-1.0 feature) and make the `_ => Null` fallback impossible —
   unknown property types must refuse to commit.
4. **[T] Duplicate writes on retry.** The client blind-retries non-idempotent
   POSTs (page/block/comment creation) on 5xx/connection errors, and the
   pusher re-sends ops left `pending` after ambiguous failures. Restrict
   automatic retry to idempotent requests; on ambiguous create failures,
   verify (refetch children/row) before re-sending.
5. **[T] Puller data loss.** `puller.rs` calls `.ok()` on every store write
   and advances the high-water mark regardless; a failed write means the page
   is never pulled again. Propagate store errors; never advance hwm on a
   failed cycle.
6. **[T] Sync tick resets view state.** `refresh_current_view` rebuilds
   Page/Table views from scratch (cursor, collapsed toggles, sort all reset)
   on every data-version tick. Preserve position/state the way Board already
   does; only reset what structurally changed.
7. **[T] Silent $EDITOR flows.** Editor launch/exit failures are discarded
   (`Err(_) => return`) and successful round-trips discard the
   inserted/updated/deleted summary. Route both into the status-bar notice
   ("editor failed: …", "3 updated · 1 inserted · 2 deleted").
8. **[T] Markdown fence safety.** An unclosed code fence swallows the rest of
   the page into one code block with no warning. Detect unterminated fences
   on parse and confirm before applying ("unclosed code fence at line N —
   apply anyway / re-edit / discard").
9. **[T] Schema forward-compatibility.** `migrate()` accepts a DB with a
   `user_version` newer than the app; error clearly instead, and copy the DB
   aside before running any migration.
10. **Smaller hardening (same theme):** wrap `replace_comments` in a
    transaction; handle store-mutex poisoning so a panicked holder doesn't
    silently kill sync (surface sync-task death in `SyncStatus`).

### M7 — Finish the v1 spec

Close every promised-but-missing behavior, or amend the spec explicitly
(§5 records the descopes).

1. **[S] Command palette parity.** Implement `rename` (page/row title),
   `move page` (re-parent via a picker), `group-by` (choose the board's
   grouping property — also fixes the hard-picked first-status behavior),
   `sync now` (manual pull trigger). Palette matching becomes subsequence
   ("fuzzy") rather than substring, and workspace search gains subsequence
   re-ranking on top of FTS prefix matching (see §5).
2. **[S] Mouse support.** Spec §4.5 promises click-to-focus, click
   links/sidebar entries, click column headers to sort, wheel scroll (exists),
   and board card drag. Implement click routing per pane + header-sort +
   card drag; on by default, still never required.
3. **[S] Universal back-history.** History is pushed only for in-page link
   follows; sidebar opens, table/board row opens, and search jumps bypass it.
   Push on every navigation so `Backspace`/`-` behaves browser-style.
4. **[S] Table filtering.** A filter key opens a per-column (or free-text
   across the row) filter; filter state survives sync ticks (per M6.6) and is
   visible in the table title.
5. **[S] First-crawl progress + checkpointing.** The puller emits
   `Syncing { done: 0 }` forever and a large first crawl (≥2 req/page, 334 ms
   pacing) restarts from scratch on any late error. Checkpoint the hwm
   incrementally, emit real progress counts, and render "syncing 240/1,893
   pages" in the status bar and wizard handoff.
6. **[S] Status bar completeness.** Add the current breadcrumb path
   (workspace → page → subpage) and pending-keystroke feedback (`d` pending
   for `dd`), both promised in spec §4.1.
7. **[S] Block placeholders.** Images render as `[image: caption]` per spec;
   other unsupported blocks get named placeholders (`[bookmark: url]`,
   `[table]`, `[embed]`) instead of a bare `⍰`.
8. **[S] Remote deletion propagation.** Search-driven pull never removes
   pages that were deleted/trashed/un-shared, so ghosts accumulate forever.
   Add a periodic mark-and-sweep reconciliation pass plus a 404/archived
   check on page open.

### M8 — Interaction polish

The "first hour of real use" annoyances.

1. **[P] Real text editing.** All text inputs (inline block edit, property
   text fields, search, palette) gain Left/Right/Home/End/Delete, a visible
   terminal cursor (`set_cursor`), and grapheme-safe editing (no broken emoji
   after Backspace).
2. **[P] Theme the modals.** search/input/props/confirm/palette/help all
   ignore the theme today; thread `&Theme` through them so dark/light isn't
   half-applied.
3. **[P] Human-readable queue.** Queue/conflict rows show page/row titles and
   property names ("edit ¶ in 'Meeting Notes'", "set Status on 'Q3 Launch'")
   instead of raw UUIDs. (Deeper conflict UX is M10.)
4. **[P] Empty states.** Sidebar/page/table/search/comments/queue each get a
   one-line hint when empty ("no results", "no comments yet — press n",
   "first sync in progress…").
5. **[P] Consistent motion keys.** `Ctrl+d/u` and `g/G` work in Table and
   Board, not just Page.
6. **[P] Wizard honesty.** Distinguish 401 ("invalid token") from network
   failure ("can't reach Notion — check your connection") in the first-run
   wizard and any token re-validation.
7. **[P] Friendly config errors.** TOML parse failures name the file and
   line; unknown theme names and unparseable key bindings warn instead of
   silently defaulting.
8. **[P] Layout niceties.** Type-aware column widths (checkbox narrow, URL
   wide) with ellipsis on clipped cells; soft-wrap long page lines; code
   blocks get a language label.
9. **[P] Destructive-action consistency.** `dd` gains the same confirm
   treatment as protected-block deletion (or an undo toast), and the
   session-only undo stack is documented in help.
10. **[P] Help completeness.** Help overlay lists fixed keys (Tab, Ctrl+d/u,
    Ctrl+P, Backspace-as-back); spec's "fully rebindable" claim is amended to
    name the fixed keys, and `parse_key` accepts `ctrl+x` syntax for the
    bindings that can safely move.

### M9 — Shipping essentials

Everything a public artifact needs besides the release pipeline itself
(which is M11).

1. **[I] CLI surface.** `--version`, `--help`, `--config <path>`,
   `--db-path <path>`, `--debug` handled before the TUI starts (today
   `--version` launches the app or the wizard).
2. **[I] Observability.** `tracing` across all four crates writing to a
   rotating log file in the state dir; `--debug`/`RUST_LOG` raise verbosity;
   panic hook writes the backtrace to the log after restoring the terminal;
   README documents the log path for bug reports.
3. **[I] Sync engine economics.** Offline/failure backoff with cap (no fixed
   30 s hammering); share one `NotionClient` between UI and sync so combined
   pacing respects ~3 rps; hwm overlap window to tolerate eventually-
   consistent search ordering.
4. **[I] Publishing hygiene.** Per-crate `license-file`/LICENSE copies,
   `readme`, `keywords`, `categories`, `documentation`, subdirectory-aware
   `repository` links, `rust-version` (MSRV) in `[workspace.package]` and
   tested in CI.
5. **[I] Docs storefront.** README demo GIF (vhs/asciinema), troubleshooting
   section (keyring issues, DB/log locations, reset instructions), sync &
   conflict explainer, FAQ, CHANGELOG.md (seeded, then automated by M11),
   CONTRIBUTING.md.
6. **[I] Windows fit-and-finish.** Default editor fallback (`notepad`) when
   `$EDITOR`/config is unset; verify paths, keyring, and terminal behavior on
   Windows CI.

### M10 — Conflict experience

Make conflicts legible: a user should see *where* the conflict is, *what*
changed on each side, and what each resolution will do — without leaving the
TUI or cross-referencing IDs.

1. **Conflict location.** Conflicted items are marked at their source: a
   marker glyph next to the affected page in the sidebar, the affected row in
   table/board views, and a status-bar count (`⚠ 2 conflicts`) that is
   already partially present gains a jump action (key + palette command
   "conflicts") straight to the queue screen with the first conflict
   selected.
2. **Conflict identity.** Building on M8.3's human-readable queue rows, each
   conflicted op shows: target page/row title, property or block affected,
   when your edit was made, when and (if the API provides it) by whom the
   remote changed.
3. **Conflict detail view.** `Enter` on a conflicted op opens a detail pane:
   local and remote versions rendered side-by-side (page conflicts render via
   the existing Markdown renderer; property conflicts show old → new values),
   with an inline unified diff for text bodies.
4. **Informed resolution.** The keep-mine / take-theirs / merge actions are
   presented in the detail view with one-line consequence descriptions
   ("keep mine — overwrites the remote edit shown right"); merge continues to
   open $EDITOR with both versions, now preceded by this context.
5. **Non-blocking flow.** Resolving one conflict returns to the queue with
   the next conflict selected; resolving the last returns to the previous
   view.

### M11 — Automated semantic-versioned CI/CD

One merged PR should be able to become a versioned, fully published release
with no manual steps.

1. **Conventional commits → semver.** The repo already uses conventional
   commits. Adopt `release-plz`: on merge to main it opens/updates a release
   PR with version bumps (fix → patch, feat → minor, `!`/BREAKING → major)
   and generated CHANGELOG entries across the workspace crates (unified
   workspace version). A PR-title/commit lint in CI keeps the convention
   honest.
2. **Publish on release-PR merge.** Merging the release PR tags `vX.Y.Z` and
   publishes the four crates to crates.io in dependency order.
3. **Binary distribution.** The tag triggers `cargo-dist`: static musl
   builds for x86_64/aarch64 Linux, macOS (universal or per-arch), and
   Windows; artifacts + checksums attached to the GitHub Release; installer
   script; Homebrew tap formula updated automatically. AUR and winget
   manifests are stretch goals, not gates.
4. **Release gates.** The release workflow runs the full M6 CI gate on all
   three OSes before anything publishes; a failed gate blocks the release,
   never half-publishes (crates.io publish happens only after all binaries
   build).
5. **Retire `scripts/build-release.sh`** (or reduce it to a local
   convenience wrapper documented as such).

**1.0 definition:** M6–M11 complete, released through the M11 pipeline.

## 4. Sequencing rationale

Trust-first (M6) because the audit found bugs that corrupt or lose real user
data; public exposure before fixing them would poison first impressions.
The thin CI gate is pulled to the very front (M6.1) because it is ~a day of
work and protects all subsequent milestones. Spec completion (M7) precedes
polish (M8) because several polish passes sweep surfaces M7 introduces
(modal theming and text-editing polish must cover the new filter input and
group-by/move-page pickers, or they'd be built twice). M10 sits after M8
because it extends M8's queue work.
M11 is last so the first fully automated release is the 1.0 itself, but its
commit-convention lint can be adopted any time earlier at zero cost.

## 5. Descope decisions (v1 spec amendments)

Recorded here so the spec and the product stop disagreeing:

- **Search is subsequence-fuzzy, not typo-tolerant.** M7.1 upgrades matching
  to subsequence (fzf-style) for the palette and FTS prefix + subsequence
  re-ranking for search; true typo tolerance (trigram/levenshtein) is
  post-1.0.
- **Relation/people editing is read-only** until proper pickers ship
  (post-1.0); the property form displays but refuses to edit them (M6.3).
- **Fixed keys stay fixed.** Tab, Ctrl+d/u, Ctrl+P remain non-rebindable;
  the spec's "fully rebindable" language is amended (M8.10).
- **Date ranges:** the property form edits `start` only; editing preserves an
  existing `end` untouched rather than dropping it (folded into M6.3's
  "never destroy what you can't edit" rule). Full range editing is post-1.0.

## 6. Out of scope (post-1.0 brainstorm)

Quick-capture hotkey, multiple workspaces, gallery/calendar/timeline views,
inline images, relation/people pickers, typo-tolerant search, realtime
presence, daemon split. A separate brainstorm follows the 1.0 release.

## 7. Verification

Each milestone lands with tests in the existing suites (wiremock for
api/sync, TestBackend for UI flows, temp-DB for store) plus:

- **M6:** regression tests for each fixed bug (null-property guard, hwm
  non-advance on store error, retry idempotency, view-state preservation);
  CI green on all three OSes.
- **M7:** flow tests per new feature; a manual acceptance pass against a
  real workspace (large-workspace crawl shows progress and survives
  interruption; deletion reconciliation removes a trashed page).
- **M8:** render smoke tests extended to empty states and themed modals;
  grapheme-editing property tests.
- **M9:** `notion-tui --version` works from a packaged binary on each OS;
  a bug report can be filed from the log file alone.
- **M10:** e2e conflict test extended to assert markers, detail view
  content, and resolution consequences.
- **M11:** a dry-run release from a test tag produces installable artifacts
  on all three platforms; a `fix:` commit produces a patch bump PR.
