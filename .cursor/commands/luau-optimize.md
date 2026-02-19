# /luau-optimize - Roblox Luau Performance Optimization

Optimize Roblox Luau files for maximum performance, type safety, and native codegen quality using `luau-compile`, `luau-analyze`, `luau-ast`, and the `luau` CLI.

All scripts are **Roblox Luau**. Roblox APIs are always available. The vector type is `Vector3` (Roblox userdata). We are NOT using the new type solver. Studio testing uses `--!optimize 1` by default; live experiences use `--!optimize 2`. All `luau-compile` invocations must include `--vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3`. Prefer `.luau` extension over `.lua` everywhere.

**Workspace hygiene:** CLI tools generate residual files (`stats.json`, `profile.out`, `coverage.out`, `trace.json`). At the start of execution, create a temp working directory and use it for all tool output:

```
# Windows
$LUAU_TMP = New-Item -ItemType Directory -Path "$env:TEMP\luau-optimize-$(Get-Random)"
# Linux/Mac
LUAU_TMP=$(mktemp -d)
```

Route all output there: `--stats-file=$LUAU_TMP\stats.json`, write benchmark files to `$LUAU_TMP\bench.luau`, run profiling/coverage tools with `$LUAU_TMP` as working directory. The OS handles cleanup -- no manual deletion needed.

---

## Phase 0 -- Scope Resolution and Configuration

### Resolve target

- If user specifies a file: optimize that file.
- If user specifies a directory: discover all `.luau`/`.lua` files, enter Phase 6 (multi-file mode).
- If no argument: optimize the currently open file.

### Infer script context

Check in this order:

1. **Filename suffix** (highest priority):
   - `*.server.luau` / `*.server.lua` -> **server** (`--!native` is high-value)
   - `*.client.luau` / `*.client.lua` -> **client** (`--!native` less beneficial due to device diversity)
   - `*.legacy.luau` / `*.legacy.lua` -> **ambiguous** (could be cloned/moved at runtime). Ask the user.
   - `*.luau` / `*.lua` (no suffix) -> ModuleScript, fall through to path check.
2. **Path-based inference** (for ModuleScripts and ambiguous cases):
   - `ServerScriptService`/`ServerStorage` in path -> server-side module
   - `StarterPlayer`/`StarterGui`/`StarterCharacterScripts`/`ReplicatedFirst` in path -> client-side module
   - `ReplicatedStorage`/`shared` in path -> shared module (conservative `--!native`)
   - Ambiguous or outside Roblox project -> ask the user

### Quiz the user

Use the AskQuestion tool with these three questions:

**Question 1: "Compilation optimization intensity"**
- `minimal` -- Headers (`--!strict`/`--!native`/`--!optimize 2`), type annotations on all function signatures, deprecated pattern replacements only
- `moderate` -- + function restructuring for inlining, import hoisting, compound operators, fastcall enablement, allocation reduction
- `insane` -- + every micro-optimization from the pattern catalog, full bytecode analysis, register pressure optimization, closure caching analysis

**Question 2: "Algorithm optimization"**
- `none` -- Don't touch logic or data flow
- `low hanging fruit` -- Obvious O(n^2)->dict, redundant iterations, missing caches
- `moderate` -- + data structure changes, caching strategies, event-driven refactors, loop fusion
- `insane` -- + full algorithmic redesign where beneficial, dynamic programming, architectural restructuring

**Question 3: "Benchmarking"**
- `yes` -- Benchmark all non-trivial changes with `luau` CLI (when function is standalone)
- `no` -- Trust bytecode/codegen metrics only
- `+EV only` -- Benchmark only when algorithmic changes or ambiguous bytecode results warrant it

### Derived behavior

| Setting | minimal | moderate | insane |
|---|---|---|---|
| Headers | Always | Always | Always |
| Type annotations | Function signatures | + hot path locals | + all locals |
| Pattern replacements | Deprecated only | Full Priority 2 | + Priority 4 |
| Function restructuring | No | Yes (Priority 3) | Aggressive splitting |
| Bytecode verification | After all changes | After each priority | After each change |
| Algorithmic changes | Per algo quiz | Per algo quiz | Per algo quiz |
| `@native` vs `--!native` | `--!native` unless big file | Selective `@native` on hot functions | Full per-function analysis |

