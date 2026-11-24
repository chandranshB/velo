use std::path::Path;
use std::process::Command;

fn velo_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_velo"))
}

fn run_velo_temp(name: &str, code: &str, backend: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join("velo_tests");
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join(format!("{name}.velo"));
    std::fs::write(&src, code).expect("write test velo file");

    let mut cmd = Command::new(velo_bin());
    cmd.arg("run").arg(&src).arg(format!("--backend={backend}"));
    let out = cmd.output().expect("execute velo");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
    )
}

#[test]
fn test_core_language_features_all_backends() {
    let code = r#"
import math

const OFFSET: i64 = 10

enum Op:
    Add = 1
    Mul = 2

struct Vec2:
    x: i32
    y: i32

    def len_sq(self) -> i32:
        return self.x * self.x + self.y * self.y

    def add_in_place(var self, dx: i32, dy: i32):
        self.x += dx
        self.y += dy

def apply(op: Op, a: i64, b: i64) -> i64:
    match op:
        case Op.Add:
            return a + b
        case Op.Mul:
            return a * b
        else:
            return 0

def sum_slice(xs: []i32) -> i64:
    var total: i64 = 0
    for i in 0..xs.len:
        total += xs[i] as i64
    return total

def main() -> i32:
    var v = Vec2(x: 3, y: 4)
    assert(v.len_sq() == 25)
    v.add_in_place(1, -2)
    assert(v.x == 4 and v.y == 2)

    let arr: [i32; 5] = [1, 2, 3, 4, 5]
    assert(sum_slice(arr) == 15)

    let res = apply(Op.Mul, 6, 7) + OFFSET
    let g = math.gcd(48, 18)
    let msg = f"res={res},gcd={g}"
    defer free(msg.ptr as ptr)
    print(msg)

    var d_sum = 0
    for i in 0..=4:
        d_sum += i
    assert(d_sum == 10)
    return 0
"#;

    for backend in ["native", "max", "jit"] {
        let (code_exit, stdout, stderr) = run_velo_temp("core_features", code, backend);
        assert_eq!(code_exit, 0, "backend {backend} failed:\nstderr={stderr}");
        assert_eq!(stdout.trim(), "res=52,gcd=6", "backend {backend} stdout mismatch");
    }
}

#[test]
fn test_defer_lifo_order() {
    let code = r#"
def test_defer() -> i32:
    let buf: *i32 = alloc(16)
    defer free(buf as ptr)
    defer print("second")
    defer print("first")
    buf[0] = 99
    return buf[0]

def main() -> i32:
    let v = test_defer()
    print(v)
    return 0
"#;
    for backend in ["native", "max", "jit"] {
        let (code_exit, stdout, stderr) = run_velo_temp("defer_order", code, backend);
        assert_eq!(code_exit, 0, "backend {backend} failed:\nstderr={stderr}");
        assert_eq!(stdout.trim(), "first\nsecond\n99");
    }
}

#[test]
fn test_function_pointers_and_callbacks() {
    let code = r#"
def square(x: i32) -> i32:
    return x * x

def apply_twice(f: fn(i32) -> i32, val: i32) -> i32:
    return f(f(val))

def main() -> i32:
    let r = apply_twice(square, 3)
    print(r)
    return 0
"#;
    for backend in ["native", "max", "jit"] {
        let (code_exit, stdout, stderr) = run_velo_temp("fn_ptrs", code, backend);
        assert_eq!(code_exit, 0, "backend {backend} failed:\nstderr={stderr}");
        assert_eq!(stdout.trim(), "81");
    }
}

#[test]
fn test_immutability_rejected_at_compile_time() {
    let code = r#"
def main() -> i32:
    let x = 10
    x = 20
    return 0
"#;
    let dir = std::env::temp_dir().join("velo_tests");
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("immut_err.velo");
    std::fs::write(&src, code).unwrap();

    let out = Command::new(velo_bin()).arg("check").arg(&src).output().unwrap();
    assert!(!out.status.success(), "expected compile error for mutating `let`");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot mutate immutable variable"), "stderr={stderr}");
}

#[test]
fn test_bounds_check_catches_oob() {
    let code = r#"
def get_item(i: i64) -> i64:
    let a = [10, 20, 30]
    return a[i]

def main() -> i32:
    print(get_item(5))
    return 0
"#;
    let (code_exit, _stdout, stderr) = run_velo_temp("oob", code, "native");
    assert_ne!(code_exit, 0);
    assert!(stderr.contains("out of bounds"), "stderr={stderr}");
}

