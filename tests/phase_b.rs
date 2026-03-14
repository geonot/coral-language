//! Phase B feature tests
//!
//! Comprehensive tests for Phase B features:
//! - TS-9: Exhaustiveness checking (now warnings, not errors)
//! - SL-14: Error propagation operator
//! - SL-13: TCP networking builtins compile
//! - SL-8/9/12: JSON, time, encoding builtins
//! - Codegen split verification (all paths still work)
//! - Additional E2E coverage for stores, actors, match, pipelines

use coralc::compiler::Compiler;

fn compile(source: &str) -> Result<String, String> {
    let compiler = Compiler;
    compiler
        .compile_to_ir(source)
        .map_err(|e| format!("{:?}", e))
}

fn compile_with_warnings(source: &str) -> Result<(String, Vec<String>), String> {
    let compiler = Compiler;
    compiler
        .compile_to_ir_with_warnings(source)
        .map(|(ir, warnings)| (ir, warnings.iter().map(|w| w.message.clone()).collect()))
        .map_err(|e| format!("{:?}", e))
}

// ========== TS-9: Exhaustiveness Checking (Warnings) ==========

#[test]
fn ts9_exhaustive_match_no_warnings() {
    let source = r#"
enum Shape
  Circle(r)
  Square(side)
  Triangle(a, b, c)

*area(s)
  match s
    Circle(r) ? r
    Square(side) ? side
    Triangle(a, b, c) ? a
"#;
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let exhaust_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("non-exhaustive"))
        .collect();
    assert!(
        exhaust_warnings.is_empty(),
        "All variants covered, no warnings expected. Got: {:?}",
        exhaust_warnings
    );
}

#[test]
fn ts9_missing_variant_is_warning_not_error() {
    let source = r#"
enum Option
  Some(value)
  None

*unwrap(opt)
  match opt
    Some(v) ? v
"#;
    // Should compile (warning, not error)
    let (_, warnings) =
        compile_with_warnings(source).expect("Should compile with warning, not error");
    let has_warning = warnings.iter().any(|w| w.contains("non-exhaustive"));
    assert!(
        has_warning,
        "Expected non-exhaustive warning, got warnings: {:?}",
        warnings
    );
}

#[test]
fn ts9_default_branch_suppresses_warning() {
    let source = r#"
enum Color
  Red
  Green
  Blue

*name(c)
  match c
    Red ? "red"
    ! "other"
"#;
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let exhaust_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("non-exhaustive"))
        .collect();
    assert!(
        exhaust_warnings.is_empty(),
        "Default branch should satisfy exhaustiveness. Got: {:?}",
        exhaust_warnings
    );
}

#[test]
fn ts9_identifier_catchall_suppresses_warning() {
    let source = r#"
enum Option
  Some(value)
  None

*describe(opt)
  match opt
    other ? 0
"#;
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let exhaust_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("non-exhaustive"))
        .collect();
    assert!(
        exhaust_warnings.is_empty(),
        "Identifier catch-all should satisfy exhaustiveness"
    );
}

#[test]
fn ts9_multiple_missing_variants_warning() {
    let source = r#"
enum Direction
  North
  South
  East
  West

*describe(d)
  match d
    North ? "up"
"#;
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let warning = warnings.iter().find(|w| w.contains("non-exhaustive"));
    assert!(warning.is_some(), "Expected non-exhaustive warning");
    let w = warning.unwrap();
    assert!(
        w.contains("South") || w.contains("East") || w.contains("West"),
        "Warning should mention missing variants, got: {}",
        w
    );
}

#[test]
fn ts9_nested_adt_exhaustiveness() {
    let source = r#"
enum Option
  Some(value)
  None

*describe(opt)
  match opt
    Some(Some(inner)) ? inner
    Some(None) ? 0
    None ? 0
"#;
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let exhaust_warnings: Vec<_> = warnings
        .iter()
        .filter(|w| w.contains("non-exhaustive"))
        .collect();
    assert!(
        exhaust_warnings.is_empty(),
        "Nested exhaustive match should have no warnings. Got: {:?}",
        exhaust_warnings
    );
}

