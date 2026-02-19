---
name: luau-optimize cursor command
overview: Create a `/luau-optimize` Cursor command at `.cursor/commands/luau-optimize.md` that uses the `luau-compile`, `luau-analyze`, and `luau-ast` CLI tools to analyze and optimize Luau files for maximum performance, type safety, and native codegen quality.
todos:
  - id: create-command
    content: Create `.cursor/commands/luau-optimize.md` with the full multi-phase optimization workflow
    status: completed
isProject: false
---

# /luau-optimize Cursor Command

## Deliverable

Single file: `[.cursor/commands/luau-optimize.md](.cursor/commands/luau-optimize.md)`

## Context

**This command is always for Roblox Luau.** All scripts being optimized run in the Roblox engine (Studio / live servers / clients). This means:

- Roblox APIs (`game`, `workspace`, `Players`, `RunService`, etc.) are always available
- The vector type is `Vector3` (Roblox userdata), not the `vector` primitive
- Native codegen (`--!native`) primarily benefits server-side Scripts; client LocalScripts benefit less due to device diversity
- The `luau` standalone runtime cannot execute most Roblox scripts (they depend on engine APIs); it is only useful for pure-logic library modules with no Roblox dependencies
- `luau-compile` should use `--vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3` for accurate Roblox vector optimization analysis
- Roblox performance patterns (event-driven design, frame budgets, RunService events, instance streaming, replication) are always relevant
- Studio testing uses `--!optimize 1` by default; live experiences use `--!optimize 2`

## Design

The command is a structured prompt file that instructs the AI agent through a multi-phase optimization workflow. It supports both single-file and multi-file/directory modes.

### Phase Structure

**Phase 0 -- Scope Resolution and Configuration**

Resolve the target:

- If user specifies a file: optimize that file
- If user specifies a directory: discover all `.luau`/`.lua` files, prioritize by traffic/complexity
- If no argument: optimize the currently open file

Infer script context from filename and path (checked in this order):

1. **Filename suffix** (highest priority, overrides path):
  - `*.server.luau` / `*.server.lua` -> always server Script (`--!native` is high-value)
  - `*.client.luau` / `*.client.lua` -> always client Script/LocalScript (`--!native` less beneficial due to device diversity)
  - `*.legacy.luau` / `*.legacy.lua` -> **ambiguous** (could be cloned/moved at runtime). Ask the user where it runs.
  - `*.luau` / `*.lua` (no suffix) -> ModuleScript, context depends on who requires it. Fall through to path check.
2. **Path-based inference** (for ModuleScripts and ambiguous cases):
  - Path contains `ServerScriptService`/`ServerStorage` -> server-side module
  - Path contains `StarterPlayer`/`StarterGui`/`StarterCharacterScripts`/`ReplicatedFirst` -> client-side module
  - Path contains `ReplicatedStorage`/`shared` -> shared module (runs on both; conservative `--!native` recommendation)
  - If path is ambiguous or outside a Roblox project structure -> ask the user

**Quiz the user using AskQuestion tool (combined, structured):**

```
Title: "/luau-optimize Configuration"

Question 1: "Compilation optimization intensity"
  - minimal -- Headers (--!strict/--!native/--!optimize 2), type annotations on all function signatures, simple deprecated pattern replacements
  - moderate -- + function restructuring for inlining, import hoisting, compound operators, fastcall enablement, allocation reduction
  - insane -- + every micro-optimization from the pattern catalog, full bytecode analysis, register pressure optimization, closure caching analysis
  - (other: freeform)

Question 2: "Algorithm optimization"
  - none -- Don't touch logic or data flow
  - low hanging fruit -- Obvious O(n^2)->dict, redundant iterations, missing caches
  - moderate -- + data structure changes, caching strategies, event-driven refactors, loop fusion
  - insane -- + full algorithmic redesign where beneficial, dynamic programming, architectural restructuring
  - (other: freeform)

Question 3: "Benchmarking"
  - yes -- Benchmark all non-trivial changes with luau CLI (when function is standalone)
  - no -- Trust bytecode/codegen metrics only
  - +EV only -- Benchmark only when algorithmic changes or ambiguous bytecode results warrant it
```

**Derived behavior from quiz answers:**


| Setting                  | minimal                     | moderate                              | insane                                   |
| ------------------------ | --------------------------- | ------------------------------------- | ---------------------------------------- |
| Headers                  | Always                      | Always                                | Always                                   |
| Type annotations         | Function signatures         | Function signatures + hot path locals | Function signatures + all locals         |
| Pattern replacements     | Deprecated only             | Full Priority 2 catalog               | Full Priority 2 + Priority 5             |
| Function restructuring   | No                          | Yes (Priority 3)                      | Yes + aggressive splitting               |
| Algorithmic changes      | Per algo quiz               | Per algo quiz                         | Per algo quiz                            |
| Bytecode verification    | After all changes           | After each priority                   | After each individual change             |
| `@native` vs `--!native` | `--!native` unless big file | Selective `@native` on hot functions  | Full analysis of which functions benefit |


**Restructuring scope** scales with compilation intensity:

- minimal: No restructuring. Only additive changes (headers, annotations, pattern swaps).
- moderate: Break up monoliths, reorder locals for inlining, hoist imports. Preserve existing exports/API.
- insane: Aggressive restructuring permitted. May change internal function boundaries, introduce helper modules, rewrite data flow.

