# The micro-benches

SilverJS's speed is measured against QuickJS-ng (the lead's request,
2026-11-20) on a set of micro-benches kept with the workspace's tooling
(silverjs-bench): property get and set on one shape and on four, calls,
closures, a tight numeric loop, array push and index, string
concatenation, string branching, and object allocation churn. Each is
run whole on both engines in seven alternating rounds, and the median of
the ratios (SilverJS's time over QuickJS-ng's) is what counts, its
spread shown beside it.

## The method (the lead's, 2026-11-23)

- Both engines are built with the bench harness's `bench-ship` profile:
  one codegen unit, fat LTO, `opt-level = 3`, panics as in release.
  QuickJS-ng's C is built to match: `/O2` (opt-level 3 through `cc`) and
  `/GL`, MSVC's link-time code generation.
- Why: the default release build (16 codegen units, no LTO) places the
  interpreter's code differently with any edit, and that alone moved the
  medians by up to 40% at J3.3b. The numeric loop measured 1.36 before
  that step and 1.51 after it in the default build, but 2.06 and 1.47
  with one codegen unit, its bytecode and the interpreter's frame the
  same in both.
- A line that moves by more than 10% at a step gets an A/B against the
  step's parent, the same build, alternating (tools/abbench.sh).
- A core operation whose median ratio is over 3x is raised with the lead
  before anything is built on it. The line applies to the baseline below
  from J3.3b on.

## The baseline (J3.3b, b25783e, `bench-ship`)

Two runs of seven rounds each (tools/abbench.sh, 2026-11-23); each run's
median ratio is shown. QuickJS-ng's C is built at `/O2` without LTO:
objects compiled with `/GL` do not link once rustc has bundled them into
an rlib, so the C side has no link-time code generation.

| Bench | SilverJS / QuickJS-ng | |
|---|---|---|
| alloc_churn | 1.85, 1.82 | |
| array_push_index | 2.22, 2.27 | |
| calls | 2.23, 2.24 | |
| closures | 2.91, 2.82 | near the line: J6's |
| numeric_loop | 1.56, 1.53 | |
| prop_mono | 2.07, 2.07 | |
| prop_poly | 1.59, 1.61 | |
| string_branch | 56.9, 55.7 | over the line: the known gap, J6's |
| string_concat | 1.27, 1.26 | |

At J3.4a (its tree against this baseline, the same run) no line moved by
more than 10%.
