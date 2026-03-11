use crate::span::Span;

/// A single module's AST, produced by parsing one source file.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Module namespace (e.g., "std.math", "std.prelude", "main")
    pub name: String,
    /// Parsed items belonging to this module
    pub items: Vec<Item>,
    /// Names of modules this module directly imports
    pub imports: Vec<String>,
    /// Exported symbols (function names, type names, etc.)
    pub exports: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
    /// Per-module ASTs, in dependency order (last is the entry module).
    /// Empty when compiled from a single flat source string.
    pub modules: Vec<Module>,
    pub span: Span,
}

impl Program {
    pub fn new(items: Vec<Item>, span: Span) -> Self {
        Self { items, modules: Vec::new(), span }
    }

    /// Create a Program from parsed modules. Items are flattened in module order.
    pub fn from_modules(modules: Vec<Module>) -> Self {
        let items: Vec<Item> = modules.iter()
            .flat_map(|m| m.items.iter().cloned())
            .collect();
        let span = if let Some(last) = modules.last() {
            last.span
        } else {
            Span::new(0, 0)
        };
        Self { items, modules, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Binding(Binding),
    Function(Function),
    ExternFunction(ExternFunction),
    Type(TypeDefinition),
    Store(StoreDefinition),
    Taxonomy(TaxonomyNode),
    ErrorDefinition(ErrorDefinition),
    TraitDefinition(TraitDefinition),
    Extension(ExtensionDefinition),
    Expression(Expression),
}

/// S4.5: Extension methods — add methods to existing types
/// ```coral
/// extend String
///     *word_count()
///         self.split(" ").length()
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionDefinition {
    pub target_type: String,
    pub methods: Vec<Function>,
    pub span: Span,
}

/// Hierarchical error definition
/// ```coral
/// err Database
///     err Connection
///         err Timeout
///             code is 5001
///             message is 'Connection timed out'
///         err Refused
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorDefinition {
    pub name: String,
    pub code: Option<i64>,
    pub message: Option<String>,
    pub children: Vec<ErrorDefinition>,
    pub span: Span,
}

/// Trait definition for mixins/interfaces
/// ```coral
/// trait Printable
///     *to_string()
///     *print()
///         log(to_string())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDefinition {
    pub name: String,
    pub required_traits: Vec<String>,  // `with TraitName` dependencies
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

/// A method in a trait - may have a default implementation
#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<Parameter>,
    pub body: Option<Block>,  // None = required method, Some = default implementation
    pub span: Span,
}

/// T2.4: A generic type parameter with optional trait bounds.
/// `T` or `T with Comparable` or `T with Comparable, Hashable`
#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<String>,  // trait bounds, e.g., ["Comparable", "Hashable"]
}

impl TypeParam {
    pub fn new(name: String, bounds: Vec<String>) -> Self {
        Self { name, bounds }
    }

    pub fn plain(name: String) -> Self {
        Self { name, bounds: vec![] }
    }
}

