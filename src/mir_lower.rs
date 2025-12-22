use crate::ast::{Program, Item, Function, Expression, Statement, BinaryOp};
use crate::mir::{MirModule, MirFunction, BasicBlock, Instr, Terminator, Operand, Literal, BinOp};
use crate::semantic::SemanticModel;
use crate::span::Span;

struct NameGen {
    next: usize,
}

impl NameGen {
    fn new() -> Self { Self { next: 0 } }
    fn temp(&mut self) -> String {
        let s = format!("t{}", self.next);
        self.next += 1;
        s
    }
}

pub fn lower_program(program: &Program) -> MirModule {
    let mut functions = Vec::new();
    for item in &program.items {
        if let Item::Function(func) = item {
            functions.push(lower_function(func));
        }
    }
    MirModule { functions }
}

pub fn lower_semantic_model(model: &SemanticModel) -> MirModule {
    let mut items = Vec::with_capacity(model.globals.len() + model.functions.len());
    for binding in &model.globals {
        items.push(Item::Binding(binding.clone()));
    }
    for function in &model.functions {
        items.push(Item::Function(function.clone()));
    }
    let program = Program::new(items, Span::new(0, 0));
    lower_program(&program)
}

fn lower_function(function: &Function) -> MirFunction {
    let mut ng = NameGen::new();
    let mut instrs = Vec::new();
    // only support a single block named entry
    let bb_name = "entry".to_string();

    // Lower statements in body
    for stmt in &function.body.statements {
        match stmt {
            Statement::Binding(binding) => {
                let dst = binding.name.clone();
                let (val_instrs, dst_op) = lower_expression(&binding.value, &mut ng);
                instrs.extend(val_instrs);
                // assign to binding name
                match dst_op {
                    Operand::Local(n) => {
                        // if temp, emit move by a const instr (here we simply rename)
                        instrs.push(Instr::Const { dst, val: match_operand_to_literal(&Operand::Local(n)) });
                    }
                    Operand::Const(lit) => {
                        instrs.push(Instr::Const { dst, val: lit });
                    }
                }
            }
            Statement::Expression(expr) => {
                let (mut s, _op) = lower_expression(expr, &mut ng);
                instrs.append(&mut s);
            }
            Statement::Return(expr, _span) => {
                let (mut s, op) = lower_expression(expr, &mut ng);
                instrs.append(&mut s);
                let term = Terminator::Ret(Some(op));
                return MirFunction {
                    name: function.name.clone(),
                    params: function.params.iter().map(|p| p.name.clone()).collect(),
                    blocks: vec![BasicBlock { name: bb_name, instrs, term }],
                };
            }
        }
    }

    // If function has a final value (block.value)
    if let Some(value) = &function.body.value {
    let (mut s, op) = lower_expression(value, &mut ng);
        instrs.append(&mut s);
        let term = Terminator::Ret(Some(op));
        MirFunction {
            name: function.name.clone(),
            params: function.params.iter().map(|p| p.name.clone()).collect(),
            blocks: vec![BasicBlock { name: bb_name, instrs, term }],
        }
    } else {
        let term = Terminator::Ret(None);
        MirFunction {
            name: function.name.clone(),
            params: function.params.iter().map(|p| p.name.clone()).collect(),
            blocks: vec![BasicBlock { name: bb_name, instrs, term }],
        }
    }
}

fn match_operand_to_literal(op: &Operand) -> Literal {
    match op {
        Operand::Const(l) => l.clone(),
        Operand::Local(_) => Literal::Unit,
    }
}

fn lower_expression(expr: &Expression, ng: &mut NameGen) -> (Vec<Instr>, Operand) {
    match expr {
        Expression::Integer(i, _span) => (vec![], Operand::Const(Literal::Number(*i as f64))),
        Expression::Float(f, _span) => (vec![], Operand::Const(Literal::Number(*f))),
        Expression::Bool(b, _span) => (vec![], Operand::Const(Literal::Bool(*b))),
        Expression::String(s, _span) => (vec![], Operand::Const(Literal::String(s.clone()))),
        Expression::Bytes(bytes, _span) => (
            vec![],
            Operand::Const(Literal::Bytes(bytes.clone())),
        ),
        Expression::Identifier(name, _span) => (vec![], Operand::Local(name.clone())),
        Expression::Binary { op, left, right, .. } => {
            let (mut ls, lop) = lower_expression(left, ng);
            let (mut rs, rop) = lower_expression(right, ng);
            ls.append(&mut rs);
            let dst = ng.temp();
            let binop = match op {
                BinaryOp::Add => BinOp::Add,
                BinaryOp::Sub => BinOp::Sub,
                BinaryOp::Mul => BinOp::Mul,
                BinaryOp::Div => BinOp::Div,
                BinaryOp::And => BinOp::And,
                BinaryOp::Or => BinOp::Or,
                BinaryOp::Equals => BinOp::Eq,
                BinaryOp::NotEquals | BinaryOp::Greater | BinaryOp::GreaterEq | BinaryOp::Less | BinaryOp::LessEq => BinOp::Eq,
                _ => BinOp::Eq,
            };
            ls.push(Instr::BinOp { dst: dst.clone(), op: binop, lhs: lop, rhs: rop });
            (ls, Operand::Local(dst))
        }
        Expression::Call { callee, args, .. } => {
            // only support simple calls like log(x)
            let mut instrs = Vec::new();
            let mut operands = Vec::new();
            for a in args {
                let (mut s, op) = lower_expression(a, ng);
                instrs.append(&mut s);
                operands.push(op);
            }
            // callee may be identifier
            if let Expression::Identifier(name, _span) = &**callee {
                let dst = ng.temp();
                instrs.push(Instr::Call { dst: Some(dst.clone()), func: name.clone(), args: operands });
                return (instrs, Operand::Local(dst));
            }
            (instrs, Operand::Const(Literal::Unit))
        }
        _ => (vec![], Operand::Const(Literal::Unit)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Program, Item, Function, Block, Statement, Expression};
    use crate::mir_interpreter::Interpreter;

    #[test]
    fn lower_and_run_simple_function() {
        let func = Function {
            name: "main".into(),
            params: vec![],
            body: Block { statements: vec![Statement::Expression(Expression::Call { callee: Box::new(Expression::Identifier("log".into(), crate::span::Span::new(0,0))), args: vec![Expression::Integer(1, crate::span::Span::new(0,0))], span: crate::span::Span::new(0,0) })], value: Some(Box::new(Expression::Integer(2, crate::span::Span::new(0,0)))), span: crate::span::Span::new(0,0) },
            kind: crate::ast::FunctionKind::Free,
            span: crate::span::Span::new(0,0),
        };
        let program = Program::new(vec![Item::Function(func)], crate::span::Span::new(0,0));
        let mir = lower_program(&program);
        let interp = Interpreter::new(mir);
        let res = interp.run_function("main");
        assert!(matches!(res, Some(crate::mir_interpreter::Value::Number(n)) if (n - 2.0).abs() < 1e-9));
    }
}
