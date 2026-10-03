# Crash recovery for multi-file resource commits

## Purpose and scope

The resource model stores each public document as JSON, while raw engine-owned
book/catalog data lives under `private-data`. A source replacement or catalog
refresh changes several files that must describe the same generation after a
process crash. The `resource_transactions` helper now implements a private JSON
journal and staged snapshots for those commits. It does not use SQLite,
involve the WebView, or claim that multiple file renames are one
filesystem-atomic operation. The helper is not yet wired into
`ApplicationService` startup or `ResourceStore` writes; current source/catalog
operations retain their existing runtime rollback behavior until that
integration is completed.

Initial integration scope:

- `ApplicationService::change_book_source`: private book/catalog JSON, public
  book JSON, progress JSON, shelf JSON, and bookmarks JSON. Old chapter HTML
  files are deferred cleanup actions.
- `ApplicationService::refresh_catalog_unlocked`: private raw chapter catalog,
  public book JSON, progress JSON, and shelf JSON. Removed chapter HTML files
  are deferred cleanup actions.

Engine calls remain outside the commit. Existing source, book, and operation
locks still validate the snapshot before a journal is prepared.

## What the current code guarantees

`ResourceStore::write_json_ref` serializes one JSON document and atomically
replaces that file through `atomicwrites::AtomicFile`; its write closure calls
`sync_all` on the temporary file. `update_json_ref` also serializes public
read/modify/write operations under one in-process mutex. These are useful
single-file guarantees. They do not group several documents into one commit,
and current resource writes do not explicitly sync the containing directory
after the atomic rename.

Private JSON uses the same atomic replacement crate, but
`ApplicationService::write_private_json` currently calls `write_all` without
`sync_all`. Source replacement and catalog refresh write private/public book,
progress, shelf, and (for source replacement) bookmarks in sequence. They try
to restore old values when a later operation returns an error. A process exit
between writes bypasses those rollback branches, so startup can observe a
mixture of old and new documents. Chapter cache deletion already occurs after
the JSON writes; that ordering should be retained and represented in the
journal as post-commit cleanup.

Backup restore is a related precedent, not a reusable file transaction as-is.
It validates a complete ZIP into a sibling staging directory, writes a
`RestoreJournal`, renames the active root to a displaced sibling, activates the
stage, and recovers those directory-rename windows before opening
`ResourceStore`. It handles whole-root replacement; source/catalog operations
need per-file before/after snapshots. The restore journal write also does not
currently sync its file or parent directory, so the proposed durability helper
should make those steps explicit rather than assuming the current code is
power-loss durable.

## Recommended protocol: undo before commit, redo after commit

Use a single active transaction per app-data root. Store its journal and
payloads in a private, non-served directory such as
`private-data/transactions/<transaction-id>/`. Backup enumeration must exclude
that directory. A backup can only start after startup recovery and under the
existing exclusive operation guard, so a valid in-flight journal should never
be included in an archive.

The existing Tokio mutexes only coordinate one `ApplicationService` process.
This protocol therefore assumes one writer process per data root. If the
desktop/mobile launch model can open the same root twice, acquire an
OS-level app-data lock before recovery and hold it for the service lifetime;
otherwise a second process could overwrite the journal or public targets
without sharing the in-memory writer guard.

Each journal contains:

```json
{
  "format": "legado-rs-file-transaction",
  "version": 1,
  "transaction_id": "txn-…",
  "operation": "catalog-refresh",
  "phase": "prepared",
  "targets": [
    {
      "domain": "public",
      "path": "books/book-id/book.json",
      "before": { "missing": false, "blob": "before/000000.bin", "bytes": 123, "sha256": "…" },
      "after": { "missing": false, "blob": "after/000000.bin", "bytes": 145, "sha256": "…" }
    }
  ],
  "post_commit_deletes": ["books/book-id/chapters/old-id.html"]
}
```

`before` may record `missing: true` when a file did not exist. `after` is the
new exact byte snapshot. Store payloads as files beside the journal rather
than embedding potentially large documents in the journal itself. Capture
exact old bytes; do not reconstruct old progress or metadata from partial
structs during recovery.

The protocol is:

1. Keep the existing source/book locks and acquire a ResourceStore-wide
   transaction writer guard before reading or staging any public target. This
   guard must also exclude ordinary `write_json_ref` and `update_json_ref`
   writers, especially updates to the shared `shelf.json` and
   `bookmarks.json`. All source/book validation and snapshots must be reread
   under their existing locks.
