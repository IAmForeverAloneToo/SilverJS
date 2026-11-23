//! Async functions (ECMA-262 section 27.7) and the jobs a script leaves:
//! an async function's frame is suspended at `await` into its driver (an
//! object as a generator is) and resumed by the reaction job of the
//! promise it awaits, on a loop of its own.

use super::{Abrupt, Completion};
use crate::agent::Agent;

impl Agent {
    /// What a script's run completes with, once the jobs it queued have
    /// run (its value or throw kept; a termination in a job ends it).
    pub(crate) fn after_script(&mut self, completion: Completion) -> Completion {
        let held = match completion {
            Completion::Normal(v) | Completion::Threw(v) => v,
            Completion::Terminated => return completion,
        };
        self.vm.temps.push(held);
        let r = self.run_jobs();
        self.vm.temps.pop();
        match r {
            Err(Abrupt::Terminate) => Completion::Terminated,
            _ => completion,
        }
    }
}