**Phase 1 -- Deep Code Read and Algorithmic Analysis (FIRST)**

Read the entire source file thoroughly before touching any tools. This is the highest-leverage phase -- algorithmic improvements produce order-of-magnitude speedups that dwarf any bytecode-level optimization. This is also the largest code change, so it must come first (everything else builds on top of it).

Understand the code's purpose, data flow, and architecture.

**Step 1 -- Structural decomposition (read and mentally refactor)**

Monolithic functions hide their actual algorithmic structure behind walls of interleaved concerns. Before analyzing complexity, first understand what the code is *actually doing* by decomposing it mentally (and then physically if restructuring is allowed):

- **Identify responsibilities**: What distinct jobs does each large function perform? A 200-line function usually contains 3-5 logical steps that should be separate functions.
- **Trace data flow**: What goes in, what comes out, what is mutated along the way? Map the pipeline.
- **Separate concerns**: Validation, transformation, I/O, state management, error handling -- these are often tangled together. Separating them reveals the core algorithm.
- **Name the steps**: If you can't name what a code block does in 3-5 words, it's doing too much.
- **Look for hidden abstractions**: Repeated patterns of "iterate, filter, transform, collect" that could be a single well-named helper.

When the quiz allows restructuring (moderate/insane compilation intensity):

- Physically decompose monoliths into focused local functions. Each function should do one thing.
- This decomposition often *reveals* optimization opportunities that were invisible in the monolith: you can now see that step 2 recomputes what step 1 already knew, or that the inner loop could be replaced with a lookup table built in the outer loop.
- Smaller functions are also easier for the compiler to inline at `-O2`, so this restructuring pays double -- clearer code AND better bytecode.
- Preserve the original function as a thin orchestrator that calls the decomposed pieces.

Even when the quiz says "minimal" (no restructuring), still perform this mental decomposition to inform the analysis below.

**Step 2 -- Algorithmic analysis**

With the structure understood (or cleaned up), analyze:

- **Complexity analysis**: Identify the time complexity of every significant function. Look for:
  - O(n^2) or worse: nested loops over the same data, repeated linear searches, quadratic string building
  - O(n) where O(1) is possible: linear search replaceable with dictionary/set lookup
  - Repeated work: same computation done multiple times when it could be cached/memoized
  - Unnecessary copies: cloning data that could be referenced or mutated in place
  - Hidden quadratics: a function that looks O(n) but calls another O(n) function inside a loop
- **Data structure fitness**: Is the right data structure being used?
  - Array used for membership testing -> should be a set (dictionary with `true` values)
  - Linear scan for lookup by key -> should be a dictionary
  - Repeated sort of mostly-sorted data -> consider maintaining sorted order on insert
  - Flat list with frequent removal from middle -> consider swap-and-pop
  - Large string built incrementally -> `table.concat` pattern or `buffer`
  - Multiple parallel arrays tracking related data -> single array of structs (table of tables)
  - Unbounded growth without cleanup -> consider size limits, LRU eviction, or periodic pruning
- **Caching and memoization**:
  - Pure functions called with the same args repeatedly -> memoize
  - Expensive property reads in loops -> cache in local before loop
  - Computed values that don't change within a scope -> hoist out
  - Derived state recomputed from scratch on every access -> maintain incrementally
- **Redundant work elimination**:
  - Multiple passes over the same data that could be fused into one
  - Recomputing derived state that could be maintained incrementally
  - Sorting or filtering that could be done once and reused
  - Deep cloning where shallow clone or reference suffices
  - Building an intermediate result that is immediately discarded after extracting one field
- **Architectural patterns**:
  - Per-frame polling where event-driven would work (RunService events are expensive)
  - Synchronous work that could be chunked across frames with `task.wait()`
  - Undisconnected event connections (memory leak + wasted computation)
  - Rebuilding entire state on small change where incremental update would work
  - God-object that owns too much state -> split responsibilities
- **Hot path identification**: Which functions are called most frequently?
  - Event handlers connected to RunService (PreAnimation, PreRender, PreSimulation, PostSimulation, Heartbeat)
  - Functions called inside loops
  - Recursive functions
  - Functions called from other hot functions

**Step 3 -- Present findings and quiz the user**

Before applying any changes, present a summary of all findings organized by impact:

```
For each finding, present:
- What: One-line description of the issue
- Where: Function name / line range
- Why: What makes it slow (e.g., "O(n^2) nested scan on every frame")
- Fix: Proposed change in plain language
- Impact: Estimated improvement (order-of-magnitude, constant-factor, or clarity-only)
```

Then quiz the user using AskQuestion tool to let them control the flow:

```
Title: "Phase 1 Findings -- Select which changes to apply"

For each finding, one question with options:
  - apply -- Apply this change
  - skip -- Skip this change
  - modify -- I want a different approach (wait for user input)

If there are many findings (5+), group them by category and present one question per category:
  - "Structural decomposition: [list of proposed decompositions]. Apply all / cherry-pick / skip all"
  - "Data structure changes: [list]. Apply all / cherry-pick / skip all"
  - "Algorithm changes: [list]. Apply all / cherry-pick / skip all"
  - "Architectural changes: [list]. Apply all / cherry-pick / skip all"
```

If the algorithm quiz from Phase 0 was "none", present findings as informational only (no apply option). The user still sees what was found.

**Step 4 -- Apply approved changes**

Apply only the user-approved changes, in this order:

