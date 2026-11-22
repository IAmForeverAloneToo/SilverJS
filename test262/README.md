# Test262

SilverJS is measured against Test262, the ECMAScript conformance suite
(docs/design.md section 1.7). The suite is fetched, never vendored, at
the revision pinned in `src/bin/test262.rs` (`REVISION`):

    git clone --depth 1 https://github.com/tc39/test262 <dir>
    git -C <dir> fetch --depth 1 origin <REVISION>
    git -C <dir> checkout <REVISION>

Then, from the repository's root:

    cargo run --release --bin test262 -- <dir>

J1 runs the parse phase: a test whose front matter expects a parse-time
SyntaxError must fail to parse, and every other test must parse, in
each mode its flags ask for. `expected-parse-failures.txt` lists every
test that does not yet, with a one-word reason; a run that fails a test
not listed, or passes one listed, fails. `--update` rewrites the file,
which only ever shrinks, each test's reason what it fails of then.

From J2.5, `--run` runs the tests: the harness's `assert.js` and
`sta.js`, the test's `includes`, then the test, in a fresh agent for
each mode, with `print` and `$262` (`global`, `evalScript`, `gc`, and
`createRealm`, a new realm of the same agent with its own `$262`) made
through the embedding API (the runner uses nothing private). A
negative runtime test must throw an error of its type; an `async` test
must print its completion. Each run stops at ten seconds through the
agent's interrupt hook.
`expected-run-failures.txt` is this mode's file, with the same rules.
Not applicable, as well as the features the reference browser does not
ship: tail calls, which it does not implement, and the
`error-stack-accessor` proposal, since its `stack` is an own accessor of
each error (docs/chrome-surface.md). `--all` prints every new failure.

`quickjs-ng-passes.txt` is QuickJS-ng's own pass list on the same
revision and measured set (2026-11-20, QuickJS-ng 0.16 through rquickjs
0.14, a harness of silver-work's tooling until the `js` adapter of
J5): one test path a line. It is fixed: J7's bar is that no test in it
fails in SilverJS without a recorded reason (docs/design.md section
1.7).
