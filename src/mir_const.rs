use crate::mir::MirModule;
use crate::mir_interpreter::{Interpreter, Value};

pub struct ConstEvaluator {
    interpreter: Interpreter,
}

impl ConstEvaluator {
    pub fn new(module: MirModule) -> Self {
        Self {
            interpreter: Interpreter::new(module),
        }
    }

    pub fn eval_zero_arity(&self, function: &str) -> Option<Value> {
        self.interpreter.run_function(function)
    }
}
