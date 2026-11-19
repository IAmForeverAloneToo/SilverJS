//! Any bytes, as a script and as a module: the parser returns a program
//! or a SyntaxError whose span lies in the source, never panics, never
//! overflows the stack, and always returns (libFuzzer's timeout holds it
//! to that).

#![no_main]

use libfuzzer_sys::fuzz_target;
use silverjs::parser::{Goal, parse};

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    for goal in [Goal::Script, Goal::Module] {
        match parse(&source, goal) {
            Ok(program) => {
                assert!(program.span.end as usize <= source.len());
            }
            Err(error) => {
                assert!(error.start <= error.end, "{error:?}");
                assert!(error.end as usize <= source.len(), "{error:?}");
                assert!(!error.message.is_empty());
            }
        }
    }
});