**Restructuring scope** scales with compilation intensity:
- **minimal**: No restructuring. Only additive changes (headers, annotations, pattern swaps).
- **moderate**: Break up monoliths, reorder locals for inlining, hoist imports. Preserve exports/API.
- **insane**: Aggressive restructuring permitted. May change internal function boundaries, rewrite data flow.

---

## Phase 1 -- Deep Code Read and Algorithmic Analysis

This phase comes FIRST. Algorithmic improvements produce order-of-magnitude speedups that dwarf any bytecode optimization. This is also the largest code change, so it must happen before any bytecode-level work.

Read the entire source file thoroughly before touching any CLI tools.

### Step 1 -- Structural decomposition

Monolithic functions hide their algorithmic structure behind interleaved concerns. Before analyzing complexity, understand what the code is *actually doing*:

- **Identify responsibilities**: A 200-line function usually contains 3-5 logical steps that should be separate functions.
- **Trace data flow**: What goes in, what comes out, what is mutated along the way.
- **Separate concerns**: Validation, transformation, I/O, state management, error handling are often tangled. Separating them reveals the core algorithm.
- **Name the steps**: If you can't name what a code block does in 3-5 words, it's doing too much.
- **Look for hidden abstractions**: Repeated "iterate, filter, transform, collect" patterns that could be a single helper.

When the quiz allows restructuring (moderate/insane):
- Physically decompose monoliths into focused local functions. Each does one thing.
- This decomposition often *reveals* optimization opportunities invisible in the monolith: step 2 recomputes what step 1 already knew, or an inner loop could be a lookup table built in the outer loop.
- Smaller functions are easier for the compiler to inline at `-O2` -- clearer code AND better bytecode.
- Preserve the original function as a thin orchestrator calling the decomposed pieces.

Even at "minimal" intensity, still perform this mental decomposition to inform the analysis below.

### Step 2 -- Algorithmic analysis

With the structure understood (or cleaned up), analyze:

**Complexity analysis** -- Identify time complexity of every significant function:
- O(n^2) or worse: nested loops, repeated linear searches, quadratic string building
- O(n) where O(1) is possible: linear search replaceable with dictionary/set lookup
- Repeated work: same computation done multiple times, cacheable/memoizable
- Unnecessary copies: cloning data that could be referenced or mutated in place
- Hidden quadratics: O(n) function that calls another O(n) function inside a loop

**Data structure fitness** -- Is the right data structure being used?
- Array for membership testing -> set (dictionary with `true` values)
- Linear scan for lookup by key -> dictionary
- Repeated sort of mostly-sorted data -> maintain sorted order on insert
- Flat list with frequent removal from middle -> swap-and-pop
- Large string built incrementally -> `table.concat` pattern or `buffer`
- Multiple parallel arrays -> single array of structs (table of tables)
- Unbounded growth without cleanup -> size limits, LRU eviction, periodic pruning

**Caching and memoization:**
- Pure functions called repeatedly with same args -> memoize
- Expensive property reads in loops -> cache in local before loop
- Computed values unchanged within a scope -> hoist out
- Derived state recomputed from scratch on every access -> maintain incrementally

**Redundant work elimination:**
- Multiple passes over same data -> fuse into one
- Recomputing derived state -> maintain incrementally
- Sort/filter that could be done once and reused
- Deep clone where shallow clone or reference suffices
- Intermediate result immediately discarded after extracting one field

**Architectural patterns:**
- Per-frame polling where event-driven would work (RunService events are expensive)
- Synchronous work that could be chunked across frames with `task.wait()`
- Undisconnected event connections (memory leak + wasted computation)
- Rebuilding entire state on small change -> incremental update
- God-object owning too much state -> split responsibilities

**Hot path identification:**
- Event handlers on RunService (PreAnimation, PreRender, PreSimulation, PostSimulation, Heartbeat)
- Functions called inside loops
- Recursive functions
- Functions called from other hot functions

