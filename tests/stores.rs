//! PS-8 — Store E2E Tests
//!
//! Full CRUD lifecycle tests for Coral stores:
//! 1. Create store, access field defaults
//! 2. Update fields via methods, verify changes
//! 3. Store methods, traits, complex interactions
//! 4. Multiple stores, passing stores to functions
//! 5. Store with list fields and nested operations

use std::process::Command;

fn run_coral(source: &str) -> (String, String, i32) {
    let compiler = coralc::compiler::Compiler;
    let ir = match compiler.compile_to_ir(source) {
        Ok(ir) => ir,
        Err(e) => panic!("Compilation failed: {:?}", e),
    };
    let tmp_dir = std::env::temp_dir().join(format!("coral_store_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let ir_path = tmp_dir.join(format!("test_{:x}.ll",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::write(&ir_path, &ir).unwrap();
    let runtime_path = std::env::current_dir()
        .unwrap()
        .join("target/debug/libruntime.so");
    let output = Command::new("lli")
        .arg("-load")
        .arg(&runtime_path)
        .arg(&ir_path)
        .output()
        .expect("Failed to run lli");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn assert_output(source: &str, expected: &[&str]) {
    let (stdout, stderr, code) = run_coral(source);
    let expected_out = expected.join("\n") + "\n";
    assert_eq!(
        stdout, expected_out,
        "\n--- STDOUT ---\n{stdout}\n--- STDERR ---\n{stderr}\n--- EXIT CODE: {code} ---\n"
    );
}

fn compile(source: &str) -> Result<String, String> {
    let compiler = coralc::compiler::Compiler;
    compiler.compile_to_ir(source).map_err(|e| format!("{:?}", e))
}

// ═══════════════════════════════════════════════════════════════════════
// 1. STORE CREATION & FIELD ACCESS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_create_with_defaults() {
    assert_output(r#"
store Config
    host ? "localhost"
    port ? 8080
    debug ? false

*main()
    c is make_Config()
    log(c.host)
    log(c.port)
    log(c.debug)
"#, &["localhost", "8080", "false"]);
}

#[test]
fn store_create_multiple_instances() {
    assert_output(r#"
store Point
    x ? 0
    y ? 0
    *set_x(val)
        self.x is val
    *set_y(val)
        self.y is val

*main()
    a is make_Point()
    b is make_Point()
    a.set_x(10)
    b.set_x(20)
    log(a.x)
    log(b.x)
"#, &["10", "20"]);
}

#[test]
fn store_numeric_defaults() {
    assert_output(r#"
store Stats
    n ? 0
    sum ? 0
    avg ? 0

*main()
    s is make_Stats()
    log(s.n)
    log(s.sum)
    log(s.avg)
"#, &["0", "0", "0"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 2. FIELD UPDATES VIA METHODS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_update_numeric_field() {
    assert_output(r#"
store Counter
    val ? 0
    *set_val(v)
        self.val is v
    *increment()
        self.val is self.val + 1

*main()
    c is make_Counter()
    log(c.val)
    c.set_val(42)
    log(c.val)
    c.increment()
    log(c.val)
"#, &["0", "42", "43"]);
}

#[test]
fn store_update_string_field() {
    assert_output(r#"
store User
    name ? "anonymous"
    email ? ""
    *set_name(n)
        self.name is n
    *set_email(e)
        self.email is e

*main()
    u is make_User()
    log(u.name)
    u.set_name("Alice")
    u.set_email("alice@example.com")
    log(u.name)
    log(u.email)
"#, &["anonymous", "Alice", "alice@example.com"]);
}

#[test]
fn store_update_boolean_field() {
    assert_output(r#"
store Toggle
    active ? false
    *activate()
        self.active is true

*main()
    t is make_Toggle()
    log(t.active)
    t.activate()
    log(t.active)
"#, &["false", "true"]);
}

#[test]
fn store_multiple_field_updates() {
    assert_output(r#"
store Rect
    x ? 0
    y ? 0
    w ? 100
    h ? 50
    *move_to(nx, ny)
        self.x is nx
        self.y is ny
    *resize(nw, nh)
        self.w is nw
        self.h is nh

*main()
    r is make_Rect()
    r.move_to(10, 20)
    r.resize(200, 150)
    log(r.x)
    log(r.y)
    log(r.w)
    log(r.h)
"#, &["10", "20", "200", "150"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 3. STORE METHODS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_method_reads_self() {
    assert_output(r#"
store Greeter
    name ? "World"
    *set_name(n)
        self.name is n
    *greet()
        log("Hello, " + self.name + "!")

*main()
    g is make_Greeter()
    g.greet()
    g.set_name("Coral")
    g.greet()
"#, &["Hello, World!", "Hello, Coral!"]);
}

#[test]
fn store_method_mutates_self() {
    assert_output(r#"
store Accumulator
    sum ? 0
    *add(n)
        self.sum is self.sum + n
    *get_total()
        return self.sum

*main()
    a is make_Accumulator()
    a.add(10)
    a.add(20)
    a.add(30)
    log(a.get_total())
"#, &["60"]);
}

#[test]
fn store_method_with_return_value() {
    assert_output(r#"
store Stack
    items ? []
    *push_item(val)
        self.items.push(val)
    *size()
        return self.items.length()

*main()
    s is make_Stack()
    s.push_item("a")
    s.push_item("b")
    s.push_item("c")
    log(s.size())
"#, &["3"]);
}

#[test]
fn store_method_with_conditional_logic() {
    assert_output(r#"
store BoundedCounter
    val ? 0
    limit ? 10
    *increment()
        if self.val < self.limit
            self.val is self.val + 1
    *get_val()
        return self.val

*main()
    bc is make_BoundedCounter()
    i is 0
    while i < 15
        bc.increment()
        i is i + 1
    log(bc.get_val())
"#, &["10"]);
}

#[test]
fn store_method_calls_another_method() {
    assert_output(r#"
store Calculator
    result ? 0
    *reset()
        self.result is 0
    *add(n)
        self.result is self.result + n
    *multiply(n)
        self.result is self.result * n
    *compute()
        self.reset()
        self.add(5)
        self.multiply(3)
        return self.result

*main()
    c is make_Calculator()
    log(c.compute())
"#, &["15"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 4. STORE WITH TRAITS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_implements_trait() {
    assert_output(r#"
trait Describable
    *describe()
        log("unknown")

store Dog with Describable
    breed ? "mutt"
    *set_breed(b)
        self.breed is b
    *describe()
        log("Dog: " + self.breed)

*main()
    d is make_Dog()
    d.describe()
    d.set_breed("labrador")
    d.describe()
"#, &["Dog: mutt", "Dog: labrador"]);
}

#[test]
fn store_trait_multiple_methods() {
    assert_output(r#"
trait Shape
    *area()
        return 0
    *label()
        return "shape"

store Circle with Shape
    radius ? 1
    *set_radius(r)
        self.radius is r
    *area()
        return 3 * self.radius * self.radius
    *label()
        return "circle"

*main()
    c is make_Circle()
    log(c.label())
    log(c.area())
    c.set_radius(5)
    log(c.area())
"#, &["circle", "3", "75"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 5. PASSING STORES TO FUNCTIONS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_passed_to_function() {
    assert_output(r#"
store Account
    balance ? 0
    *deposit(amount)
        self.balance is self.balance + amount
    *get_balance()
        return self.balance

*process_deposits(account, amounts)
    i is 0
    while i < amounts.length()
        account.deposit(amounts[i])
        i is i + 1

*main()
    acc is make_Account()
    process_deposits(acc, [100, 200, 50])
    log(acc.get_balance())
"#, &["350"]);
}

#[test]
fn store_returned_from_function() {
    assert_output(r#"
store Pair
    first ? ""
    second ? ""
    *set_first(v)
        self.first is v
    *set_second(v)
        self.second is v

*make_pair(a, b)
    p is make_Pair()
    p.set_first(a)
    p.set_second(b)
    return p

*main()
    p is make_pair("hello", "world")
    log(p.first)
    log(p.second)
"#, &["hello", "world"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 6. STORE WITH LIST/MAP FIELDS
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_list_field_operations() {
    assert_output(r#"
store TodoList
    items ? []
    *add(item)
        self.items.push(item)
    *how_many()
        return self.items.length()

*main()
    todos is make_TodoList()
    todos.add("Buy milk")
    todos.add("Write code")
    todos.add("Walk dog")
    log(todos.how_many())
"#, &["3"]);
}

#[test]
fn store_nested_field_access() {
    assert_output(r#"
store Registry
    entries ? []
    *register(label)
        self.entries.push(label)
    *list_all()
        i is 0
        while i < self.entries.length()
            log(self.entries[i])
            i is i + 1

*main()
    r is make_Registry()
    r.register("alpha")
    r.register("beta")
    r.register("gamma")
    r.list_all()
"#, &["alpha", "beta", "gamma"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 7. MULTIPLE STORES INTERACTING
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn multiple_stores_interaction() {
    assert_output(r#"
store Wallet
    balance ? 0
    *deposit(n)
        self.balance is self.balance + n
    *withdraw(n)
        self.balance is self.balance - n
    *get_balance()
        return self.balance

*transfer(from, to, amount)
    from.withdraw(amount)
    to.deposit(amount)

*main()
    alice is make_Wallet()
    bob is make_Wallet()
    alice.deposit(1000)
    bob.deposit(500)
    transfer(alice, bob, 300)
    log(alice.get_balance())
    log(bob.get_balance())
"#, &["700", "800"]);
}

#[test]
fn store_loop_accumulation() {
    assert_output(r#"
store RunningAvg
    sum ? 0
    n ? 0
    *add_sample(val)
        self.sum is self.sum + val
        self.n is self.n + 1
    *get_average()
        if self.n > 0
            return self.sum / self.n
        return 0

*main()
    avg is make_RunningAvg()
    avg.add_sample(10)
    avg.add_sample(20)
    avg.add_sample(30)
    avg.add_sample(40)
    log(avg.get_average())
"#, &["25"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 8. STORE FIELD RESET / REINITIALIZE
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_field_reassign_cycle() {
    assert_output(r#"
store State
    value ? 0
    *update(v)
        self.value is v
    *read()
        return self.value

*main()
    s is make_State()
    s.update(1)
    log(s.read())
    s.update(2)
    log(s.read())
    s.update(3)
    log(s.read())
"#, &["1", "2", "3"]);
}

#[test]
fn store_complex_lifecycle() {
    assert_output(r#"
store Contact
    name ? ""
    phone ? ""
    *set_info(n, p)
        self.name is n
        self.phone is p
    *info()
        return self.name + " (" + self.phone + ")"

*main()
    c is make_Contact()
    c.set_info("Alice", "555-1234")
    log(c.info())
    c.set_info("Alice Smith", "555-5678")
    log(c.info())
"#, &["Alice (555-1234)", "Alice Smith (555-5678)"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 9. STORE TYPE INTROSPECTION
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_type_of() {
    assert_output(r#"
store Widget
    label ? "button"

*main()
    w is make_Widget()
    log(type_of(w))
"#, &["map"]);
}

// ═══════════════════════════════════════════════════════════════════════
// 10. STORE COMPILATION EDGE CASES
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn store_many_fields_compiles() {
    compile(r#"
store BigStore
    a ? 0
    b ? 0
    c ? 0
    d ? ""
    e ? ""
    f ? false
    g ? []

*main()
    log("ok")
"#).expect("Store with many fields should compile");
}

#[test]
fn store_with_method_only_compiles() {
    compile(r#"
store Service
    status ? "idle"
    *start()
        self.status is "running"
    *stop()
        self.status is "stopped"
    *get_status()
        return self.status

*main()
    log("ok")
"#).expect("Store with methods should compile");
}

#[test]
fn store_with_trait_compiles() {
    compile(r#"
trait Printable
    *print_info()
        log("printable")

store Document with Printable
    title ? ""
    *print_info()
        log("Doc: " + self.title)

*main()
    log("ok")
"#).expect("Store with trait should compile");
}
