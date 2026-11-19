//! How much of the Rust stack a recursive algorithm has used since it
//! began, so that it can refuse its input before the stack overflows
//! (the design's rule: nothing recurses without a bound). A count of
//! levels alone is no bound: what one level costs differs between debug
//! and release builds, and the thread's stack is the embedder's.
//!
//! The depth is read from a local variable's address, which safe Rust
//! may take (never dereference). Stacks grow down on every platform the
//! engine targets, but the distance is taken either way.

/// The stack a parse may use by default: room for real scripts' nesting
/// on a thread with the smallest common stack (Windows' main thread has
/// 1 MiB), whatever its caller used before.
pub const DEFAULT_STACK_BUDGET: usize = 384 << 10;

/// A guard measuring the stack used from where it was made.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StackGuard {
    base: usize,
    budget: usize,
}

/// The address of a local of a frame just below the caller's.
#[inline(never)]
fn here() -> usize {
    let marker = 0u8;
    std::hint::black_box(std::ptr::addr_of!(marker)) as usize
}

impl StackGuard {
    pub(crate) fn new(budget: usize) -> StackGuard {
        StackGuard {
            base: here(),
            budget,
        }
    }

    /// Whether more than the budget is in use since the guard was made.
    pub(crate) fn exceeded(&self) -> bool {
        here().abs_diff(self.base) > self.budget
    }
}
