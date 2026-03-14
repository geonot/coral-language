use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,

    pub items: Vec<Item>,

    pub imports: Vec<String>,

    pub exports: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,

    pub modules: Vec<Module>,
    pub span: Span,
}

impl Program {
    pub fn new(items: Vec<Item>, span: Span) -> Self {
        Self {
            items,
            modules: Vec::new(),
            span,
        }
    }

    pub fn from_modules(modules: Vec<Module>) -> Self {
        let items: Vec<Item> = modules
            .iter()
            .flat_map(|m| m.items.iter().cloned())
            .collect();
        let span = if let Some(last) = modules.last() {
            last.span
        } else {
            Span::new(0, 0)
        };
        Self {
            items,
            modules,
            span,
        }
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

#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionDefinition {
    pub target_type: String,
    pub methods: Vec<Function>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorDefinition {
    pub name: String,
    pub code: Option<i64>,
    pub message: Option<String>,
    pub children: Vec<ErrorDefinition>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDefinition {
    pub name: String,
    pub required_traits: Vec<String>,
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<Parameter>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeParam {
    pub name: String,
    pub bounds: Vec<String>,
    pub is_const: bool,
}

impl TypeParam {
    pub fn new(name: String, bounds: Vec<String>) -> Self {
        Self {
            name,
            bounds,
            is_const: false,
        }
    }

    pub fn plain(name: String) -> Self {
        Self {
            name,
            bounds: vec![],
            is_const: false,
        }
    }

    pub fn const_param(name: String) -> Self {
        Self {
            name,
            bounds: vec![],
            is_const: true,
        }
    }
}

impl From<&str> for TypeParam {
    fn from(name: &str) -> Self {
        TypeParam {
            name: name.to_string(),
            bounds: vec![],
            is_const: false,
        }
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
    pub type_args: Vec<TypeAnnotation>,
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

    ForKV {
        key_var: String,
        value_var: String,
        iterable: Expression,
        body: Block,
        span: Span,
    },

    ForRange {
        variable: String,
        start: Expression,
        end: Expression,
        step: Option<Expression>,
        body: Block,
        span: Span,
    },

    FieldAssign {
        target: Expression,
        field: String,
        value: Expression,
        span: Span,
    },
    Break(Span),
    Continue(Span),

    PatternBinding {
        pattern: MatchPattern,
        value: Expression,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDefinition {
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub with_traits: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub variants: Vec<TypeVariant>,
    pub span: Span,
}

impl TypeDefinition {
    pub fn param_names(&self) -> Vec<&str> {
        self.type_params.iter().map(|tp| tp.name.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeVariant {
    pub name: String,
    pub fields: Vec<VariantField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantField {
    pub name: Option<String>,
    pub type_annotation: Option<TypeAnnotation>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreDefinition {
    pub name: String,
    pub with_traits: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub is_actor: bool,
    pub is_persistent: bool,

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
    None(Span),
    Identifier(String, Span),
    Integer(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    String(String, Span),
    Bytes(Vec<u8>, Span),
    Placeholder(u32, Span),
    TaxonomyPath {
        segments: Vec<String>,
        span: Span,
    },
    Throw {
        value: Box<Expression>,
        span: Span,
    },
    Lambda {
        params: Vec<Parameter>,
        body: Block,
        span: Span,
    },
    List(Vec<Expression>, Span),
    Map(Vec<(Expression, Expression)>, Span),

    Spread(Box<Expression>, Span),

    ListComprehension {
        body: Box<Expression>,
        var: String,
        iterable: Box<Expression>,
        condition: Option<Box<Expression>>,
        span: Span,
    },

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

    Pipeline {
        left: Box<Expression>,
        right: Box<Expression>,
        span: Span,
    },

    ErrorValue {
        path: Vec<String>,
        span: Span,
    },

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

    Index {
        target: Box<Expression>,
        index: Box<Expression>,
        span: Span,
    },

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

    Constructor {
        name: String,
        fields: Vec<MatchPattern>,
        span: Span,
    },

    Wildcard(Span),

    Or(Vec<MatchPattern>),

    Range {
        start: i64,
        end: i64,
        span: Span,
    },

    RangeBinding {
        name: String,
        start: i64,
        end: i64,
        span: Span,
    },

    Rest(String, Span),
}