### Step 3 -- Present findings and quiz the user

Before applying any changes, present a summary of all findings organized by impact. For each finding:

- **What**: One-line description of the issue
- **Where**: Function name / line range
- **Why**: What makes it slow (e.g., "O(n^2) nested scan on every frame")
- **Fix**: Proposed change in plain language
- **Impact**: Estimated improvement (order-of-magnitude, constant-factor, or clarity-only)

Then quiz the user using the AskQuestion tool:

- For fewer than 5 findings: one question per finding with options `apply` / `skip` / `modify`
- For 5+ findings: group by category with options `apply all` / `cherry-pick` / `skip all`:
  - Structural decomposition
  - Data structure changes
  - Algorithm changes
  - Architectural changes

If the algorithm quiz from Phase 0 was "none", present findings as informational only (no apply options). The user still sees what was found.

If the user selects "modify", wait for their input before proceeding with that specific change.

### Step 4 -- Apply approved changes

Apply only user-approved changes, in this order (dependencies flow downward):

1. **Structural decomposition** -- break monoliths into focused functions (foundation for everything else)
2. **Data structure changes** -- swap arrays for dicts, add indexes, etc.
3. **Algorithm changes** -- replace O(n^2) with O(n), add caching/memoization
4. **Architectural changes** -- event-driven refactors, incremental updates

Benchmark algorithmic changes if the benchmarking quiz warrants it (see Phase 4.5).

---

## Phase 2 -- Baseline Capture

Run all diagnostic tools on the code (post-algorithmic changes if any were applied in Phase 1). These metrics are the baseline for bytecode optimization phases.

### Tools to run

Detect OS: use `--target=x64_ms` on Windows, `--target=x64` on Linux/Mac. Route stats output to temp directory.

```
luau-analyze --mode=strict <file>
luau-analyze --annotate <file>
luau-compile --remarks -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
luau-compile --text -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
luau-compile --codegen --target=x64_ms --record-stats=function --stats-file=$LUAU_TMP\stats.json -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
```

### Metrics to capture

- Total bytecode instruction count (from `--text`)
- Allocation count: NEWTABLE, NEWCLOSURE occurrences (from `--remarks`)
- Inlining success/failure count (from `--remarks`: "inlining succeeded"/"inlining failed")
- Type coverage: count of `any` types in `--annotate` output
- Register spills and skipped functions (from `--record-stats` JSON `lowerStats`)
- Lint warning count (from `--mode=strict`)

---

## Phase 3 -- Compilation-Level Code Analysis

Using both the source (already read in Phase 1) and tool output from Phase 2, identify bytecode-level optimization opportunities:

- **Missing types**: `any` inferences hurting native codegen (especially `Vector3`, `CFrame`, `buffer` params). JIT uses annotations directly -- no runtime type analysis. Unannotated params assumed to be tables.
- **Import patterns**: `math.max` resolved at load time via GETIMPORT. Broken by `getfenv`/`setfenv`/`loadstring` (marks env "impure").
- **Allocation in loops**: NEWTABLE, NEWCLOSURE in hot loops. High allocation rate = more GC assist work.
- **Closure caching**: Repeated function expressions cached when: no upvalues, or all upvalues immutable and module-scope. Mutable captures prevent caching.
- **Upvalue mutability**: ~90% immutable in typical code. Immutable = no allocation, no closing, faster access. Mutable = extra object.
- **Method call patterns**: `obj:Method()` uses fast method call instruction. Avoid `obj.Method(obj)`. `__index` should point at a table directly (not function or deep chain).
- **Inline caching**: Best when field name known at compile time, no metatables, uniform shapes.
- **pcall in hot paths**: Prevents native codegen optimization.
- **String concatenation**: `..` in loops -> `table.concat` or string interpolation.
- **Deoptimizing APIs**: `getfenv`/`setfenv` (even read-only!), `loadstring` disable ALL import optimization and fastcalls.
- **Builtin global writes**: `math = ...` disables fastcall. Lint `BuiltinGlobalWrite` catches this.
- **Metamethod cost**: `__eq` always called on `==`/`~=` even for rawequal values.
- **GC pressure**: Incremental GC with "GC assists" -- allocating code pays proportional GC work.

