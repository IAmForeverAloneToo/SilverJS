//! A small shell over SilverJS, its reference embedder (docs/design.md
//! section 1.10): runs files or `-e` sources in one agent, with a
//! `print` made through the embedding API, and prints what each script
//! threw.
//!
//!     cargo run --example shell -- [-e <source>] [file...]

use silverjs::agent::{Agent, AgentOptions, Outcome, PropertyDescriptor};

fn main() {
    let mut sources = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-e" {
            if let Some(src) = args.next() {
                sources.push(("-e".to_owned(), src));
            }
        } else {
            match std::fs::read_to_string(&arg) {
                Ok(src) => sources.push((arg, src)),
                Err(e) => {
                    eprintln!("shell: {arg}: {e}");
                    std::process::exit(2);
                }
            }
        }
    }
    let mut agent = Agent::new(AgentOptions::default());
    let mut failed = false;
    agent.scope(|scope| {
        if let Ok(global) = scope.global_object()
            && let Ok(print) = scope.native_function("print", 1, |scope, _, args| {
                let mut line = Vec::new();
                for &a in args {
                    line.push(scope.to_text(a)?);
                }
                println!("{}", line.join(" "));
                Ok(scope.undefined())
            })
        {
            let key = scope.key("print");
            scope.define_property(
                global,
                key,
                PropertyDescriptor {
                    value: Some(print),
                    writable: Some(true),
                    enumerable: Some(false),
                    configurable: Some(true),
                    ..PropertyDescriptor::default()
                },
            );
        }
        for (name, src) in &sources {
            scope.nested(|scope| match scope.run_script(src) {
                Ok(Outcome::Value(_)) => {}
                Ok(Outcome::Threw(e)) => {
                    let text = scope.to_text(e).unwrap_or_default();
                    eprintln!("{name}: uncaught {text}");
                    failed = true;
                }
                Ok(Outcome::Terminated) => {
                    eprintln!("{name}: terminated");
                    failed = true;
                }
                Err(e) => {
                    eprintln!("{name}: {e}");
                    failed = true;
                }
            });
        }
    });
    std::process::exit(i32::from(failed));
}
