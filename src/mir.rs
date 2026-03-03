//! Simple MIR structures used as a bridge between AST and LLVM.

#[derive(Debug, Clone)]
pub struct MirModule {
    pub functions: Vec<MirFunction>,
}

#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: String,
    pub params: Vec<String>,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub name: String,
    pub instrs: Vec<Instr>,
    pub term: Terminator,
}

#[derive(Debug, Clone)]
pub enum Instr {
    Const { dst: String, val: Literal },
    BinOp { dst: String, op: BinOp, lhs: Operand, rhs: Operand },
    Call { dst: Option<String>, func: String, args: Vec<Operand> },
    AllocList { dst: String, len: Operand },
    ListPush { list: Operand, value: Operand },
    MapMake { dst: String, entries: Vec<(Operand, Operand)> },
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Ret(Option<Operand>),
    Jump(String),
    Cond { cond: Operand, then_bb: String, else_bb: String },
}

#[derive(Debug, Clone)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    And,
    Or,
    Eq,
    NotEq,
}

#[derive(Debug, Clone)]
pub enum Literal {
    Number(f64),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
    Unit,
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(String),
    Const(Literal),
}