---

## Phase 4 -- Compilation Optimization

Apply changes in priority order. Re-run `luau-compile --remarks -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>` after structural changes to verify the compiler benefits.

### Priority 1 -- Headers and type safety (always apply)

- Add `--!strict` / `--!native` / `--!optimize 2` header. (`--!optimize 2` is default in live but not Studio testing.)
- Fix type errors from `luau-analyze`.
- Add missing type annotations where `--annotate` shows `any`. Focus on function signatures first, then locals in hot paths per intensity level.
- Annotate `Vector3`, `CFrame`, `buffer` params explicitly -- native codegen generates specialized vector code.
- Replace `getfenv`/`setfenv` -- disables builtins, imports, and optimizations globally.
- Replace `table.getn` -> `#t`, `table.foreach`/`table.foreachi` -> `for..in`.

### Priority 2 -- Low-hanging structural wins

- Hoist library functions: `local floor = math.floor` enables GETIMPORT fastcall. Fastcall builtins: `assert`, `type`, `typeof`, `rawget`/`rawset`/`rawequal`, `getmetatable`/`setmetatable`, `tonumber`/`tostring`, most `math.*` (not `noise`, `random`/`randomseed`), `bit32.*`, some `string.*`/`table.*`. Partial specializations: `assert` (unused return + truthy), `bit32.extract` (constant field/width), `select(n, ...)` O(1).
- Replace `pairs(t)`/`ipairs(t)` -> `for k, v in t do`. Generalized iteration skips the `pairs()` call. `for i=1,#t` is slightly slower.
- `math.floor(a / b)` -> `a // b` (dedicated VM opcode, `//=` compound form).
- Use compound assignment (`+=`, `-=`, `*=`, `//=`, `..=`) -- LHS evaluated once.
- String concat in loops -> `table.concat` or backtick interpolation (lowers to optimized `string.format`).
- `table.create(n)` for known-size arrays. Sequential fill: `local t = table.create(N); for i=1,N do t[i] = ... end`.
- `table.insert(t, v)` for unknown-size append -- `#t` is O(1) cached, worst case O(log N).
- `a + (b - a) * t` -> `math.lerp(a, b, t)` (exact at endpoints, monotonic).
- Manual byte swap -> `bit32.byteswap(n)` (CPU bswap).
- Manual log2 -> `bit32.countlz(n)` (CPU instruction, ~8x faster).
- Manual linear search -> `table.find(t, v)`.
- Manual `pairs` clone -> `table.clone(t)`.
- Manual `string.byte`/`string.char` -> `string.pack`/`string.unpack`.
- `rawlen(t)` when metamethods not needed.
- Explicit `./`/`../`/`@` prefixes in `require()`.

### Priority 3 -- Function structure for inlining

- Break monoliths into small local functions (compiler inlines at `-O2`).
- Inlining requirements: local, non-mutated, non-recursive, not OOP (`:` syntax), not metamethod. Disabled by `getfenv`/`setfenv`.
- Inlining + constant folding cascade: `local function double(x) return x*2 end; local y = double(5)` folds to `y = 10`.
- Create local function wrappers for frequently-called imported module methods.
- Move `pcall` out of hot loops.
- Use `obj:Method()` not `obj.Method(obj)` -- fast method call instruction.
- `__index` should point at a table directly for inline caching.
- Reduce mutable upvalue captures in loop closures. Immutable upvalues = no allocation, no closing.
- Set all table fields at once (compiler infers hash capacity from subsequent assignments).
- Loop unrolling: constant bounds only, simple body.
- Native codegen size limits: 64K instructions/block, 32K blocks/function, 1M instructions/module. Split if exceeded.

### Priority 4 -- Micro-optimizations (hot paths only)