1. Structural decomposition first (if approved) -- break monoliths into focused functions. This is the foundation; other changes may depend on it.
2. Data structure changes -- swap arrays for dicts, add indexes, etc.
3. Algorithm changes -- replace O(n^2) with O(n), add caching/memoization
4. Architectural changes -- event-driven refactors, incremental updates

If the user selected "modify" on any finding, wait for their input before proceeding with that specific change.

Benchmark algorithmic changes if the benchmarking quiz answer warrants it (Phase 4.5 pattern).

**Phase 2 -- Baseline Capture (read-only)**

Run all diagnostic tools on the code (post-algorithmic changes if any were made in Phase 1) and capture baseline metrics for the bytecode optimization phases:

1. `luau-analyze --mode=strict <file>` -- type errors and lint warnings
2. `luau-analyze --annotate <file>` -- inferred types (count `any` occurrences for type coverage score)
3. `luau-compile --remarks -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>` -- inlining success/failure, allocation remarks (with Roblox vector optimization)
4. `luau-compile --text -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>` -- full bytecode listing (instruction count, opcodes)
5. `luau-compile --codegen --target=x64_ms --record-stats=function -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>` -- native codegen stats (spills, skipped functions, block counts). Use `--target=x64` on Linux/Mac.

Parse and summarize:

- Total bytecode instruction count
- Allocation count (NEWTABLE, NEWCLOSURE from remarks)
- Inlining success/failure count (from remarks)
- Type coverage: count of `any` types in annotate output
- Register spills and skipped functions from codegen stats
- Lint warnings count

**Phase 3 -- Compilation-Level Code Analysis**

Using both the source (already read in Phase 1) and the tool output from Phase 2, identify bytecode-level optimization opportunities:

- **Missing types**: `any` inferences that hurt native codegen (especially `Vector3`, `CFrame`, `buffer` params)
- **Import patterns**: Global chains like `math.max` are resolved at load time (not execution) via GETIMPORT. Broken by `getfenv`/`setfenv`/`loadstring` which mark environment "impure" and disable this.
- **Allocation in loops**: Table/closure creation inside hot loops (NEWTABLE, NEWCLOSURE). High allocation rate = more GC assist work. Avoid temporary tables and userdata in hot loops.
- **Closure caching**: Repeated function expressions can be cached (same closure reused) when: no upvalues, or all upvalues are immutable and declared at module scope. Mutable upvalue captures prevent caching.
- **Upvalue mutability**: ~90% of upvalues are immutable in typical code. Immutable upvalues = no extra allocation, no closing, faster access, better locality. Mutable upvalues need extra objects.
- **Method call patterns**: `obj:Method()` uses specialized fast method call instruction. Avoid `obj.Method(obj)`. For tables: `__index` should point at a table directly (not a function or deep chain). Ideal: metatable whose `__index` points to itself.
- **Inline caching**: VM uses inline caching for table field access. Best when field name is known at compile time, no metatables, and object shapes are uniform. Varying keys/shapes degrade performance.
- **pcall in hot paths**: Prevents native codegen optimization
- **String concatenation patterns**: `..` in loops vs `table.concat`
- **Deprecated/deoptimizing API usage**: `getfenv`/`setfenv` (marks env "impure", disables ALL import optimization and fastcalls), `loadstring` (same deoptimization), `table.getn`/`table.foreach`/`table.foreachi`, `wait()`. Even read-only `getfenv()` deoptimizes.
- **Builtin global writes**: Overwriting builtins (e.g., `math = ...`) disables fastcall optimization. Lint rule `BuiltinGlobalWrite` catches this.
- **Metamethod cost**: `__eq` is always called on `==`/`~=` even for rawequal values; ensure cheap implementations
- **GC pressure**: Incremental GC uses "GC assists" -- allocating code pays for collection work proportionally. Fewer allocations = less GC overhead.

**Phase 4 -- Compilation Optimization (graduated priority)**

Apply changes in priority order, re-running `luau-compile --remarks -O2` after structural changes to verify the compiler actually benefits:

**Priority 1 -- Headers and type safety (always apply)**

- Add `--!strict` / `--!native` / `--!optimize 2` header (note: `--!optimize 2` is default in live experiences but not in Studio testing)
- Fix type errors reported by `luau-analyze`
- Add missing type annotations where `--annotate` shows `any` (focus on function signatures first, then locals in hot paths)
- Annotate `Vector3`, `CFrame`, `buffer` parameters explicitly -- native codegen generates specialized vector code when types are known; unannotated params are assumed to be tables with extra checks
- Replace `getfenv`/`setfenv` usage -- these disable builtin calls, imports, and optimizations globally for the affected function
- Replace `table.getn` with `#t` or `rawlen(t)`, `table.foreach`/`table.foreachi` with `for..in` loops

**Priority 2 -- Low-hanging structural wins**

