use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
    pub span: Span,
}

impl Program {
    pub fn new(items: Vec<Item>, span: Span) -> Self {
        Self { items, span }
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
    Expression(Expression),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDefinition {
    pub name: String,
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreDefinition {
    pub name: String,
    pub fields: Vec<Field>,
    pub methods: Vec<Function>,
    pub is_actor: bool,
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
}

impl Expression {
    pub fn span(&self) -> Span {
        match self {
            Expression::Unit => Span::default(),
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
            | Expression::Binary { span, .. }
            | Expression::Unary { span, .. }
            | Expression::Call { span, .. }
            | Expression::Member { span, .. }
            | Expression::Ternary { span, .. } => *span,
            Expression::Match(expr) => expr.span,
            Expression::Throw { span, .. } => *span,
            Expression::Lambda { span, .. } => *span,
            Expression::InlineAsm { span, .. } => *span,
            Expression::PtrLoad { span, .. } => *span,
            Expression::Unsafe { span, .. } => *span,
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
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchPattern {
    Integer(i64),
    Bool(bool),
    Identifier(String),
    String(String),
    List(Vec<Expression>),
}
