# Agent instructions for `legado_rs`

## Product and repository scope

- `legado` is the retired reference application. Do not develop, build, or test it. Develop `legado_rs` only.
- Implement the complete useful feature set represented by the mature parts of `legado`, with a complete and usable Vue UI. Treat old source code as behavior/UI evidence. Do not copy old bugs, wrong behavior, or unfinished/placeholder UI; implement the intended useful function when the reference is incomplete.
- Advance Android, desktop (Windows/macOS/Linux), and iOS together. HarmonyOS is outside the current product scope.
- Keep the existing Kotlin Multiplatform source engine. Rust may call and coordinate it. Do not port or rewrite it as part of current feature work. Continue accepting the source JSON/rules the engine already supports. Compatibility with old bookshelf, progress, settings, and backup data is not required.

## Architecture: preserve the agreed responsibility boundary

The application flow is **JS/WebView receives the user's interaction → Rust performs all intermediate business processing → JS/WebView reads and displays the final resources**.

- JS handles interaction, UI state, responsive layout, and direct display of already prepared resources. Page turning, pagination, reading a chapter already available by `src`, and switching to an available chapter do not need a Rust call.
- Rust is the business entry point between UI intent and final resources. It coordinates source selection, work, network, source-engine calls, persistence, and resource preparation, then notifies JS about changed state/resources. Use IPC for intent, status, cancellation/errors, and resource locations; do not send large chapter/media payloads through IPC as the ordinary display path.
- Kotlin/KMP alone executes source definitions, source rules, and scripts. A source-management UI may collect source configuration as user input and ask Rust to validate, import, save, or invoke the engine. The WebView must not execute source rules, parse source definitions to perform business work, or consume an unprocessed source definition as a display resource. Reading consumes only processed results/resources.
- Do not introduce SQLite. Persist bookshelf, progress, settings, task state, and other structured application state as JSON files. Rust owns application-file reads/writes; JS reads JSON resources and does not directly mutate the application's files.
- Persist each cached chapter as browser-consumable HTML, not as JSON or plain text. HTML includes the engine-processed chapter body and default reading style. Book/chapter JSON exposes the chapter's `src`; local resources and remote URLs must use the same browser-consumption model. Images and media are separate resources addressed by URLs.
- JS reads the cached HTML and may override its default reading style with the user's current preferences. User display replacement rules are a second, presentation-only stage in JS. Neither the style override nor the display replacement is written back to cached HTML. Source-rule replacement remains engine work and the cached chapter contains its processed result.
- JS checks whether chapter resources are sufficiently cached on book open or when approaching the next chapter. When more content is needed, JS asks Rust; Rust obtains/processes it through the source engine, writes HTML and JSON/resource references, then notifies JS. Existing resource navigation remains in JS.
- JS records low-frequency reading progress from UI interaction and delegates JSON persistence to Rust at capturable lifecycle points such as leaving a book or entering the background. Deleting books and other persistent mutations also go through Rust and return via a resource update/notification. Never rely on an OS force-kill always delivering a final callback.

## Implementation and verification

- The user delegated code writing to a GPT-6 Luna sub-agent at **xhigh** reasoning. The coordinator owns task decomposition, architecture/interface coordination, reviews, integration, and end-to-end validation. Respect this delegation for code tasks; do not quietly switch to an unrelated implementation role/model.
- The user authorized autonomous decisions and asked not to be interrupted with implementation questions. Resolve normal choices using the architecture and code evidence. For a material ambiguity or blocker, record the evidence, decision, impact, and validation in `docs/decisions.md`, then continue independent work.
- Test dependencies must match what each test exercises. Do not add heavyweight GTK/WebKitGTK dependencies to headless logic tests unless that test actually exercises the desktop UI. Do not remove dependencies required by the real application to make a test pass.
- A successful compile or unit test is not proof that a user feature works. A feature is complete only after its user flow is usable end to end: UI interaction, Rust/KMP business handling, persistence/resource delivery, JS consumption, and recovery/error behavior as applicable. Validate Android, desktop, and iOS separately. Record actual actions, environments, results, and gaps in `docs/validation.md`; update `docs/feature-matrix.md` only after the agreed acceptance flow passes.
- Do not claim the whole app is complete while required features or platform flows remain unimplemented, unverified, or blocked.
- The user has authorized regular GitHub commits and pushes without per-push confirmation. Push each verifiable feature milestone; during work lasting roughly 30 minutes, check whether a stable, compilable slice is ready and push it with a clear account of what remains unverified. Never include secrets, credentials, generated build output, or local runtime data, and never force-push. A successful push records progress; it does not make a feature or the whole app complete.
