// PDF.js uses Promise.withResolvers in its worker runtime. Safari 15 lacks it,
// so install the focused core-js polyfill in the worker's own global scope.
import "core-js/actual/promise/with-resolvers.js";
import "pdfjs-dist/legacy/build/pdf.worker.mjs";