#[test]
fn ts9_nested_adt_missing_inner_variant() {
    let source = r#"
enum Option
  Some(value)
  None

*describe(opt)
  match opt
    Some(Some(inner)) ? inner
    None ? 0
"#;
    // Some(None) is missing - should warn about nested non-exhaustiveness
    let (_, warnings) = compile_with_warnings(source).expect("Should compile");
    let has_warning = warnings
        .iter()
        .any(|w| w.contains("non-exhaustive") || w.contains("None"));
    assert!(
        has_warning,
        "Expected warning about missing Some(None) pattern. Got: {:?}",
        warnings
    );
}

// ========== SL-14: Error Propagation Operator ==========

#[test]
fn sl14_error_propagate_compiles() {
    let source = r#"
*risky()
    err NotFound

*safe()
    result is risky() ! return err
    result
"#;
    compile(source).expect("Error propagation should compile");
}

#[test]
fn sl14_error_value_creation_compiles() {
    let source = r#"
err DatabaseError
    err ConnectionFailed
    err QueryFailed

*fail()
    err DatabaseError
"#;
    compile(source).expect("Error value creation should compile");
}

#[test]
fn sl14_is_err_is_ok_compile() {
    let source = r#"
*check(val)
    is_err(val) ? log("error") ! log("ok")
    is_ok(val) ? log("ok") ! log("error")
"#;
    compile(source).expect("is_err/is_ok should compile");
}

// ========== SL-13: TCP Networking Builtins Compile ==========

#[test]
fn sl13_tcp_listen_compiles() {
    let source = r#"
listener is tcp_listen("0.0.0.0", 8080)
log(listener)
"#;
    compile(source).expect("tcp_listen should compile");
}

#[test]
fn sl13_tcp_connect_compiles() {
    let source = r#"
conn is tcp_connect("127.0.0.1", 80)
log(conn)
"#;
    compile(source).expect("tcp_connect should compile");
}

#[test]
fn sl13_tcp_read_write_compiles() {
    let source = r#"
conn is tcp_connect("127.0.0.1", 80)
tcp_write(conn, "GET / HTTP/1.0\r\n\r\n")
data is tcp_read(conn, 4096)
tcp_close(conn)
log(data)
"#;
    compile(source).expect("tcp_read/write/close should compile");
}

#[test]
fn sl13_tcp_accept_compiles() {
    let source = r#"
*serve(host, port)
    listener is tcp_listen(host, port)
    conn is tcp_accept(listener)
    data is tcp_read(conn, 1024)
    tcp_write(conn, "Response")
    tcp_close(conn)
"#;
    compile(source).expect("Full TCP server flow should compile");
}

// ========== SL-8/9/12: JSON, Time, Encoding ==========

#[test]
fn sl8_json_parse_compiles() {
    let source = r#"
data is json_parse("{\"key\": 42}")
log(data)
"#;
    compile(source).expect("json_parse should compile");
}

#[test]
fn sl8_json_serialize_compiles() {
    let source = r#"
m is map("key" is 42)
result is json_serialize(m)
log(result)
"#;
    compile(source).expect("json_serialize should compile");
}

#[test]
fn sl9_time_now_compiles() {
    let source = r#"
t is time_now()
log(t)
"#;
    compile(source).expect("time_now should compile");
}

#[test]
fn sl12_base64_encode_compiles() {
    let source = r#"
encoded is base64_encode("hello world")
log(encoded)
"#;
    compile(source).expect("base64_encode should compile");
}

#[test]
fn sl12_hex_encode_compiles() {
    let source = r#"
encoded is hex_encode("hello")
log(encoded)
"#;
    compile(source).expect("hex_encode should compile");
}

// ========== Codegen Split Verification ==========
// These tests verify all code paths still work after IQ-2 split

#[test]
fn codegen_split_builtins_still_work() {
    let source = r#"
log("test")
result is concat("hello", " world")
log(result)
"#;
    compile(source).expect("Basic builtins should work after split");
}

