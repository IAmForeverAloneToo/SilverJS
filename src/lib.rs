//! SilverJS, the Silver Suite's own JavaScript engine (`docs/design.md`).
//! It depends on nothing outside `std`, and everything browser-shaped is
//! its embedder's.

// No module needs `unsafe` before the replacement (section 1.5). It turns
// to `deny` with the first reviewed module that does, in a commit of its
// own.
#![forbid(unsafe_code)]
// Every index is a checked `get` outside the tests (section 1.5): a
// panic on any input is a bug the fuzzers hunt.
#![cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::unwrap_used))]