#[test]
fn test_tictactoe_minimax_ai_unbeatable() {
    let code = r#"
const CELL_EMPTY: i32 = 0
const CELL_X: i32 = 1
const CELL_O: i32 = 2

def check_winner_pure(b: *i32) -> i32:
    if b[0] != 0 and b[0] == b[1] and b[1] == b[2]:
        return b[0]
    if b[3] != 0 and b[3] == b[4] and b[4] == b[5]:
        return b[3]
    if b[6] != 0 and b[6] == b[7] and b[7] == b[8]:
        return b[6]
    if b[0] != 0 and b[0] == b[3] and b[3] == b[6]:
        return b[0]
    if b[1] != 0 and b[1] == b[4] and b[4] == b[7]:
        return b[1]
    if b[2] != 0 and b[2] == b[5] and b[5] == b[8]:
        return b[2]
    if b[0] != 0 and b[0] == b[4] and b[4] == b[8]:
        return b[0]
    if b[2] != 0 and b[2] == b[4] and b[4] == b[6]:
        return b[2]
    return 0

def minimax(b: *i32, depth: i32, is_max: bool, alpha_in: i32, beta_in: i32) -> i32:
    let w = check_winner_pure(b)
    if w == CELL_O:
        return 100 - depth
    if w == CELL_X:
        return depth - 100
    var full = true
    for i in 0..9:
        if b[i] == CELL_EMPTY:
            full = false
            break
    if full:
        return 0
    var alpha = alpha_in
    var beta = beta_in
    if is_max:
        var best: i32 = -1000
        for i in 0..9:
            if b[i] == CELL_EMPTY:
                b[i] = CELL_O
                let score = minimax(b, depth + 1, false, alpha, beta)
                b[i] = CELL_EMPTY
                if score > best:
                    best = score
                if best > alpha:
                    alpha = best
                if beta <= alpha:
                    break
        return best
    else:
        var best: i32 = 1000
        for i in 0..9:
            if b[i] == CELL_EMPTY:
                b[i] = CELL_X
                let score = minimax(b, depth + 1, true, alpha, beta)
                b[i] = CELL_EMPTY
                if score < best:
                    best = score
                if best < beta:
                    beta = best
                if beta <= alpha:
                    break
        return best

def verify_never_loses(b: *i32) -> bool:
    let w = check_winner_pure(b)
    if w == CELL_X:
        return false
    if w == CELL_O:
        return true
    var full = true
    for i in 0..9:
        if b[i] == CELL_EMPTY:
            full = false
            break
    if full:
        return true
    # Try every possible human X move, then play AI O optimal move
    for i in 0..9:
        if b[i] == CELL_EMPTY:
            b[i] = CELL_X
            let w2 = check_winner_pure(b)
            if w2 == CELL_X:
                b[i] = CELL_EMPTY
                return false
            var best_sc: i32 = -10000
            var best_mv: i32 = -1
            for j in 0..9:
                if b[j] == CELL_EMPTY:
                    b[j] = CELL_O
                    let sc = minimax(b, 0, false, -10000, 10000)
                    b[j] = CELL_EMPTY
                    if sc > best_sc:
                        best_sc = sc
                        best_mv = j as i32
            if best_mv >= 0:
                b[best_mv] = CELL_O
                let ok = verify_never_loses(b)
                b[best_mv] = CELL_EMPTY
                if not ok:
                    b[i] = CELL_EMPTY
                    return false
            b[i] = CELL_EMPTY
    return true

def main() -> i32:
    var board: [i32; 9] = [0; 9]
    assert(verify_never_loses(&board[0]))
    print("UNBEATABLE_OK")
    return 0
"#;
    for backend in ["native", "max"] {
        let (exit_code, stdout, stderr) = run_velo_temp("ai_unbeatable", code, backend);
        assert_eq!(exit_code, 0, "backend {backend} failed:\nstderr={stderr}");
        assert_eq!(stdout.trim(), "UNBEATABLE_OK");
    }
}

#[test]
fn test_tictactoe_gui_builds_both_backends() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gui_src = repo_root.join("examples").join("tictactoe_gui.velo");
    let out_dir = std::env::temp_dir().join("velo_tests");
    let _ = std::fs::create_dir_all(&out_dir);

    for backend in ["native", "max"] {
        let exe = out_dir.join(format!("ttt_{backend}.exe"));
        let out = Command::new(velo_bin())
            .arg("build")
            .arg(&gui_src)
            .arg(format!("--backend={backend}"))
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "GUI build failed for {backend}:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(exe.exists());
    }
}
