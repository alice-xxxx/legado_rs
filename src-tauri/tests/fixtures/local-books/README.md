# Local book end-to-end fixtures

These files are small checked-in inputs for local-import and browser resource tests.

- `two-page-text.pdf`: valid, searchable two-page text PDF, title `Legado Local PDF Fixture`.
- `two-page-encrypted.pdf`: same two pages, protected with user password `fixture-pass` (owner password `fixture-owner`). The original encrypted bytes should remain the PDF asset.
- `three-page-comic.cbz`: three valid 640×900 PNG pages, archive order is intentionally `page10`, `page2`, `page1`; expected reading order is `page1`, `page2`, `page10`. Their outer page colors are red, green, and blue.
- `page1.png`, `page2.png`, `page10.png`: standalone matching pages, also usable by browser image pixel checks.
- `custom-toc.txt`: uses `VOLUME N: Title` headings; the import test saves a matching Rust-owned TOC rule and verifies import after reopening the resource store.
- `toc-override.txt`: uses `SCENE N / Title` headings; the import test verifies a one-time `tocRegex` takes precedence over saved rules.

PDF chapter HTML contract: `section[data-legado-document="pdf-page"]`, zero-based `data-page-index`, `data-default-zoom="page-fit"`, with `a[data-legado-pdf-src]` pointing to the same book's relative PDF asset. PDF requests should use the resource HTTP service (including Range); the document bytes are not sent over IPC or JSON.
