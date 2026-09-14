# includecost

Find the C/C++ headers whose removal would delete the most preprocessed code, proven by dominators.

## The problem

Large C++ codebases spend most of their compile time re-parsing headers. The usual way to pick which header to cut is to count what it pulls in: include popularity, transitive include size, or `clang -ftime-trace` totals. Those are *inclusive* numbers, and they overstate the gain. If `app.h` includes `log.h`, but half the translation units also include `log.h` directly or through another header, then emptying `app.h` removes nothing of `log.h` in those TUs. Engineers end up spending a refactor on a header that looked expensive and saves little. Tools that get this right, such as include-what-you-use, need a working clang toolchain for the project. This one needs only `compile_commands.json` and the source files.

## How it works

For every translation unit (TU) in `compile_commands.json`:

1. **Scan.** Each file is lexed once. Comments are stripped, with string, character and raw-string literals and line splices respected. The file's *weight* is its significant bytes: every non-blank logical line after comment removal, trimmed, plus its newline. The scan collects `#include "..."` and `#include <...>` directives and marks those inside an `#if`/`#ifdef` block as conditional.
2. **Detect guards.** A file counts as guarded when its first significant line is `#ifndef X` (or `#if !defined(X)`), its second is `#define X`, and the matching `#endif` is its last significant line, with no `#else`/`#elif` at guard level. `#pragma once` also counts. Anything significant outside the guard means the file is *not* guarded.
3. **Resolve.** Quoted includes search the including file's directory, then `-iquote`, `-I`, `-isystem` and `-idirafter` directories, in command-line order. Angled includes skip the first two. Includes that cannot be resolved are reported.
4. **Build the TU include graph.** The TU is the root. A guarded header is expanded at most once per TU, so it is **one node** however many routes reach it. An unguarded header is expanded at every inclusion site, so **each site is its own node**. Conditional includes are assumed taken. With these rules, the code the preprocessor emits is exactly the set of nodes reachable from the root.
5. **Compute dominators.** A node `d` dominates `n` if every path from the root to `n` passes through `d`. The implementation uses the Cooper–Harvey–Kennedy iterative algorithm ("A Simple, Fast Dominance Algorithm", 2001): number nodes in reverse postorder, then repeatedly set each node's immediate dominator to the intersection of its processed predecessors' dominators, walking up the partial tree until nothing changes.
6. **Cost.** Emptying header `h` removes its out-edges. The nodes that become unreachable are exactly the nodes `h` dominates. So `h`'s **exclusive cost** in a TU is the total weight of its dominator subtree, computed for every node in one reverse pass over the reverse postorder. Its **inclusive cost** is the weight of every distinct file reachable from it. The report sums both across TUs, gives their ratio, and lists each header's largest dominator-tree children: the files it is really carrying in.

A worked example is the diamond `a.cpp → b.h → d.h`, `a.cpp → c.h → d.h`:

```
             guarded d.h                    unguarded d.h

   graph:     a.cpp                          a.cpp
             /     \                        /     \
           b.h     c.h                    b.h     c.h
             \     /                       |       |
              d.h                         d.h#1   d.h#2

   dominator  a.cpp ─┬─ b.h                a.cpp ─┬─ b.h ── d.h#1
   tree:             ├─ c.h                       └─ c.h ── d.h#2
                     └─ d.h

   exclusive  b.h = |b|                    b.h = |b| + |d|
              d.h = |d|  (credited to a)   d.h = 2·|d|
   inclusive  b.h = |b| + |d|              b.h = |b| + |d|
```

With a guard, removing `b.h` saves only `b.h` itself, because `c.h` still brings `d.h` in. An inclusive count would claim `|b| + |d|` for it.

## Install and usage

Requires a Rust toolchain (1.74 or newer). There are no runtime dependencies. From a checkout of this repository:

```sh
cargo build --release
./target/release/includecost analyze path/to/compile_commands.json [--top N] [--json] [--jobs N]
```

A small hand-written project ships in `examples/demo`. It has three TUs, a facade header `app/app.h`, a logging header on top of a formatting library, a JSON parser, and a socket header with platform-conditional includes. Its output:

```
$ ./target/release/includecost analyze examples/demo/compile_commands.json
3 translation units, 9 headers, 16.3 KiB preprocessed in total
8 unresolved includes, 2 conditional includes (treated as taken), 0 computed includes (skipped)

rank   exclusive   inclusive   excl%    TUs  header
   1     7.9 KiB     8.3 KiB   94.7%      3  include/util/log.h
                                            carries: third_party/fmt/format.h 5.8 KiB, include/util/strings.h 902 B
   2     6.6 KiB    13.0 KiB   50.4%      2  include/app/app.h
                                            carries: include/json/json.h 5.1 KiB, include/net/socket.h 928 B
   3     5.8 KiB     5.8 KiB  100.0%      3  third_party/fmt/format.h
   4     5.1 KiB     5.1 KiB  100.0%      2  include/json/json.h
                                            carries: include/json/detail/parser.h 4.7 KiB
   5     4.7 KiB     4.7 KiB  100.0%      2  include/json/detail/parser.h
   6     1.8 KiB     7.4 KiB   24.7%      2  include/net/socket.h
                                            carries: include/net/platform_win.h 516 B, include/net/platform_posix.h 382 B
   7     1.3 KiB     1.3 KiB  100.0%      3  include/util/strings.h
   8       516 B       516 B  100.0%      2  include/net/platform_win.h  [conditional]
   9       382 B       382 B  100.0%      2  include/net/platform_posix.h  [conditional]

unresolved includes:
  third_party/fmt/format.h:5: <cstdio>
  third_party/fmt/format.h:6: <string>
  include/util/strings.h:4: <string>
  include/json/detail/parser.h:4: <map>
  include/json/detail/parser.h:5: <memory>
  include/json/detail/parser.h:6: <string>
  include/json/detail/parser.h:7: <vector>
  src/server.cpp:4: <vector>
```

How to read it:

- By inclusive bytes, `app/app.h` is the most expensive header at 13.0 KiB. But `main.cpp` includes `util/log.h` itself and `server.cpp` includes `net/socket.h` itself, so emptying `app.h` saves only 6.6 KiB. Almost all of that is the JSON parser, which the `carries` line names.
- `net/socket.h` reaches 7.4 KiB, but only a quarter of that is its own. Cutting it would be a poor use of a refactor.
- `util/log.h` keeps 95% of its inclusive cost, because nothing else pulls in the formatting library. It is the best target.
- Standard-library headers show up as unresolved: compiler built-in include directories are not searched (see Limitations).

`--json` prints the same data with exact byte counts, plus `ratio`, `tus`, `expansions`, `guard`, `conditional` and `carries` per header, and the full list of unresolved includes. `--top 0` shows every header.

To try it on a larger synthetic project:

```sh
./target/release/includecost generate /tmp/synth --headers 2000 --tus 500 --seed 7
./target/release/includecost analyze /tmp/synth/compile_commands.json --top 10
```

Run the tests with `cargo test`.

## Results

### Correctness

Each claim has its own test, and `cargo test` runs them all:

| Claim | Test |
|---|---|
| Immediate dominators equal the brute-force definition (`d` dominates `n` iff deleting `d` makes `n` unreachable) on 2,000 seeded random graphs, half DAGs and half cyclic, every node reachable from the root | `tests/dominators.rs` |
| For **every node** of every TU in 12 generated projects (with unguarded headers, `#pragma once`, conditional and missing includes, and guarded cycles), dominator-subtree weight equals the byte difference from re-preprocessing the TU with that node emptied. The re-traversal is an independent sequential walk that shares no code with the graph or dominator code | `tests/cost_claim.rs` |
| Guarded diamond counts `D` once and credits it to `A`; unguarded `D` counts twice, one copy under each of `B` and `C` | `tests/semantics.rs` |
| Partial guards (code before or after the block, an include after it, `#else`, wrong macro, missing `#define`, two sibling blocks) are not guards, and a partially guarded `D` in the diamond is expanded twice | `tests/semantics.rs` |
| Quoted search order is includer dir, `-iquote`, `-I`, `-isystem`, `-idirafter`; angled skips the first two | `tests/resolution.rs` |
| Missing headers, missing TU sources, computed includes and malformed databases are reported, never a panic | `tests/resolution.rs`, `tests/cli.rs` |
| Guarded and `#pragma once` include cycles terminate with exact costs; cycles of unguarded headers are cut and reported | `tests/semantics.rs` |

### Speed

```sh
cargo run --release --example benchmark
```

The benchmark generates projects with `GenConfig::benchmark` (seed 2026) and builds every TU graph once. On those same graphs it times two things single-threaded: the dominator method, and brute-force per-node removal (delete each node, BFS from the root, measure lost weight: O(n·(n+e)) per TU). It asserts that both give identical per-node exclusive costs on every TU. The last column is the whole `analyze` pipeline, including graph building and inclusive closure, on all cores. Measured on an Apple Silicon (arm64) Mac with 8 hardware threads, macOS 26, Rust 1.94, release build:

| headers | TUs | include edges | nodes/TU | edges/TU | dominators (1 thread) | brute force (1 thread) | TUs brute-forced | speedup per TU | full `analyze` (8 threads) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 250 | 100 | 2914 | 285 | 2311 | 12.5 ms | 97.1 ms | 100 | 8x | 3.5 ms |
| 1000 | 400 | 11455 | 478 | 3967 | 27.9 ms | 1097.7 ms | 400 | 39x | 19.0 ms |
| 2500 | 1000 | 28799 | 715 | 5948 | 97.7 ms | 5849.2 ms | 1000 | 60x | 67.4 ms |
| 5000 | 2000 | 57314 | 929 | 7812 | 259.3 ms | 19764.2 ms | 2000 | 76x | 193.0 ms |

Outputs were identical on all 3,500 brute-forced TUs.

These numbers are below the "several orders of magnitude" I expected when I started. The reason is the generator. Headers sit in a tree of modules, so a TU reaches about 930 of the 5,000 headers rather than all of them. Brute force costs about n times as much as one dominator pass, so the speedup grows linearly with nodes per TU, as the table shows. A TU that reaches 10,000 headers would see roughly ten times the gap measured here. That is an extrapolation, not a measurement. The benchmark takes about 30 seconds; `--quick` runs only the two smallest rows.

## Design notes

**Guarded headers as one node, unguarded headers as one node per site.** Merging every file into one node would make the graph small, but the costs would be wrong for any header expanded more than once. Giving every inclusion site its own node would be exact, but the graph would blow up and the per-file answer would split across copies. Guards are what actually collapse repeated expansions in the preprocessor, so the graph collapses exactly where the preprocessor does. The payoff is that "reachable from the root" equals "emitted by the preprocessor". Dominator subtrees are then exact removal costs, not an approximation, which is why the tests can check equality on every node instead of a tolerance. The cost is an asymmetry for unguarded headers included from several sites. Their per-file number is the sum of their non-nested per-site costs, and it can under-count emptying all sites at once: code reachable only through two different sites of the same unguarded file is dominated by neither site. The test suite asserts this bound rather than hiding it. In practice unguarded headers are mostly `.def`/`.inc` leaf files, where the sum is exact.

**No clang, no preprocessor.** Evaluating `#if` would require the exact macro environment of each TU: compiler built-ins, `-D` flags, and target headers. That is precisely the toolchain dependency this tool avoids. Treating every conditional include as taken over-approximates, and it errs in a known direction: a header behind `#ifdef _WIN32` is counted on Linux builds too. The report marks such headers `[conditional]`, so the over-approximation is visible, not silent. Inclusive bytes come from one file-level graph per distinct set of search paths. Strongly connected components are condensed with Tarjan's algorithm and the components get reachability bitsets, so the "inclusive" column does not reintroduce the O(n·(n+e)) cost the dominators avoid.

## Limitations

- **No macro evaluation.** `#if`/`#ifdef` branches are all assumed taken, both sides of `#else` included. `#include MACRO` forms are counted and skipped. `#include_next` and `#import` are ignored.
- **No compiler built-in include paths.** Standard-library and SDK headers resolve only if their directories appear as `-I`/`-isystem` in the database. Otherwise they are listed as unresolved and contribute no bytes. Forced includes (`-include file`) and MSVC `/I` flags are not read.
- **Bytes, not time.** Significant bytes are a proxy for parse cost. A header full of templates costs more per byte than one full of declarations.
- **Guard identity is the file.** Two different files that share a guard macro are treated as independent, whereas the real preprocessor would skip the second. A guard must be `#ifndef X` followed directly by `#define X`. Guards written in other shapes are treated as unguarded, which over-counts those headers.
- **Unguarded headers included from several sites** get a per-file exclusive cost that can under-count emptying all sites at once (see Design notes). Per-site costs are always exact. Cycles made only of unguarded headers are cut at the repeat and reported.
- **Paths are normalized lexically.** A header reached through a symlink and through its real path counts as two files.
- **Inclusive closure memory** is quadratic in the number of distinct included files per search-path configuration (about 3 MB for 5,000 headers). Very large databases with many distinct `-I` sets will use more.
- A single TU graph is capped at 4 million nodes to bound exponential expansion of unguarded headers. Truncated TUs are reported.

## License

MIT. See [LICENSE](LICENSE).