- Table shape consistency (same fields, same order) -- inline caching predicts hash slots.
- `buffer` for binary data (fixed-size, offset-based, efficient native lowering).
- `buffer.readu32` + `bit32` over `buffer.readbits` when schema known.
- Strength reduction: `* 2^n` -> `bit32.lshift`, `/ 2^n` -> `bit32.rshift`.
- Minimize `tostring`/`tonumber` in tight loops.
- `table.freeze` for readonly config (avoids proxy `__index` overhead).
- Keep `__eq` cheap -- fires on every `==`/`~=` and `table.find`.
- `math.isfinite`/`math.isnan`/`math.isinf` over `x ~= x`.
- Annotate `Vector3` params for native specialization.
- `if expr then A else B` over `cond and A or B` (one branch, falsy-safe).

---

## Phase 4.5 -- Benchmark with `luau` CLI

When a non-trivial optimization is applied to an isolated, pure-logic function (no Roblox API dependencies), write a small benchmark harness:

```luau
local function original(...)
    -- paste original implementation
end

local function optimized(...)
    -- paste optimized implementation
end

local ITERATIONS = 1_000_000
local clock = os.clock

for _ = 1, 1000 do original(...) end
for _ = 1, 1000 do optimized(...) end

local t0 = clock()
for _ = 1, ITERATIONS do original(...) end
local t1 = clock()
for _ = 1, ITERATIONS do optimized(...) end
local t2 = clock()

print(`Original:  {t1 - t0:.4f}s`)
print(`Optimized: {t2 - t1:.4f}s`)
print(`Speedup:   {(t1 - t0) / (t2 - t1):.2f}x`)
```

Run with both paths:

```
luau -O2 bench.luau
luau -O2 --codegen bench.luau
```

**When to benchmark:** Self-contained function, algorithmic/data structure change, ambiguous bytecode diff, tiebreak between approaches.

**When NOT to benchmark:** Roblox API dependencies (use Studio Script Profiler instead), purely additive changes, cold code.

Write the benchmark file to `$LUAU_TMP\bench.luau`, not the project directory. Run `luau` from `$LUAU_TMP` as working directory so `profile.out` and other residuals land there too.

---

## Phase 5 -- Verification

Re-run all Phase 2 tools and present a before/after comparison:

- Bytecode instruction count delta
- Allocation count delta
- Inlining success rate delta
- Type coverage improvement (fewer `any` types)
- Register spill delta
- Lint warning delta
- Benchmark results (if Phase 4.5 was used)

If any metric regressed, investigate and explain why (or revert).

---

## Phase 6 -- Multi-file Mode

When optimizing a directory:

1. Run Phase 1 (code read) on all files to identify algorithmic improvement potential.
2. Run Phase 2 (baseline) on all files to identify bytecode optimization potential.
3. Prioritize by: worst algorithmic complexity, most lint warnings, most `any` types, most allocations, most failed inlines.
4. Process files one at a time through Phases 1-5.
5. Present aggregate metrics at the end.

---

## Key Principles

These principles govern every optimization decision:

- **Same implementation, better performance** -- behavioral equivalence is non-negotiable. Never change what the code does, only how fast it does it.
- **Algorithms first, bytecode second** -- an O(n) algorithm with unoptimized bytecode beats an O(n^2) algorithm with perfect bytecode every time.
- **Low-hanging fruit first** -- headers and types before restructuring, restructuring before micro-optimizations.
- **Compiler feedback loop** -- verify each structural change with `luau-compile --remarks`. If the compiler didn't benefit, the change wasn't worth it.
- **Small functions over monoliths** -- easier for the compiler to inline, easier for humans to maintain, easier to reason about algorithmically.
- **Local functions for inlining** -- even at the cost of duplicating imported behavior in high-traffic code.
- **Quantify everything** -- before/after metrics for every optimization pass. No "I think this is faster" -- prove it.
- **Type annotations drive native codegen quality** -- every `any` type is a missed specialization opportunity. The JIT uses annotations directly with no runtime analysis.
- **Event-driven over polling** -- avoid per-frame calculations when events suffice.

---

## Reference: Type System (Old Typechecker Only)

