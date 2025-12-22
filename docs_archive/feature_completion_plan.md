# Feature Completion (Non-Actor) – Starter Checklist

- **Match patterns:** add string/bool/list pattern lowering; emit runtime pattern helpers; add parser snapshot tests for mixed patterns.
- **Stores/types:** lower store/type fields into LLVM structs; generate constructors and method dispatch tables; validate defaults.
- **Closures/ABI:** define closure env struct layout, invoke trampoline signature, and capture analysis hooks.
- **List/map HOFs:** runtime helpers (`map`, `filter`, `reduce`) calling closures; codegen lowers `$` placeholders into closures.
- **String/bytes slices:** ensure codegen lowers slices to runtime slice helpers; add bounds-check diagnostics.
- **Bitwise typing:** route bitwise ops through typed integer paths once type inference lands; fallback to runtime tag errors otherwise.
- **Module loader hardening:** snapshot layout tokens; reject mixed tabs/spaces with multi-error reporting; add fixtures.