2. Validate every target path with the same public allowlist or a dedicated
   private-path validator. Reject symlinks, duplicate targets, traversal,
   and unsupported file kinds before writing a journal. Cache cleanup entries
   must be typed chapter-resource references or pass a strict equivalent
   validator; never accept arbitrary paths from JSON.
3. Write and `sync_all` all `before` and `after` payloads. Sync their staging
   directories. Atomically write the journal in `prepared` phase, sync the
   journal file, then sync the journal's parent directory. No target is
   modified before this prepared record is durable.
4. Verify every staged `after` payload before changing the first target, then
   apply each snapshot using a single-file atomic replacement. `missing` is
   valid only for a before-image; rollback restores it by removing the target
   idempotently. Sync each output file and every changed containing directory
   before proceeding. The transaction writer guard stays held throughout.
5. Once all targets are durable, atomically rewrite the journal phase to
   `committed`, sync the journal file, and sync the parent directory. This is
   the commit decision. Return success only after this marker is durable.
6. Apply the journal's post-commit cache deletions and sync affected
   directories. Remove the journal and sync its parent. Remove the now-orphaned
   staging directory last. A cleanup error after the committed marker must not
   roll the JSON documents back; leave the journal for recovery and report that
   the data commit succeeded but cleanup is pending.

If a normal error happens before the committed marker, restore all `before`
snapshots while the writer guard is still held. If rollback itself fails,
leave the prepared journal intact for startup recovery and return both errors.
The helper verifies the complete set of before-images before changing any
target during rollback, so a damaged later snapshot cannot cause a partial
rollback.
After the committed marker, only finish the new state and cleanup; do not
attempt an old-state rollback.

## Startup recovery

Call recovery after `backup::recover_interrupted_restore` and before
`ResourceStore::open`, source loading, HTTP serving, or Tauri bootstrap. This
order first resolves any whole-root restore, then resolves file transactions
inside whichever app-data tree became active. `ResourceStore::open` writes
default documents when files are absent, so it must not run before either
recovery step.

Recovery is idempotent:

- `prepared`: verify the journal and every referenced `before` payload, then
  restore all targets to their before state. Do not run post-commit deletions.
- `committed`: verify all `after` payloads, replay every after-state write,
  replay post-commit deletions, then remove the journal and stage directory.
- A stage directory with no journal is safe to remove: target writes are
  forbidden until the durable prepared journal exists.
- Invalid schema, unsafe paths, missing payloads, or checksum mismatches must
  stop startup with a clear recovery error and preserve the journal and
  payloads. Do not silently pick a generation or initialize an empty library.

Only one journal is expected because commits are serialized per root. If
startup finds more than one valid journal, stop and report the conflict rather
than guessing transaction order. Recovery logs should report operation ID and
relative target names only; never print source rules or document payloads.

## Helper API and integration state

The new module exposes crate-private typed inputs. A public book JSON target
uses a validated `ResourceRef` such as
`resource://books/<book-id>/book.json`; the private snapshot target is built
from a validated book ID and maps only to
`private-data/books/<book-id>.json`. Chapter cleanup accepts only a chapter
HTML `ResourceRef`. It validates exact JSON bytes and rejects transient
resource-server capability URLs in `src`/`*Src` fields before staging.

The current API shape is:

```rust
let tx = FileTransaction::prepare(
    app_data_root,
    "catalog-refresh",
    vec![
        Replacement::public_json(book_ref, book_json_bytes)?,
        Replacement::public_json(progress_ref, progress_json_bytes)?,
        Replacement::private_book_json(book_id, private_book_json_bytes)?,
    ],
    vec![PostCommitDelete::chapter(old_chapter_ref)?],
    &resource_store_writer_guard,
)?;
tx.commit()?;
```

The caller must acquire the `ResourceStore` global JSON writer guard before
reading any target snapshot and retain it through `commit`. The helper has no
private per-file mutex. Its guard trait checks that the protected canonical
app-data root matches the transaction root; the future application adapter
must implement the trait for the actual store-wide guard.