**Available NOW:**
- `--!strict` / `--!nonstrict` / `--!nocheck`
- Basic annotations: `local x: number`, `function f(a: string): boolean`
- Optional: `string?`, Union: `number | string`, Intersection: `T1 & T2`
- Generics: `function id<T>(x: T): T`, explicit instantiation: `f<<number>>()`
- Cast: `x :: string`
- Array: `{number}`, Dict: `{[string]: number}`, Table: `type T = { field: type }`
- `typeof()`, `export type`, typed variadics, alias defaults, singletons
- `never`, `unknown`, `read`/`write` modifiers
- Type refinements: `type()`, `typeof()`, `:IsA()`, truthiness, equality, `assert()`

**DO NOT recommend (requires new typechecker):**
- `keyof<T>`, `index<T,K>`, `rawkeyof<T>`, `rawget<T,K>`
- `getmetatable<T>`, `setmetatable<T,M>`
- User-defined type functions, negation types (`~T`)
- Relaxed recursive type restrictions

## Reference: Type Refinement Patterns

The compiler narrows types after certain checks, improving both type safety and native codegen. Use refinements to provide type information without explicit annotations where the type is known from a runtime check:

- **Truthiness:** `if x then` narrows `x` from falsy (`nil`/`false`)
- **Type guards:** `if type(x) == "number" then` narrows `x` to `number`
- **Typeof guards (Roblox):** `if typeof(x) == "Vector3" then` narrows to `Vector3`; `x:IsA("TextLabel")` narrows `Instance` to subclass
- **Equality:** `if x == "hello" then` narrows `x` to singleton `"hello"`
- **Assert:** `assert(type(x) == "string")` narrows `x` to `string` after the call
- **Composition:** Supports `and`, `or`, `not` for compound refinements

## Reference: Deprecated Patterns

| Deprecated | Replacement | Impact |
|---|---|---|
| `getfenv()` / `setfenv()` | `debug.info(i, "snl")` | Disables ALL optimizations. Even read-only `getfenv()` deoptimizes. |
| `loadstring()` | Restructure with `require` | Marks env "impure", disables imports |
| `table.getn(t)` | `#t` | Slower |
| `table.foreach(t, f)` | `for k, v in t do` | Slower |
| `table.foreachi(t, f)` | `for i, v in t do` | Slower |
| `wait()` | `task.wait()` | Deprecated Roblox API |
| `obj.Method(obj, ...)` | `obj:Method(...)` | Misses fast method call instruction |
| `string:method()` | `string.method(s)` | Fastcall: `string.byte(s)` faster than `s:byte()` |

## Reference: Lint Rules

| Lint | Name | Optimization impact |
|---|---|---|
| 10 | BuiltinGlobalWrite | Overwriting builtins disables fastcall |
| 22 | DeprecatedApi | Deprecated APIs: perf/correctness issues |
| 23 | TableOperations | Wrong index, redundant insert, `#` on non-arrays |
| 3 | GlobalUsedAsLocal | Global in one function -> should be local |
| 7 | LocalUnused | Dead locals |
| 12 | UnreachableCode | Dead code |
| 25 | MisleadingAndOr | `a and false or c` bugs |

## Reference: Bytecode Pattern Catalog

| Slow | Fast | Why |
|---|---|---|
| `math.floor(a / b)` | `a // b` | Dedicated VM opcode |
| `data[i].x = data[i].x + 1` | `data[i].x += 1` | LHS evaluated once |
| `a + (b - a) * t` | `math.lerp(a, b, t)` | Exact at endpoints, monotonic |
| `"pre" .. v .. "suf"` | `` `pre{v}suf` `` | Optimized `string.format` |
| Manual byte swap | `bit32.byteswap(n)` | CPU bswap |
| Manual log2 loop | `bit32.countlz(n)` | CPU instruction, ~8x faster |
| `cond and A or B` | `if cond then A else B` | Safe for falsy, one branch |

## Reference: Allocation Reduction

| Slow | Fast | Why |
|---|---|---|
| `local t = {}` in loop | `table.clear(t)` + reuse | Avoids GC pressure |
| Repeated `table.insert` | `table.create(n)` + indexed writes | Preallocated |
| Manual `pairs` clone | `table.clone(t)` | Faster, copies layout |
| `string.byte`/`char` loops | `string.pack`/`unpack` | Native implementation |
| String binary data | `buffer` type | Fixed-size, efficient |

