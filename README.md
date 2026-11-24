# Velo Programming Language

**Velo** is a statically-typed, native-compiled systems and application programming language with predictable Python-inspired syntax, zero garbage-collector pauses, strict `noalias` memory semantics, and direct Win32/C interoperability.

## Compiler Architecture

Velo is built in Rust with a full multi-stage compiler pipeline:
1. **Indentation Lexer** (`src/lexer.rs`): Python-style `INDENT`/`DEDENT` tokenization, line continuations, f-strings (`f"..."`), C strings (`c"..."`), and compile-time UTF-16 wide strings (`w"..."`).
2. **Recursive Descent + Pratt Parser** (`src/parser.rs`): Predictable, ambiguity-free syntax.
3. **Static Type Checker & Bounds Prover** (`src/checker.rs`, `src/types.rs`): Bidirectional type inference, C-ABI struct layouts, mutability safety (`let` vs `var`, `var self`), compile-time constant evaluation, and loop bounds-check elimination.
4. **Typed SSA/CFG Mid-Level IR & Optimizer** (`src/ir.rs`, `src/ir_lower.rs`, `src/ir_opt.rs`): Constant folding, algebraic simplification, block-local CSE, store-to-load forwarding, small-function inlining, unreachable CFG pruning, and dead code elimination.
5. **Dual Native Execution Engines**:
   - **Native Machine Code & JIT (`--backend=native` / `--jit`)** (`src/backend_native.rs`): Compiles Velo IR directly to x86_64 machine code via Cranelift (AOT Windows COFF `.obj` linked by `lld-link`, or in-memory JIT execution).
   - **Max-Optimization Vectorizing Backend (`--backend=max` / `--release`)** (`src/backend_c.rs`): Lowers Velo IR with strict `noalias` (`__restrict`) pointer guarantees and compiles with AVX2/LTCG optimizations.

## Quick Commands

```powershell
# Run a program using the native compiler
velo run examples\hello.velo

# Run directly in memory with JIT
velo run examples\hello.velo --jit

# Compile with maximum AVX2 optimization
velo build examples\benchmark_matmul.velo --release -o matmul.exe

# Type-check only
velo check examples\tictactoe_gui.velo

# Build the native GUI Tic-Tac-Toe app
velo build examples\tictactoe_gui.velo --backend=native -o tictactoe.exe
```

## Included Examples

- [examples/hello.velo](examples/hello.velo): Basic syntax, arrays, loops, f-strings.
- [examples/benchmark_sieve.velo](examples/benchmark_sieve.velo): Sieve of Eratosthenes up to 10,000,000.
- [examples/benchmark_matmul.velo](examples/benchmark_matmul.velo): 512x512 dense matrix multiplication.
- [examples/tictactoe_gui.velo](examples/tictactoe_gui.velo): Double-buffered Win32 GUI Tic-Tac-Toe with unbeatable alpha-beta Minimax AI, 2-player mode, live scoreboard, hover previews, and responsive resizing.
