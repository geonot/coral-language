use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Alloc,
    Io,
    Time,
    Scheduler,
    Metrics,
    Tracing,
}

#[derive(Debug, Clone)]
pub struct RuntimeModule {
    pub name: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
}

impl RuntimeModule {
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            capabilities,
        }
    }
}

#[derive(Debug, Default)]
pub struct RuntimeModuleRegistry {
    modules: Vec<RuntimeModule>,
}

impl RuntimeModuleRegistry {
    pub fn register(&mut self, module: RuntimeModule) {
        self.modules.push(module);
    }

    pub fn list(&self) -> &[RuntimeModule] {
        &self.modules
    }
}

static REGISTRY: OnceLock<Mutex<RuntimeModuleRegistry>> = OnceLock::new();

pub fn registry() -> &'static Mutex<RuntimeModuleRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(RuntimeModuleRegistry::default()))
}