## Reference: Inlining Rules

| Blocks inlining | Enables inlining | Why |
|---|---|---|
| `function Module.foo()` | `local function foo()` | Mutable table prevents |
| `function Obj:method()` | `local function method(self)` | OOP syntax not inlineable |
| Recursive function | Split base/recursive | Recursion prevents |
| Deep upvalue captures | Pass as parameters | Reduces closure cost |

## Reference: Native Codegen

**`@native` vs `--!native`:**
- `--!native`: entire script. Good for math-heavy utility modules.
- `@native`: per-function. Better for selective hot functions or near the 1M instruction limit.
- Inner functions do NOT inherit `@native`.
- Top-level code runs once; `@native` has minimal benefit there.

**Hurts native:** `getfenv`/`setfenv`, wrong types to typed functions, non-numeric math args, breakpoints, size limits (64K instructions/block, 32K blocks/function, 1M/module).

**Helps native:** Type annotations (especially `Vector3`), small functions, consistent table shapes, `buffer` ops, `bit32` ops.

## Reference: CLI Tools

### `luau-compile` -- Static bytecode/codegen analysis (primary tool)

Modes (mutually exclusive): `binary` (raw bytecode), `text` (human-readable opcodes with source annotations), `remarks` (source with inlining/allocation comments -- **most useful**), `codegen` (native assembly, requires `--target`).

Options: `-O<n>` (0-2, default 1; `-O2` enables inlining), `-g<n>` (debug level 0-2), `--target=<arch>` (`x64`, `x64_ms`, `a64`, `a64_nf`; use `x64_ms` on Windows), `--record-stats=<total|file|function>` (JSON stats: bytecodeInstructionCount, lowerStats with spills/skipped/blocks/errors), `--bytecode-summary` (opcode distribution, requires `--record-stats=function`), `--stats-file=<name>` (default `stats.json`), `--timetrace` (trace.json), `--vector-lib`/`--vector-ctor`/`--vector-type` (always set for Roblox).

### `luau-analyze` -- Type checking and linting

Modes: omitted (typecheck + lint), `--annotate` (source with all inferred types inline -- **critical for finding `any`**).

Options: `--mode=strict` (force strict even without directive), `--formatter=plain` (machine-parseable), `--formatter=gnu` (grep-friendly), `--timetrace`.

### `luau` -- Runtime execution, profiling, and coverage

**Limited for Roblox scripts** (most depend on engine APIs). Only usable for pure-logic modules. For Roblox scripts, use Studio Script Profiler and `debug.dumpcodesize()`.

Options: `-O<n>`, `-g<n>`, `--codegen` (native execution), `--profile[=N]` (sampling at N Hz, default 10000, outputs `profile.out`), `--coverage` (outputs `coverage.out`), `--timetrace`, `-a` (program args).

### `luau-ast` -- AST dump

No flags. Outputs full JSON AST with node types, source locations, variable scopes, type annotations. Useful for programmatic analysis: function nesting depth, function body size (monolith detection), closure captures (upvalue analysis), table construction patterns, loop-invariant expression detection.

## Reference: VM Architecture

**Fastcall builtins:** `assert`, `type`, `typeof`, `rawget`/`rawset`/`rawequal`/`rawlen`, `getmetatable`/`setmetatable`, `tonumber`/`tostring`, most `math.*` (not `noise`/`random`/`randomseed`), `bit32.*`, some `string.*`/`table.*`. Partial: `assert` (unused return + truthy), `bit32.extract` (constant field/width), `select(n, ...)` O(1). With `-O2`: constant-arg builtins folded; `math.pi`, `math.huge` folded.

**Inline caching:** HREF-style for table fields. Compiler predicts hash slot; VM corrects. Best: compile-time field name, no metatables, uniform shapes.

**Import resolution:** `math.max` resolved at load time. Invalidated by `getfenv`/`setfenv`/`loadstring`.

**GC:** Incremental mark-sweep with "GC assists" (allocation pays proportional GC). Paged sweeper (16 KB pages). No `__gc` metamethod.

