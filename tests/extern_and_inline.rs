use coralc::diagnostics::Stage;
use coralc::Compiler;
use std::sync::{Mutex, OnceLock};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, prev }
    }

    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(val) = &self.prev {
            unsafe { std::env::set_var(self.key, val) };
        } else {
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

#[test]
fn lowers_typed_extern_memory_intrinsics() {
    let source = r"extern fn coral_malloc(size: usize) : usize
extern fn coral_store_u64(p: usize, value: u64)
extern fn coral_load_u64(p: usize) : u64

*main()
    p is coral_malloc(8)
    coral_store_u64(p, 42)
    coral_load_u64(p)
";
    let compiler = Compiler;
    let ir = compiler
        .compile_to_ir(source)
        .expect("extern memory intrinsics should lower");
    assert!(ir.contains("declare i64 @coral_malloc"), "malloc extern should be declared as i64->i64");
    assert!(ir.contains("declare void @coral_store_u64(i64, i64)"), "store_u64 extern should take ptr,value as i64");
    assert!(ir.contains("declare i64 @coral_load_u64(i64)"), "load_u64 extern should return i64");
}

#[test]
fn inline_asm_denied_by_default() {
    let source = "*main()\n    asm(\"nop\")\n";
    let _lock = env_lock().lock().unwrap();
    let _guard = EnvGuard::unset("CORAL_INLINE_ASM");
    let compiler = Compiler;
    let err = compiler
        .compile_to_ir(source)
        .expect_err("inline asm should fail without feature flag");
    assert_eq!(err.stage, Stage::Codegen);
    assert!(err.diagnostic.message.contains("inline asm not supported"));
}

#[test]
fn inline_asm_noop_when_allowed() {
    let source = "*main()\n    asm(\"nop\")\n";
    let _lock = env_lock().lock().unwrap();
    let _guard = EnvGuard::set("CORAL_INLINE_ASM", "allow-noop");
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect("inline asm should noop when allowed");
    assert!(ir.contains("define double @__user_main"));
}

#[test]
fn inline_asm_emits_when_enabled() {
    let source = "*main()\n    asm(\"nop\")\n";
    let _lock = env_lock().lock().unwrap();
    let _guard = EnvGuard::set("CORAL_INLINE_ASM", "emit");
    let compiler = Compiler;
    let ir = compiler.compile_to_ir(source).expect("inline asm should compile when enabled");
    assert!(
        ir.contains("asm") && ir.contains("nop"),
        "inline asm should be present in IR: {ir}"
    );
}