- Hoist frequently-used library functions: `local floor = math.floor` enables `GETIMPORT` fastcall. Fastcall builtins include: `assert`, `type`, `typeof`, `rawget`/`rawset`/`rawequal`, `getmetatable`/`setmetatable`, `tonumber`/`tostring`, most `math.`* (not `noise`, `random`/`randomseed`), `bit32.`*, and some `string.`*/`table.*`. Partial specializations: `assert` when return value unused and condition truthy; `bit32.extract` when field/width constant; `select(n, ...)` is O(1) via builtin dispatch.
- Replace `pairs(t)` / `ipairs(t)` with generalized `for k, v in t do` iteration. Generalized iteration skips the `pairs()` call, which matters for very short tables. Performance of all three is similar for longer tables. Note: `for i=1,#t` is slightly slower due to extra element access cost.
- Use `//` floor division instead of `math.floor(a / b)` -- dedicated VM opcode, supports `__idiv` metamethod, also `//=` compound form
- Use compound assignment (`+=`, `-=`, `*=`, `//=`, `..=`) -- LHS evaluated once, e.g. `data[index].cost += 1` avoids double-indexing
- Replace string concatenation in loops with `table.concat` or string interpolation (backticks lower to `string.format` with no extra overhead)
- Use `table.create(n)` for known-size arrays -- preallocates capacity, much faster than repeated `table.insert`
- For sequential fill with known index, use indexed writes with `table.create`: `local t = table.create(N); for i=1,N do t[i] = ... end` (fastest pattern)
- Use `table.insert(t, v)` for appending when size unknown -- `#t` is usually O(1) (cached, updated by `table.insert`/`table.remove`; worst case O(log N) branch-free binary search, "50%+ faster" than Lua's O(N))
- Use `math.lerp(a, b, t)` instead of `a + (b - a) * t` -- exact for t=0/1, monotonic, bounded
- Use `bit32.byteswap(n)` instead of manual shifting/masking for endian conversion (uses CPU bswap instruction)
- Use `bit32.countlz`/`bit32.countrz` instead of manual log2 loops (uses CPU instructions, ~8x faster)
- Use `table.find(t, v)` instead of manual linear search loops
- Use `table.clone(t)` instead of manual `pairs` copy loops
- Use `string.pack`/`string.unpack` instead of manual `string.byte`/`string.char`/`bit32` for binary data
- Use `rawlen(t)` when metamethods are not needed for table length
- Use explicit `./`/`../`/`@` prefixes in `require()` to avoid extra path-resolution work

**Priority 3 -- Function structure for inlining**

- Break monolith functions into smaller, focused local functions (compiler inlines automatically at `-O2`; small local functions = best inlining candidates)
- Inlining requirements: function must be local, non-mutated, non-recursive, not an OOP method (`Foo:bar()`), not a metamethod. Exported functions in mutable tables CANNOT be inlined. Disabled in modules using `getfenv`/`setfenv`.
- Inlining + constant folding: inlined functions with constant args can enable further constant folding (e.g., `local function double(x) return x*2 end; local y = double(5)` folds to `y = 10`)
- For high-traffic functions that call imported module methods frequently, create local function wrappers at module top-level to enable inlining
- Move `pcall` out of hot loops where possible (wrap the loop, not the body) -- pcall prevents native codegen optimization
- Use `obj:Method()` not `obj.Method(obj)` -- colon syntax gets specialized fast method call instruction
- Keep `__index` pointing at a table directly (not a function or deep chain) for optimal inline caching
- Reduce upvalue captures in closures created inside loops. Immutable upvalues (never reassigned) are much cheaper -- no extra allocation, no closing. Mutable upvalues need an extra object.
- Prefer object-like table construction with all fields set at once (enables compiler capacity inference): `local v = {}; v.x = 1; v.y = 2; v.z = 3` -- compiler infers hash capacity from subsequent assignments
- Loop unrolling: only for compile-time constant bounds (`for i=1,4 do`); body must be simple. Unrolling enables further constant folding on the loop index.
- Watch for native codegen size limits: 64K instructions per code block, 32K blocks per function, 1M instructions per module. Split large functions if limits are hit.

**Priority 4 -- Micro-optimizations (hot paths only)**

- Table shape consistency (same fields, same order across constructors) -- VM uses inline caching (HREF-style) that predicts hash slots; uniform shapes = better cache hit rates
- Use `buffer` for binary data manipulation instead of string ops (fixed-size, offsets start at 0, far more space/time efficient)
- Use `buffer.readu32` + `bit32` operations instead of `buffer.readbits` when schema is known (faster due to fewer bounds checks)
- Strength reduction: multiplication by power-of-2 to `bit32.lshift`, division by power-of-2 to `bit32.rshift`
- Minimize `tostring`/`tonumber` in tight loops
- Use `table.freeze` for readonly config tables (avoids need for proxy tables and `__index` overhead)
- Keep `__eq` metamethods cheap -- they fire on every `==`/`~=` and `table.find` comparison
- Use `math.isfinite`/`math.isnan`/`math.isinf` instead of `x ~= x` patterns for clarity (no perf difference)
- For vector math: use `Vector3.new(x, y, z)` (Roblox userdata, not the `vector` primitive); annotate params as `Vector3` for native specialization
- Use `if expr then A else B` instead of `cond and A or B` ternary (only evaluates one branch; avoids falsy-value bugs)

**Phase 4.5 -- Benchmark with `luau` CLI (when warranted)**

When a non-trivial optimization is applied to an isolated, pure-logic function (no Roblox API dependencies), write a small benchmark harness and run it with the `luau` CLI to measure actual wall-clock impact. This is especially valuable for:

- Algorithmic changes (Priority 4) where bytecode metrics alone can't prove the win
- Micro-optimizations (Priority 5) where the improvement is ambiguous
- Competing approaches where both look reasonable in bytecode but one may be faster at runtime

**Benchmark harness pattern:**

```luau
-- bench.luau (temporary file, deleted after benchmarking)
local function original(...)
    -- paste original implementation
end

local function optimized(...)
    -- paste optimized implementation
end

local ITERATIONS = 1_000_000
local clock = os.clock

-- Warmup
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

**Run with both interpreter and native codegen to compare:**

```
luau -O2 bench.luau                     # Interpreter at -O2
luau -O2 --codegen bench.luau           # Native codegen at -O2
```

**When to benchmark:**

- The function is self-contained (no `game`, `workspace`, Roblox services)
- The optimization changes algorithm or data structure (not just headers/types)
- The bytecode diff is ambiguous (instruction count similar, or more instructions but fewer allocations)
- Two candidate approaches need a tiebreak

**When NOT to benchmark:**

- The function depends on Roblox APIs (use Studio Script Profiler instead)
- The change is purely additive (headers, type annotations) -- trust the bytecode metrics
- The function is not hot (benchmarking cold code is misleading)

**Cleanup:** Delete the temporary benchmark file after capturing results. Do not leave bench files in the project.

**Phase 5 -- Verification**

Re-run all Phase 2 tools and present a before/after comparison:

- Bytecode instruction count delta
- Allocation count delta
- Inlining success rate delta
- Type coverage improvement (fewer `any` types)
- Register spill delta
- Lint warning delta
- Benchmark results (if Phase 4.5 was used) -- wall-clock speedup for interpreter and native codegen

If any metric regressed, investigate and explain why (or revert that change).

**Phase 6 -- Multi-file mode (directory scope only)**

When optimizing a directory:

- Run Phase 1 (code read) on all files first to identify which files have the most algorithmic improvement potential
- Run Phase 2 (baseline capture) on all files to identify which have the most bytecode optimization potential
- Prioritize files by: worst algorithmic complexity, most lint warnings, most `any` types, most allocations, most failed inlines
- Process files one at a time through Phases 1-5
- Present aggregate metrics at the end

### Key Principles (embedded in command)

- **Same implementation, better performance** -- behavioral equivalence is non-negotiable
- **Low-hanging fruit first** -- headers and types before restructuring
- **Compiler feedback loop** -- verify each structural change with `--remarks`
- **Small functions over monoliths** -- easier for compiler, easier to maintain
- **Local functions for inlining** -- even at cost of duplicating imported behavior in high-traffic code
- **Quantify everything** -- before/after metrics for every optimization pass
- **Type annotations drive native codegen quality** -- every `any` is a missed specialization opportunity
- **Event-driven over polling** -- avoid per-frame calculations when events suffice

### Type System Availability (Old Typechecker)

The command must only recommend type features available with the OLD typechecker (we are NOT using the new type solver).

**Available NOW (old typechecker):**

- `--!strict` / `--!nonstrict` / `--!nocheck` directives
- Basic type annotations: `local x: number`, `function f(a: string): boolean`
- Optional types: `string?` (sugar for `string | nil`)
- Union types: `number | string`
- Intersection types: `type1 & type2`
- Generic functions: `function id<T>(x: T): T`
- Explicit generic instantiation: `f<<number>>()`
- Type ascription (cast): `x :: string`
- Array types: `{number}`, dictionary types: `{[string]: number}`
- Typed table constructors: `type Car = { Speed: number, Drive: (Car) -> () }`
- `typeof()` for inferring types from values
- `export type` for cross-module type sharing
- Typed variadics: `function f(...: number)`
- Type alias defaults: `type Map<K, V = string> = {[K]: V}`
- Singleton/literal types: `"hello"`, `true`
- `never` and `unknown` types (exhaustiveness checking, safe top type)
- `read` / `write` property modifiers
- Sealed/unsealed table subtyping
- `table.freeze` type narrowing

**REQUIRES new typechecker (DO NOT recommend):**

- `keyof<T>`, `index<T, K>`, `rawkeyof<T>`, `rawget<T, K>` type functions
- `getmetatable<T>`, `setmetatable<T, M>` type functions
- User-defined type functions (`type function ...`)
- Negation types (`~T`)
- Relaxed recursive type restrictions
- Local type inference improvements (better bounds tracking)
- `new-nonstrict` unified engine

### Native Codegen Knowledge Base

Embedded in the command for reference during optimization:

**When to use `@native` vs `--!native`:**

- `--!native`: entire script compiled natively. Good for math-heavy utility modules.
- `@native`: per-function. Better when only specific hot functions benefit, or when script is close to the 1M instruction module limit.
- Inner functions do NOT inherit `@native` -- each needs its own annotation.
- Top-level module code usually runs once; `@native` on it has minimal benefit.

**What hurts native execution:**

- `getfenv()` / `setfenv()` -- disables optimizations
- Passing wrong types to typed functions (triggers type guard failures)
- Non-numeric args to math builtins
- Breakpoints disable native execution for affected functions
- Functions exceeding size limits: 64K instructions/block, 32K blocks/function, 1M instructions/module

**What helps native execution:**

- Type annotations, especially `Vector3` parameters (specialized vector codegen). JIT uses annotations directly -- no runtime type analysis.
- Small, focused functions (fewer register spills, better block optimization)
- Consistent table shapes (predictable field access, inline caching hits)
- `buffer` operations (efficient native lowering)
- `bit32` operations (map directly to CPU instructions)
- Native 3-component 32-bit float vector type: 16-byte tagged values, VM-level 3-wide SIMD, lower GC pressure than userdata Vector3

### VM Architecture Knowledge

The command should understand how the Luau VM works to give accurate advice:

**Bytecode interpreter:**

- Highly tuned portable bytecode interpreter in C. Core dispatch loop ~16 KB on x64 (cache-friendly).
- Can match LuaJIT interpreter on some workloads.
- Computationally intensive scripts mostly use the interpreter core + builtins.

**Compiler pipeline:**

- Multi-pass: frontend parses AST, backend emits bytecode.
- Without type info: deep constant folding (across functions and locals), upvalue optimization for non-mutated upvalues, builtin usage analysis, multi-assignment optimization, peephole optimizations.
- With `-O2`: function inlining, loop unrolling (constant bounds only), more aggressive constant folding after inlining.
- Type-directed: currently small peephole optimizations; potential for CSE and allocation hoisting.
- Interprocedural: limited to single modules. Local function calls optimized with known arg/return counts.

**Inline caching:**

- Table field access uses HREF-style inline caching. Compiler predicts hash slot; VM corrects dynamically.
- Best when: field name known at compile time, no metatables, uniform object shapes.
- `table.field` and `table["constantString"]` both get inline caching.

**Import resolution:**

- Global chains like `math.max` resolved at load time, not execution time.
- Invalidated by `getfenv`/`setfenv`/`loadstring` (marks environment "impure").

**Fastcall builtins (complete list):**

- `assert`, `type`, `typeof`, `rawget`, `rawset`, `rawequal`, `rawlen`
- `getmetatable`, `setmetatable`
- `tonumber`, `tostring`
- Most `math.`* (NOT `math.noise`, `math.random`, `math.randomseed`)
- `bit32.`*
- Some `string.`*, `table.`*
- Partial specializations: `assert` (unused return + truthy), `bit32.extract` (constant field/width), `select(n, ...)` (O(1) via builtin dispatch)
- `math.floor` and similar use SSE4.1 when available
- With `-O2`: builtins with constant args and single return can be constant-folded; `math.pi`, `math.huge` etc. are folded

**GC architecture:**

- Incremental mark-sweep with "GC assists" (allocating code pays proportional GC work)
- Paged sweeper: 16 KB pages by size class, 2-3x faster than linked-list, saves 16 bytes/object on 64-bit
- No `__gc` metamethod (removed for performance)
- Weak tables with `__mode = "s"` are "shrinkable" (resized during GC)

### Type Refinement Patterns

The compiler narrows types after certain checks, improving both type safety and native codegen quality:

- **Truthiness:** `if x then` narrows `x` from falsy (`nil`/`false`)
- **Type guards:** `if type(x) == "number" then` narrows `x` to `number`
- **Typeof guards (Roblox):** `if typeof(x) == "Vector3" then` narrows to `Vector3`; `x:IsA("TextLabel")` narrows `Instance` to subclass
- **Equality:** `if x == "hello" then` narrows `x` to singleton `"hello"`
- **Assert:** `assert(type(x) == "string")` narrows `x` to `string` after the call
- **Composition:** Supports `and`, `or`, `not` for compound refinements

Use refinements to provide type information to the compiler without explicit annotations in contexts where the type is known from a runtime check.

### Comprehensive Pattern Catalog

The command will embed this as the reference for pattern replacement:

**Bytecode-level improvements:**


| Slow pattern                  | Fast pattern                   | Why                                 |
| ----------------------------- | ------------------------------ | ----------------------------------- |
| `math.floor(a / b)`           | `a // b`                       | Dedicated VM opcode                 |
| `data[i].x = data[i].x + 1`   | `data[i].x += 1`               | LHS evaluated once                  |
| `a + (b - a) * t`             | `math.lerp(a, b, t)`           | Exact at endpoints, monotonic       |
| `"prefix" .. var .. "suffix"` | `prefix{var}suffix`            | Lowers to optimized `string.format` |
| Manual byte swap              | `bit32.byteswap(n)`            | CPU bswap instruction               |
| Manual log2 loop              | `bit32.countlz(n)`             | CPU instruction, ~8x faster         |
| `table.getn(t)`               | `#t` or `rawlen(t)`            | Deprecated, slower                  |
| `table.foreach(t, f)`         | `for k, v in t do f(k, v) end` | Deprecated, slower                  |
| `cond and A or B`             | `if cond then A else B`        | Safe for falsy values, one branch   |
| `x ~= x` (NaN check)          | `math.isnan(x)`                | Clearer, same performance           |


**Allocation reduction:**


| Slow pattern                      | Fast pattern                       | Why                                 |
| --------------------------------- | ---------------------------------- | ----------------------------------- |
| `local t = {}` in loop            | `table.clear(t)` + reuse           | Avoids GC pressure                  |
| `table.insert(t, v)` repeated     | `table.create(n)` + indexed writes | Preallocated capacity               |
| Manual `pairs` clone              | `table.clone(t)`                   | Faster, copies layout               |
| `string.byte`/`string.char` loops | `string.pack`/`string.unpack`      | Native implementation               |
| String-based binary data          | `buffer` type                      | Fixed-size, offset-based, efficient |


**Inlining enablers:**


| Blocks inlining         | Enables inlining              | Why                                      |
| ----------------------- | ----------------------------- | ---------------------------------------- |
| `function Module.foo()` | `local function foo()`        | Exported/mutable table prevents inlining |
| `function Obj:method()` | `local function method(self)` | OOP colon syntax not inlineable          |
| Recursive function      | Split base/recursive cases    | Recursion prevents inlining              |
| Deep upvalue captures   | Pass as parameters            | Reduces closure cost                     |


**Deprecated patterns to detect and replace:**


| Deprecated                | Replacement                | Impact                                                                                             |
| ------------------------- | -------------------------- | -------------------------------------------------------------------------------------------------- |
| `getfenv()` / `setfenv()` | `debug.info(i, "snl")`     | Disables ALL optimizations (imports, fastcalls, inlining). Even read-only `getfenv()` deoptimizes. |
| `loadstring()`            | Restructure with `require` | Marks environment "impure", disables import optimization                                           |
| `table.getn(t)`           | `#t`                       | Slower                                                                                             |
| `table.foreach(t, f)`     | `for k, v in t do`         | Slower                                                                                             |
| `table.foreachi(t, f)`    | `for i, v in t do`         | Slower                                                                                             |
| `wait()`                  | `task.wait()`              | Deprecated Roblox API                                                                              |
| `obj.Method(obj, ...)`    | `obj:Method(...)`          | Misses fast method call instruction                                                                |
| `string:method()`         | `string.method(s)`         | `string.byte(s)` is faster than `s:byte()` for builtins                                            |


**Lint rules to check (via `luau-analyze`):**

The command should flag these lint warnings as optimization opportunities:


| Lint | Name               | Optimization impact                                                                              |
| ---- | ------------------ | ------------------------------------------------------------------------------------------------ |
| 10   | BuiltinGlobalWrite | Overwriting builtins disables fastcall optimization                                              |
| 22   | DeprecatedApi      | Deprecated APIs may have performance/correctness issues                                          |
| 23   | TableOperations    | `table.insert(t, 0, x)` (wrong index), `table.insert(t, #t+1, x)` (redundant), `#` on non-arrays |
| 3    | GlobalUsedAsLocal  | Global used in one function -- should be local for performance                                   |
| 7    | LocalUnused        | Dead locals (cleanup candidate)                                                                  |
| 12   | UnreachableCode    | Dead code (removal candidate)                                                                    |
| 25   | MisleadingAndOr    | `a and false or c` bugs -- use `if a then false else c`                                          |


### CLI Tool Reference

All four tools live at `C:\Program Files\Atlas\` and are in PATH.

#### `luau-compile` -- Static bytecode/codegen analysis (primary tool)

**Modes** (mutually exclusive, controls output format):

- `binary` -- Raw bytecode blob (default, not human-readable)
- `text` -- Human-readable bytecode listing with source line annotations. Shows every opcode, register, constant, and jump label per function.
- `remarks` -- Original source with compiler remarks injected as `-- remark:` comments. Shows inlining success/failure (with cost/profit), allocation tracking (`table hash N`, `table array N`). **Single most useful mode for optimization.**
- `codegen` -- Native assembly output. Shows full register allocation, memory ops, branch structure, and outlined helpers. Requires `--target`.

**Options:**

- `-O<n>` -- Optimization level 0-2 (default 1). `**-O2` enables automatic inlining.** Always use `-O2` to see production compiler behavior. `-O0` useful as baseline comparison.
- `-g<n>` -- Debug info level 0-2 (default 1). `-g0` strips debug info for smallest bytecode.
- `--target=<arch>` -- Architecture for codegen mode: `x64` (Linux/Mac), `x64_ms` (Windows x64 calling convention), `a64` (ARM64), `a64_nf` (ARM64 no-FP). **Use `x64_ms` on Windows.**
- `--record-stats=<granularity>` -- JSON compilation stats at `total`, `file`, or `function` granularity. At `function` level shows: `bytecodeInstructionCount`, codegen size, parse/compile time, and `lowerStats` (spillsToSlot, spillsToRestore, maxSpillSlotsUsed, skippedFunctions, blocksPreOpt, blocksPostOpt, regAllocErrors, loweringErrors).
- `--bytecode-summary` -- Bytecode operation distribution. Requires `--record-stats=function`. Shows which opcodes dominate.
- `--stats-file=<filename>` -- Where to write stats JSON (default `stats.json`). Useful when analyzing multiple files.
- `--timetrace` -- Writes `trace.json` with compiler phase timings (Chrome `chrome://tracing` compatible).
- `--vector-lib=<name>` -- Library name for vector ops (e.g., `"Vector3"`). Tells compiler about vector type for better optimization.
- `--vector-ctor=<name>` -- Constructor function name (e.g., `"new"`). Paired with `--vector-lib` for constant-folding.
- `--vector-type=<name>` -- Vector type name. Completes the vector triplet for full vector optimization.

**Key invocations for the command (always include Roblox vector flags):**

```
luau-compile --remarks -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
luau-compile --text -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
luau-compile --codegen --target=x64_ms --record-stats=function -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>   # Windows
luau-compile --codegen --target=x64 --record-stats=function -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>      # Linux/Mac
luau-compile --bytecode-summary --record-stats=function -O2 --vector-lib=Vector3 --vector-ctor=new --vector-type=Vector3 <file>
```

#### `luau-analyze` -- Type checking and linting

**Modes:**

- *(omitted)* -- Default: typecheck + lint. Reports type errors and lint warnings to stderr.
- `--annotate` -- Outputs source with all inferred types written inline. Every variable, parameter, and return gets its inferred type. **Critical for finding `any` types that hurt native codegen.**

**Options:**

- `--mode=strict` -- Forces `--!strict` type checking even if file lacks the directive. Catches type errors silent in nonstrict mode.
- `--formatter=plain` -- Luacheck-compatible output (machine-parseable, good for counting errors).
- `--formatter=gnu` -- GNU-compatible output (`file:line:col: message`), grep-friendly.
- `--timetrace` -- Writes `trace.json` with analysis phase timings.

**Key invocations:**

```
luau-analyze --mode=strict <file>        # Type errors + lint warnings
luau-analyze --annotate <file>           # Inferred types (find 'any' types)
```

#### `luau` -- Runtime execution, profiling, and coverage

**Limited utility for Roblox scripts.** Most Roblox scripts depend on engine APIs (`game`, `workspace`, `Players`, etc.) that the standalone `luau` runtime does not provide. This tool is only usable for pure-logic library modules with zero Roblox dependencies.

**Options:**

- `-O<n>` -- Optimization level 0-2 (default 1). Same as compile.
- `-g<n>` -- Debug level 0-2 (default 1). Same as compile.
- `--codegen` -- Execute using native code generation.
- `--profile[=N]` -- CPU profiling at N Hz sampling (default 10000). Outputs `profile.out`.
- `--coverage` -- Code coverage collection. Outputs `coverage.out`. Can identify dead code paths.
- `--timetrace` -- Compiler time tracing to `trace.json`.
- `-i, --interactive` -- REPL after script execution.
- `-a, --program-args` -- Pass arguments to the Luau program.

**Key invocations (only for pure-logic modules without Roblox API dependencies):**

```
luau --codegen --profile -O2 <file>      # Profile with native codegen
luau --coverage -O2 <file>               # Dead code detection
```

**For Roblox scripts:** Use Roblox Studio's Script Profiler and `debug.dumpcodesize()` instead. The command should note this when the target file uses Roblox APIs.

#### `luau-ast` -- AST dump

**No flags.** Takes a single file, outputs full JSON AST.

- Node types, source locations, variable scopes, type annotations
- Programmatically parseable for: function nesting depth, function body size (monolith detection), closure captures (upvalue analysis), table construction patterns, loop-invariant expression detection

**Key invocation:**

```
luau-ast <file>                          # Full JSON AST
```

### Reference Documentation

The command will reference these docs (read during execution if needed for detail):

**Core optimization:**

- `.cursor/rules/luau.mdc` -- Luau language rules, optimization tips, performance patterns
- `.cursor/rules/luau/function-inlining.md` -- Inlining RFC (no user `@inline`, automatic at `-O2`, requirements for inlineability)
- `.cursor/rules/luau/syntax-attribute-functions-native.md` -- `@native` per-function attribute RFC
- `.cursor/rules/roblox/en-us/luau/native-code-gen.md` -- Native codegen docs (size limits, type annotation impact, gotchas)

**Type system (old typechecker only):**

- `.cursor/rules/roblox/en-us/luau/type-checking.md` -- Type annotation syntax and modes
- `.cursor/rules/luau/generic-functions.md` -- Generic function syntax
- `.cursor/rules/luau/never-and-unknown-types.md` -- `never`/`unknown` for exhaustiveness
- `.cursor/rules/luau/syntax-type-ascription.md` -- `::` cast operator

**Library optimizations:**

- `.cursor/rules/luau/function-table-create-find.md` -- `table.create`, `table.find`
- `.cursor/rules/luau/function-table-clear.md` -- `table.clear` for reuse
- `.cursor/rules/luau/function-table-clone.md` -- `table.clone` vs manual copy
- `.cursor/rules/luau/function-table-freeze.md` -- `table.freeze` for readonly
- `.cursor/rules/luau/function-math-lerp.md` -- `math.lerp` properties and correctness
- `.cursor/rules/luau/function-bit32-byteswap.md` -- CPU-level byte swap
- `.cursor/rules/luau/function-bit32-countlz-countrz.md` -- CPU-level bit counting
- `.cursor/rules/luau/function-buffer-bits.md` -- Buffer bit operations
- `.cursor/rules/luau/type-byte-buffer.md` -- `buffer` type for binary data
- `.cursor/rules/luau/vector-library.md` -- Vector library and fastcalls
- `.cursor/rules/luau/function-string-pack-unpack.md` -- Binary string operations
- `.cursor/rules/luau/syntax-floor-division-operator.md` -- `//` dedicated opcode
- `.cursor/rules/luau/syntax-compound-assignment.md` -- `+=` single LHS evaluation

**Deprecation/avoidance:**

- `.cursor/rules/luau/deprecate-getfenv-setfenv.md` -- Why fenv disables optimizations
- `.cursor/rules/luau/deprecate-table-getn-foreach.md` -- Deprecated table functions
- `.cursor/rules/luau/generalized-iteration.md` -- Modern iteration patterns

**Roblox performance (always applicable):**

- `.cursor/rules/roblox/en-us/performance-optimization/improve.md` -- Script computation, memory, physics, rendering, networking, assets
- `.cursor/rules/roblox/en-us/performance-optimization/design.md` -- Event-driven patterns, frame budgets (~16.67ms at 60 FPS), multithreading
- `.cursor/rules/roblox/en-us/luau/variables.md` -- Local vs global: "global in a time-critical loop can make it perform more than 10% slower"
- `.cursor/rules/roblox/en-us/luau/scope.md` -- Scope performance implications
- `.cursor/rules/roblox/en-us/luau/native-code-gen.md` -- `@native` attribute, Vector3 annotation impact, size limits, `debug.dumpcodesize()`