#[test]
fn codegen_split_member_calls_still_work() {
    let source = r#"
s is "hello world"
log(s.length)
parts is split(s, " ")
log(parts)
"#;
    compile(source).expect("Member calls should work after split");
}

#[test]
fn codegen_split_match_still_works() {
    let source = r#"
enum Result
  Ok(value)
  Error(msg)

*describe(r)
  match r
    Ok(v) ? v
    Error(msg) ? msg

x is Ok(42)
log(describe(x))
"#;
    compile(source).expect("Match expressions should work after split");
}

#[test]
fn codegen_split_stores_still_work() {
    let source = r#"
store Counter
  count ? 0
  *increment()
    self.count is self.count + 1
  *get_count()
    self.count

c is make_Counter()
c.increment()
log(c.get_count())
"#;
    compile(source).expect("Stores should work after split");
}

#[test]
fn codegen_split_closures_still_work() {
    let source = r#"
*apply(f, x)
    f(x)

double is *fn(n) n * 2
result is apply(double, 21)
log(result)
"#;
    compile(source).expect("Closures should work after split");
}

#[test]
fn codegen_split_enum_constructors_still_work() {
    let source = r#"
enum Option
  Some(value)
  None

x is Some(42)
y is None
log(x)
log(y)
"#;
    compile(source).expect("Enum constructors should work after split");
}

#[test]
fn codegen_split_io_calls_still_work() {
    let source = r#"
log("hello")
"#;
    compile(source).expect("IO calls should work after split");
}

#[test]
fn codegen_split_string_ops_still_work() {
    let source = r#"
s is "hello world"
upper is string_to_upper(s)
lower is string_to_lower(s)
trimmed is string_trim("  hi  ")
idx is string_index_of(s, "world")
log(upper)
log(lower)
log(trimmed)
log(idx)
"#;
    compile(source).expect("String ops should work after split");
}

#[test]
fn codegen_split_list_ops_still_work() {
    let source = r#"
*main()
    nums is [1, 2, 3, 4, 5]
    log(nums.length)
    log(nums[0])
    nums.push(6)
    log(nums.length)
"#;
    compile(source).expect("List ops should work after split");
}

#[test]
fn codegen_split_map_ops_still_work() {
    let source = r#"
m is map("a" is 1, "b" is 2)
keys is map_keys(m)
has is map_has_key(m, "a")
val is m.get("b")
log(keys)
log(has)
log(val)
"#;
    compile(source).expect("Map ops should work after split");
}

#[test]
fn codegen_split_math_ops_still_work() {
    let source = r#"
x is abs(0 - 5)
y is floor(3.7)
z is ceil(3.2)
w is sqrt(16)
log(x)
log(y)
log(z)
log(w)
"#;
    compile(source).expect("Math ops should work after split");
}

#[test]
fn codegen_split_bitwise_still_work() {
    let source = r#"
a is bit_and(15, 9)
b is bit_or(5, 3)
c is bit_xor(10, 6)
d is bit_not(0)
log(a)
log(b)
log(c)
log(d)
"#;
    compile(source).expect("Bitwise ops should work after split");
}

#[test]
fn codegen_split_error_ops_still_work() {
    let source = r#"
err MyError

e is err MyError
check is is_err(e)
log(check)
"#;
    compile(source).expect("Error ops should work after split");
}

#[test]
fn codegen_split_actor_ops_compile() {
    let source = r#"
actor Counter
    count is 0
    
    @increment(amount)
        self.count is self.count + amount
    @get_count()
        log(self.count)
"#;
    compile(source).expect("Actor definitions should work after split");
}

#[test]
fn codegen_split_pipeline_still_works() {
    let source = r#"
*double(x)
    x * 2
*add_one(x)
    x + 1

result is 5 ~ double ~ add_one
log(result)
"#;
    compile(source).expect("Pipeline should work after split");
}

