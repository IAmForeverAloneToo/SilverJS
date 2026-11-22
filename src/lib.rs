//! SilverJS, the Silver Suite's own JavaScript engine (`docs/design.md`).
//! It depends on nothing outside `std`, and everything browser-shaped is
//! its embedder's.
//!
//! J1: the lexer (ECMA-262 section 12), the parser and its early errors
//! (sections 13 to 17), and RegExp's pattern syntax. J2 so far: values,
//! the heap and its collector, handles, strings, symbols and objects,
//! the compiler to register bytecode, its interpreter, the core
//! built-ins, and the embedder's natives.

// No module needs `unsafe` before the replacement (section 1.5). It turns
// to `deny` with the first reviewed module that does, in a commit of its
// own.
#![forbid(unsafe_code)]
// Every index is a checked `get` outside the tests (section 1.5): a
// panic on any input is a bug the fuzzers hunt.
#![cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::unwrap_used))]

pub mod agent;
pub mod ast;
mod builtins;
pub mod compiler;
mod embed;
mod heap;
mod internal;
mod interp;
pub mod lexer;
mod number;
mod object;
pub mod parser;
mod regexp;
mod stack;
mod string;
mod unicode;
mod unicode_tables;
pub mod value;

#[cfg(test)]
mod builtins_tests;
#[cfg(test)]
mod compiler_tests;
#[cfg(test)]
mod generator_tests;
#[cfg(test)]
mod heap_tests;
#[cfg(test)]
mod interp_tests;
#[cfg(test)]
mod iteration_tests;
#[cfg(test)]
mod lexer_tests;
#[cfg(test)]
mod object_tests;
#[cfg(test)]
mod parser_tests;
#[cfg(test)]
mod realm_tests;
#[cfg(test)]
mod stack_tests;

pub use embed::{ErrorKind, HostFn, JsResult, RealmId, Thrown};
pub use heap::HeapStats;
pub use string::{Atom, PropertyKey};
pub use unicode_tables::UNICODE_VERSION;

/// An early error (ECMA-262 section 17): a source that does not parse,
/// its message worded as the reference browser words it, and the bytes
/// of the source it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    /// Byte offsets into the source, `start..end`.
    pub start: u32,
    pub end: u32,
}

impl SyntaxError {
    pub(crate) fn new(message: impl Into<String>, start: usize, end: usize) -> SyntaxError {
        SyntaxError {
            message: message.into(),
            start: clamp(start),
            end: clamp(end),
        }
    }
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SyntaxError: {}", self.message)
    }
}

impl std::error::Error for SyntaxError {}

/// A byte offset as the `u32` positions are kept in; a source is never
/// longer ([`lexer::MAX_SOURCE_BYTES`]).
pub(crate) fn clamp(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}