`AppDataProcessLock` uses stable `File::try_lock` advisory locking on a sibling
lock file. Acquire it before backup restore recovery and hold it through the
service lifetime; it does not create a missing root, so it cannot hide an
interrupted root rename from recovery. The lock file stays outside app data so
backup root renames do not replace its inode. This lock is provided for later
app-lifecycle integration and is not currently acquired by startup.
`recover_all(root)` must then run after backup root-restore recovery and before
`ResourceStore::open`, source loading, HTTP serving, or bootstrap. A pending
journal blocks later writes until recovery succeeds.

Integration must preserve these constraints:

- `ResourceStore` owns one writer guard shared with normal public JSON writes,
  so a transaction cannot commit a stale shelf/bookmarks snapshot.
- Public targets use validated `ResourceRef`s. Private targets use a
  dedicated validated relative-path type rooted under `private-data`; do not
  pass absolute `PathBuf`s or raw source-controlled paths.
- The helper records old bytes itself while the caller holds the appropriate
  application locks, and it applies staged bytes through an internal
  already-locked atomic-write path. It must not call public write methods while
  holding their mutex recursively.
- `commit` distinguishes failures before and after the durable commit marker,
  so callers cannot mistakenly run compensating rollback after commit.
- Recovery is synchronous and available before `ResourceStore::open`.
- The helper stays Rust-only. JS continues to read the ordinary public JSON
  resources; journal and stage files are private implementation data.

The helper lives in `resource_transactions.rs` so private app-data paths stay
out of `resources.rs`. Do not create a second JSON store or duplicate the
`ResourceRef` public-path allowlist.

## Durability and visibility limits

An atomic rename protects an individual file from a torn write. A journal
allows startup to restore one consistent generation after a process crash, but
the sequence of per-file replacements is not a cross-file atomic operation.
During the short live commit window, a WebView that independently fetches
several resources may observe files from different moments. The first slice
should serialize Rust writers and perform startup recovery; it must not claim
instantaneous multi-resource snapshot isolation. If the UI later requires that
stronger guarantee, use versioned resource generations with one atomic
manifest pointer, or expose the combined view as one resource. Do not solve it
by having JS coordinate file writes.

For power-loss durability, file `sync_all` is insufficient by itself: the
directory entry created by a rename/unlink must also be flushed where the OS
supports it. The helper's `sync_directory` flushes directory handles on Unix;
on non-Unix it is currently a documented no-op because Rust's portable `std`
API does not expose a uniform directory-sync contract. Windows behavior is
therefore weaker and neither Android nor iOS metadata durability has been
validated on-device. This module does not claim power-loss atomicity. The
existing public writer may reuse the primitive later without changing the
resource JSON schema.

## Fault-injection verification

The helper has headless temporary-directory tests; they do not bring GTK/WebKit
into the test binary. They reproduce the durable journal and target-file states
at these boundaries, then invoke startup recovery:

1. Before a durable prepared journal: an unjournaled staging directory is
   swept on reopen. This test constructs the orphan directory directly.
2. After prepared, before any target write: startup restores old snapshots.
3. After each individual target replacement and after all replacements but
   before the committed marker: startup restores the entire old generation.
4. After all replacements and the durable committed marker: startup completes
   the new generation and performs the cache deletion.
5. During post-commit cleanup, after one deletion: startup repeats the cleanup
   safely without reverting documents.
6. After journal removal but before stage removal: startup sweeps the harmless
   orphan stage.

The helper tests also cover path traversal/symlink rejection, checksum
fail-closed behavior without partially restoring or redoing targets when a
later payload is damaged, a previously missing target, and writer-guard root
matching. Tests also call `commit()` with an induced target-write failure and
with a post-commit cleanup failure, then verify the error's commit state and
recovery result. A damaged after-image is also rejected before target writes,
and the intact before-images are restored before the prepared journal is
removed. The process-lock test opens the same root from a child process and
after replacing the root directory at the same path. These exercise actual
file operations and OS locking, but do not kill a process at arbitrary machine
instructions or simulate power loss. Real `ApplicationService` reopen tests,
multi-document domain invariants, concurrent shelf-update exclusion, and
mobile/Windows directory-sync behavior still require app and platform
integration; this helper alone does not establish those guarantees.

The existing `backup::tests::interrupted_restore_recovers_after_either_directory_rename`
remains the model for restore crash windows. Transaction tests need their own
file-level crash boundaries and must not describe backup recovery as covering
them.