#[test]
fn codegen_split_ternary_still_works() {
    let source = r#"
x is 10
result is x > 5 ? "big" ! "small"
log(result)
"#;
    compile(source).expect("Ternary should work after split");
}

#[test]
fn codegen_split_for_loop_still_works() {
    let source = r#"
*main()
    nums is [1, 2, 3]
    for n in nums
        log(n)
"#;
    compile(source).expect("For loops should work after split");
}

#[test]
fn codegen_split_while_loop_still_works() {
    let source = r#"
*main()
    i is 0
    while i < 5
        log(i)
        i is i + 1
"#;
    compile(source).expect("While loops should work after split");
}

// ========== Complex Integration Tests ==========

#[test]
fn integration_store_with_methods_and_match() {
    let source = r#"
enum Status
  Active
  Inactive
  Suspended

store User
  name ? "anonymous"
  status ? "active"
  
  *display()
    log(self.name)

u is make_User()
u.display()
"#;
    compile(source).expect("Store + enum integration should compile");
}

#[test]
fn integration_higher_order_functions() {
    let source = r#"
*apply_twice(f, x)
    f(f(x))

*double(n)
    n * 2

result is apply_twice(double, 3)
log(result)
"#;
    compile(source).expect("Higher-order functions should compile");
}

#[test]
fn integration_recursive_function() {
    let source = r#"
*factorial(n)
    n < 2 ? 1 ! n * factorial(n - 1)

result is factorial(10)
log(result)
"#;
    compile(source).expect("Recursive functions should compile");
}

#[test]
fn integration_nested_data_structures() {
    let source = r#"
matrix is [[1, 2, 3], [4, 5, 6], [7, 8, 9]]
log(matrix)
"#;
    compile(source).expect("Nested lists should compile");
}

#[test]
fn integration_map_of_lists() {
    let source = r#"
data is map("fruits" is ["apple", "banana"], "vegs" is ["carrot"])
log(data)
"#;
    compile(source).expect("Map of lists should compile");
}

#[test]
fn integration_complex_pipeline() {
    let source = r#"
*double(x)
    x * 2
*negate(x)
    0 - x
*add_hundred(x)
    x + 100

result is 5 ~ double ~ negate ~ add_hundred
log(result)
"#;
    compile(source).expect("Complex pipeline should compile");
}

#[test]
fn integration_process_and_env() {
    let source = r#"
args is process_args()
log(args)
home is env_get("HOME")
log(home)
"#;
    compile(source).expect("Process/env operations should compile");
}

#[test]
fn integration_bytes_operations() {
    let source = r#"
b is bytes_from_string("hello")
len is b.length
s is bytes_to_string(b)
log(len)
log(s)
"#;
    compile(source).expect("Bytes operations should compile");
}

#[test]
fn integration_char_operations() {
    let source = r#"
code is ord("A")
ch is chr(65)
log(code)
log(ch)
"#;
    compile(source).expect("Char operations should compile");
}

#[test]
fn integration_sort_operations() {
    let source = r#"
nums is [3, 1, 4, 1, 5]
sorted is sort_natural(nums)
log(sorted)
"#;
    compile(source).expect("Sort operations should compile");
}

#[test]
fn integration_type_of_operations() {
    let source = r#"
log(type_of(42))
log(type_of("hello"))
log(type_of(true))
log(type_of([1, 2]))
log(type_of(map("a" is 1)))
"#;
    compile(source).expect("type_of should compile");
}

#[test]
fn integration_error_hierarchy() {
    let source = r#"
err DatabaseError
    err ConnectionFailed
    err QueryFailed
        err Timeout
        err Syntax

*handle_error(e)
    name is error_name(e)
    log(name)
"#;
    compile(source).expect("Error hierarchies should compile");
}

#[test]
fn integration_trait_definition() {
    let source = r#"
trait Printable
    *display()
        log("default display")

store Widget with Printable
    name ? "widget"
    *display()
        log(self.name)

w is make_Widget()
w.display()
"#;
    compile(source).expect("Traits should compile");
}
