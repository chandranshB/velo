# The Velo Programming Language (v0.1)

**Velo** is a statically-typed, ahead-of-time and JIT-compiled systems & application language with clean, predictable Python-inspired syntax and zero-overhead native execution.

## Core Design Principles
1. **Pythonic Clarity, Zero Magic:** Indentation-delimited blocks, clean type annotations, `let` (immutable) and `var` (mutable) bindings, `struct` methods, `enum`, `match`, `defer`, and readable string formatting.
2. **Predictable Performance:** Static types with local type inference, contiguous C-ABI value structs, stack-allocated fixed arrays, no garbage collector pauses, and strict `noalias` pointer/slice semantics that enable aggressive SIMD vectorization.
3. **Seamless Native Interop:** First-class `extern def` and `link` directives give direct, zero-overhead access to Windows APIs (`user32`, `gdi32`, `kernel32`) and any native C library.
4. **Safety Where It Doesn't Cost Speed:** Immutability by default (`let`), explicit casts (`as`), compile-time constant folding, and loop bounds verification that eliminates redundant runtime bounds checks.

## Quick Syntax Tour

```python
link "user32"

extern def MessageBoxA(hwnd: ptr, text: cstr, caption: cstr, utype: u32) -> i32

const BOARD_LEN = 9

enum Cell:
    Empty = 0
    X = 1
    O = 2

struct Board:
    cells: [i32; 9]

    def clear(var self):
        for i in 0..9:
            self.cells[i] = Cell.Empty as i32

    def winner(self) -> i32:
        wins := [
            [0, 1, 2], [3, 4, 5], [6, 7, 8],
            [0, 3, 6], [1, 4, 7], [2, 5, 8],
            [0, 4, 8], [2, 4, 6]
        ]
        for w in wins:
            a := self.cells[w[0]]
            if a != 0 and a == self.cells[w[1]] and a == self.cells[w[2]]:
                return a
        return 0

def main() -> i32:
    var b = Board(cells: [0; 9])
    b.cells[0] = Cell.X as i32
    b.cells[1] = Cell.X as i32
    b.cells[2] = Cell.X as i32
    print("Winner: {b.winner()}")
    return 0
```
