use crate::mir::{MirModule, Instr, Terminator, Operand, Literal};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
    Unit,
}

pub struct Interpreter {
    module: MirModule,
}

impl Interpreter {
    pub fn new(module: MirModule) -> Self {
        Self { module }
    }

    pub fn run_function(&self, name: &str) -> Option<Value> {
        let func = self.module.functions.iter().find(|f| f.name == name)?;
        let mut locals: HashMap<String, Value> = HashMap::new();
        // Start at first block
        let mut cur = 0usize;
        loop {
            let bb = &func.blocks[cur];
            for instr in &bb.instrs {
                match instr {
                    Instr::Const { dst, val } => {
                        let value = match val {
                            Literal::Number(n) => Value::Number(*n),
                            Literal::Bool(b) => Value::Bool(*b),
                            Literal::String(s) => Value::String(s.clone()),
                            Literal::Bytes(bytes) => Value::Bytes(bytes.clone()),
                            Literal::Unit => Value::Unit,
                        };
                        locals.insert(dst.clone(), value);
                    }
                    Instr::BinOp { dst, op, lhs, rhs } => {
                        let l = self.eval_operand(lhs, &locals);
                        let r = self.eval_operand(rhs, &locals);
                        let res = match (l, r) {
                            (Value::Number(a), Value::Number(b)) => match op {
                                crate::mir::BinOp::Add => Value::Number(a + b),
                                crate::mir::BinOp::Sub => Value::Number(a - b),
                                crate::mir::BinOp::Mul => Value::Number(a * b),
                                crate::mir::BinOp::Div => Value::Number(a / b),
                                _ => Value::Unit,
                            },
                            _ => Value::Unit,
                        };
                        locals.insert(dst.clone(), res);
                    }
                    Instr::Call { dst, func, args } => {
                        if func == "log" {
                            let arg0 = args.get(0).and_then(|o| self.eval_operand_opt(o, &locals));
                            if let Some(v) = arg0 {
                                println!("{}", self.format_value(&v));
                            }
                            if let Some(dst) = dst {
                                locals.insert(dst.clone(), Value::Unit);
                            }
                        } else {
                            // module-local function call (no args support yet)
                            let res = self.run_function(func).unwrap_or(Value::Unit);
                            if let Some(dst) = dst {
                                locals.insert(dst.clone(), res);
                            }
                        }
                    }
                    Instr::AllocList { dst, len } => {
                        let _ = self.eval_operand(len, &locals);
                        locals.insert(dst.clone(), Value::Unit);
                    }
                    Instr::ListPush { list, value } => {
                        let _ = self.eval_operand(list, &locals);
                        let _ = self.eval_operand(value, &locals);
                    }
                    Instr::MapMake { dst, entries } => {
                        for (k, v) in entries {
                            let _ = self.eval_operand(k, &locals);
                            let _ = self.eval_operand(v, &locals);
                        }
                        locals.insert(dst.clone(), Value::Unit);
                    }
                }
            }
            match &bb.term {
                Terminator::Ret(opt) => return opt.as_ref().map(|o| self.eval_operand(o, &locals)),
                Terminator::Jump(name) => {
                    cur = func
                        .blocks
                        .iter()
                        .position(|b| &b.name == name)
                        .unwrap_or(cur + 1);
                }
                Terminator::Cond { cond, then_bb, else_bb } => {
                    let c = self.eval_operand(cond, &locals);
                    let is_true = matches!(c, Value::Bool(true)) || matches!(c, Value::Number(n) if n.abs() > f64::EPSILON);
                    let target = if is_true { then_bb } else { else_bb };
                    cur = func
                        .blocks
                        .iter()
                        .position(|b| &b.name == target)
                        .unwrap_or(cur + 1);
                }
            }
            // If we fell off the end, stop
            if cur >= func.blocks.len() {
                return None;
            }
        }
    }

    fn eval_operand(&self, operand: &Operand, locals: &HashMap<String, Value>) -> Value {
        self.eval_operand_opt(operand, locals).unwrap_or(Value::Unit)
    }

    fn eval_operand_opt(&self, operand: &Operand, locals: &HashMap<String, Value>) -> Option<Value> {
        match operand {
            Operand::Local(name) => locals.get(name).cloned(),
            Operand::Const(lit) => match lit {
                Literal::Number(n) => Some(Value::Number(*n)),
                Literal::Bool(b) => Some(Value::Bool(*b)),
                Literal::String(s) => Some(Value::String(s.clone())),
                Literal::Bytes(bytes) => Some(Value::Bytes(bytes.clone())),
                Literal::Unit => Some(Value::Unit),
            },
        }
    }

    fn format_value(&self, v: &Value) -> String {
        match v {
            Value::Number(n) => format!("{}", n),
            Value::Bool(b) => format!("{}", b),
            Value::String(s) => s.clone(),
            Value::Bytes(bytes) => format!("[bytes {}]", bytes.len()),
            Value::Unit => "()".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{MirModule, MirFunction, BasicBlock, Instr, Terminator, Operand, Literal};

    #[test]
    fn run_simple_add() {
        let func = MirFunction {
            name: "main".into(),
            params: vec![],
            blocks: vec![BasicBlock {
                name: "entry".into(),
                instrs: vec![Instr::Const { dst: "a".into(), val: Literal::Number(1.0) }, Instr::Const { dst: "b".into(), val: Literal::Number(2.0) }, Instr::BinOp { dst: "c".into(), op: crate::mir::BinOp::Add, lhs: Operand::Local("a".into()), rhs: Operand::Local("b".into()) }],
                term: Terminator::Ret(Some(Operand::Local("c".into()))),
            }],
        };
        let module = MirModule { functions: vec![func] };
        let interp = Interpreter::new(module);
        let res = interp.run_function("main");
        assert!(matches!(res, Some(Value::Number(n)) if (n - 3.0).abs() < 1e-9));
    }
}