**Compiler:** Multi-pass. Constant folding across functions/locals. Upvalue optimization. `-O2`: inlining, loop unrolling (constant bounds), aggressive constant folding. Interprocedural limited to single modules.

**Table length:** `#t` is O(1) cached, worst case O(log N) branch-free binary search. `table.insert`/`table.remove` update the cache.

**Upvalues:** ~90% immutable. Immutable = no allocation, no closing, faster access. Mutable = extra object.

**Closures:** Cached when no upvalues, or all upvalues immutable and module-scope.

## Reference: Documentation to Consult

Read these docs during execution if needed for specific detail. Two doc trees:
- `.cursor/rules/luau/` -- Official Luau site docs (guides, types, reference, getting-started)
- `.cursor/rules/luau-rfcs/` -- Luau RFCs (language proposals, library additions)

**Luau site docs (primary):**
- `.cursor/rules/luau/guides/performance.md` -- **The official Luau performance guide.** Covers interpreter, compiler, fastcalls, inline caching, imports, table creation, GC, inlining, loop unrolling, upvalues, closures.
- `.cursor/rules/luau/guides/profile.md` -- Profiling guide (flame graphs, sampling)
- `.cursor/rules/luau/types/` -- Full type system docs (basic-types, tables, generics, refinements, unions-and-intersections, object-oriented-programs, roblox-types, considerations)
- `.cursor/rules/luau/getting-started/lint.md` -- All 28 lint rules
- `.cursor/rules/luau/reference/library.md` -- Complete library reference
- `.cursor/rules/luau/getting-started/compatibility.md` -- Luau vs Lua differences

**Luau RFCs (specific features):**
- `.cursor/rules/luau-rfcs/function-inlining.md` -- Inlining RFC (no user `@inline`, automatic at `-O2`)
- `.cursor/rules/luau-rfcs/syntax-attribute-functions-native.md` -- `@native` per-function attribute
- `.cursor/rules/luau-rfcs/function-table-create-find.md`, `function-table-clear.md`, `function-table-clone.md`, `function-table-freeze.md` -- Table operations
- `.cursor/rules/luau-rfcs/function-math-lerp.md` -- `math.lerp`
- `.cursor/rules/luau-rfcs/function-bit32-byteswap.md`, `function-bit32-countlz-countrz.md` -- CPU-level bit ops
- `.cursor/rules/luau-rfcs/function-buffer-bits.md`, `type-byte-buffer.md` -- Buffer type and ops
- `.cursor/rules/luau-rfcs/vector-library.md` -- Vector library and fastcalls
- `.cursor/rules/luau-rfcs/function-string-pack-unpack.md` -- Binary string operations
- `.cursor/rules/luau-rfcs/syntax-floor-division-operator.md` -- `//` dedicated opcode
- `.cursor/rules/luau-rfcs/syntax-compound-assignment.md` -- `+=` single LHS evaluation
- `.cursor/rules/luau-rfcs/deprecate-getfenv-setfenv.md` -- Why fenv disables optimizations
- `.cursor/rules/luau-rfcs/deprecate-table-getn-foreach.md` -- Deprecated table functions
- `.cursor/rules/luau-rfcs/generalized-iteration.md` -- Modern iteration patterns
- `.cursor/rules/luau-rfcs/generic-functions.md`, `never-and-unknown-types.md`, `syntax-type-ascription.md` -- Type system RFCs

**Cursor rules:**
- `.cursor/rules/luau.mdc` -- Luau language rules and optimization tips

**Roblox docs:**
- `.cursor/rules/roblox/en-us/luau/native-code-gen.md` -- Native codegen (size limits, Vector3 annotation impact)
- `.cursor/rules/roblox/en-us/luau/type-checking.md` -- Type annotation syntax and modes
- `.cursor/rules/roblox/en-us/performance-optimization/improve.md` -- Script computation, memory, physics, rendering
- `.cursor/rules/roblox/en-us/performance-optimization/design.md` -- Event-driven patterns, frame budgets
- `.cursor/rules/roblox/en-us/luau/variables.md` -- Local vs global performance
- `.cursor/rules/roblox/en-us/luau/scope.md` -- Scope performance implications
