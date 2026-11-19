# SilverJS

A JavaScript engine written from scratch in Rust, part of the Silver
Suite. It is built to run inside Silver Browser and to be usable on its
own by any other program that needs to run JavaScript.

## Goals

- Correct first: measured against Test262, the official ECMAScript
  conformance suite.
- Safe: the engine is written in safe Rust. Its heap is index-based,
  so the interpreter, the garbage collector and the value
  representation need no `unsafe` code.
- Embeddable: no dependency on Silver Browser. Everything a browser
  needs (DOM bindings, realms per agent, its event loop) lives on the
  embedder's side, behind SilverJS's public embedding API.
- Fast later: a register bytecode interpreter with shapes and inline
  caches first; a baseline JIT after that.

## Status

Early. The design is written; the lexer is the first piece in progress.
Silver Browser runs QuickJS-ng today and moves to SilverJS once SilverJS
handles real sites at least as well.

## Licence

Mozilla Public License 2.0, see `LICENSE`. You may use SilverJS in any
program, open or closed; changes to SilverJS's own files are shared
under the same licence.