impl From<&str> for TypeParam {
    fn from(name: &str) -> Self {
        TypeParam { name: name.to_string(), bounds: vec![] }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternFunction {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub value: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Parameter>,
    pub body: Block,
    pub kind: FunctionKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionKind {
    Free,
    Method,
    ActorMessage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<TypeAnnotation>,
    pub default: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeAnnotation {
    pub segments: Vec<String>,
    pub type_args: Vec<TypeAnnotation>,  // For generic types like List[int], Map[string, int]
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub value: Option<Box<Expression>>,
    pub span: Span,
}

impl Block {
    pub fn from_expression(expr: Expression) -> Self {
        let span = expr.span();
        Self {
            statements: vec![],
            value: Some(Box::new(expr)),
            span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Binding(Binding),
    Expression(Expression),
    Return(Expression, Span),
    If {
        condition: Expression,
        body: Block,
        elif_branches: Vec<(Expression, Block)>,
        else_body: Option<Block>,
        span: Span,
    },
    While {
        condition: Expression,
        body: Block,
        span: Span,
    },
    For {
        variable: String,
        iterable: Expression,
        body: Block,
        span: Span,
    },
    /// Key-value for loop over maps: `for key, value in map`
    ForKV {
        key_var: String,
        value_var: String,
        iterable: Expression,
        body: Block,
        span: Span,
    },
    /// Range-based for loop: `for i in start to end [step s]`
    ForRange {
        variable: String,
        start: Expression,
        end: Expression,
        step: Option<Expression>,
        body: Block,
        span: Span,
    },
    /// Field assignment on a store/actor: `self.field is value`
    FieldAssign {
        target: Expression,
        field: String,
        value: Expression,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    /// S2.4: Destructuring assignment — `[a, b] is expr` or `Some(v) is expr`
    PatternBinding {
        pattern: MatchPattern,
        value: Expression,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDefinition {
    pub name: String,
    pub type_params: Vec<TypeParam>,  // T2.4: Generic type parameters with optional bounds
    pub with_traits: Vec<String>,  // `with TraitName` clauses
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub variants: Vec<TypeVariant>,  // For sum types (ADTs)
    pub span: Span,
}

impl TypeDefinition {
    /// Get just the parameter names (without bounds).
    pub fn param_names(&self) -> Vec<&str> {
        self.type_params.iter().map(|tp| tp.name.as_str()).collect()
    }
}

/// A variant of a sum type (ADT).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeVariant {
    pub name: String,
    pub fields: Vec<VariantField>,  // Named or positional fields
    pub span: Span,
}

/// A field in a type variant (can be named or positional).
#[derive(Debug, Clone, PartialEq)]
pub struct VariantField {
    pub name: Option<String>,  // None for positional fields like Some(value)
    pub type_annotation: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreDefinition {
    pub name: String,
    pub with_traits: Vec<String>,  // `with TraitName` clauses
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub is_actor: bool,
    pub is_persistent: bool,  // `persist store` vs `store`
    /// R2.7: `@messages(TypeName)` annotation restricting accepted message types.
    pub message_type: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaxonomyNode {
    pub name: String,
    pub children: Vec<TaxonomyNode>,
    pub bindings: Vec<Binding>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub is_reference: bool,
    pub default: Option<Expression>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Unit,
    None(Span),  // the `none` keyword - represents absence/null
    Identifier(String, Span),
    Integer(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    String(String, Span),
    Bytes(Vec<u8>, Span),
    Placeholder(u32, Span),
    TaxonomyPath { segments: Vec<String>, span: Span },
    Throw { value: Box<Expression>, span: Span },
    Lambda {
        params: Vec<Parameter>,
        body: Block,
        span: Span,
    },
    List(Vec<Expression>, Span),
    Map(Vec<(Expression, Expression)>, Span),
    /// Spread expression: `...expr` — used inside list/map literals
    Spread(Box<Expression>, Span),
    /// List comprehension: `[body for var in iterable if condition]`
    ListComprehension {
        body: Box<Expression>,
        var: String,
        iterable: Box<Expression>,
        condition: Option<Box<Expression>>,
        span: Span,
    },
    /// Map comprehension: `{key: value for var in iterable if condition}`
    MapComprehension {
        key: Box<Expression>,
        value: Box<Expression>,
        var: String,
        iterable: Box<Expression>,
        condition: Option<Box<Expression>>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expression>,
        right: Box<Expression>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expression>,
        span: Span,
    },
    Call {
        callee: Box<Expression>,
        args: Vec<Expression>,
        /// S4.1: Optional names for named arguments.
        /// Empty means all positional. When non-empty, same length as `args`.
        arg_names: Vec<Option<String>>,
        span: Span,
    },
    Member {
        target: Box<Expression>,
        property: String,
        span: Span,
    },
    Ternary {
        condition: Box<Expression>,
        then_branch: Box<Expression>,
        else_branch: Box<Expression>,
        span: Span,
    },
    /// Pipeline operator: `expr ~ fn(args)` desugars to `fn(expr, args)`
    /// Chains left-to-right for better readability of data flow
    Pipeline {
        left: Box<Expression>,
        right: Box<Expression>,
        span: Span,
    },
    /// Error value expression: `err Name` or `err Name:SubName`
    ErrorValue {
        path: Vec<String>,
        span: Span,
    },
    /// Error propagation: `expr ! return err` - returns immediately if expr is error
    ErrorPropagate {
        expr: Box<Expression>,
        span: Span,
    },
    Match(Box<MatchExpression>),
    InlineAsm {
        template: String,
        inputs: Vec<(String, Expression)>,
        outputs: Vec<String>,
        span: Span,
    },
    PtrLoad {
        address: Box<Expression>,
        span: Span,
    },
    Unsafe {
        block: Block,
        span: Span,
    },
    /// Index/subscript expression: `expr[index]`
    Index {
        target: Box<Expression>,
        index: Box<Expression>,
        span: Span,
    },
    /// Slice expression: `expr[start to end]`
    Slice {
        target: Box<Expression>,
        start: Box<Expression>,
        end: Box<Expression>,
        span: Span,
    },
}

impl Expression {
    pub fn span(&self) -> Span {
        match self {
            Expression::Unit => Span::default(),
            Expression::None(span) => *span,
            Expression::Identifier(_, span)
            | Expression::Integer(_, span)
            | Expression::Float(_, span)
            | Expression::Bool(_, span)
            | Expression::String(_, span)
            | Expression::Bytes(_, span)
            | Expression::Placeholder(_, span)
            | Expression::TaxonomyPath { span, .. }
            | Expression::List(_, span)
            | Expression::Map(_, span)
            | Expression::Spread(_, span)
            | Expression::ListComprehension { span, .. }
            | Expression::MapComprehension { span, .. }
            | Expression::Binary { span, .. }
            | Expression::Unary { span, .. }
            | Expression::Call { span, .. }
            | Expression::Member { span, .. }
            | Expression::Ternary { span, .. }
            | Expression::Pipeline { span, .. }
            | Expression::ErrorValue { span, .. }
            | Expression::ErrorPropagate { span, .. } => *span,
            Expression::Match(expr) => expr.span,
            Expression::Throw { span, .. } => *span,
            Expression::Lambda { span, .. } => *span,
            Expression::InlineAsm { span, .. } => *span,
            Expression::PtrLoad { span, .. } => *span,
            Expression::Unsafe { span, .. } => *span,
            Expression::Index { span, .. } => *span,
            Expression::Slice { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Equals,
    NotEquals,
    Greater,
    GreaterEq,
    Less,
    LessEq,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchExpression {
    pub value: Box<Expression>,
    pub arms: Vec<MatchArm>,
    pub default: Option<Box<Block>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    /// S3.2: Optional guard clause — `Pattern if condition ? body`
    pub guard: Option<Box<Expression>>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    Integer(i64),
    Bool(bool),
    Identifier(String),
    String(String),
    List(Vec<MatchPattern>),
    /// Constructor pattern for sum types: Some(x), None, Ok(val), Err(e)
    Constructor {
        name: String,
        fields: Vec<MatchPattern>,  // Nested patterns for fields
        span: Span,
    },
    /// Wildcard pattern that matches anything without binding
    Wildcard(Span),
    /// S3.3: Or-pattern — matches if any sub-pattern matches.
    /// e.g. `Circle(r) or Sphere(r) ? compute(r)`
    Or(Vec<MatchPattern>),
    /// S3.5: Range pattern — matches if value is within an inclusive range.
    /// e.g. `200 to 299` matches integers from 200 to 299 inclusive.
    Range { start: i64, end: i64, span: Span },
    /// S3.4: Rest/spread pattern — captures remaining list elements.
    /// e.g. `[first, ...rest]` binds `rest` to remaining elements.
    Rest(String, Span),
}
