use super::binary::StoredValue;
use super::engine::SharedStoreEngine;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

static TX_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxState {
    Active,
    Committed,
    Aborted,
}

#[derive(Debug, Clone)]
enum TxOp {
    Insert {
        index: u64,
        fields: Vec<(String, StoredValue)>,
    },
    Update {
        index: u64,
        fields: Vec<(String, StoredValue)>,
    },
    Delete {
        index: u64,
    },
}

pub struct Transaction {
    id: u64,
    state: TxState,
    engine: SharedStoreEngine,
    ops: Vec<TxOp>,
    snapshots: HashMap<u64, Vec<(String, StoredValue)>>,
}

impl Transaction {
    pub fn begin(engine: SharedStoreEngine) -> Self {
        let id = TX_COUNTER.fetch_add(1, Ordering::SeqCst);
        Self {
            id,
            state: TxState::Active,
            engine,
            ops: Vec::new(),
            snapshots: HashMap::new(),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn state(&self) -> TxState {
        self.state
    }

    fn check_active(&self) -> io::Result<()> {
        if self.state != TxState::Active {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("transaction {} is {:?}, not active", self.id, self.state),
            ));
        }
        Ok(())
    }

    pub fn insert(&mut self, fields: Vec<(String, StoredValue)>) -> io::Result<u64> {
        self.check_active()?;
        let index = self.engine.create(fields.clone())?;
        self.ops.push(TxOp::Insert { index, fields });
        Ok(index)
    }

    pub fn update(
        &mut self,
        index: u64,
        fields: Vec<(String, StoredValue)>,
    ) -> io::Result<()> {
        self.check_active()?;
        if !self.snapshots.contains_key(&index) {
            if let Some(obj) = self.engine.get(index)? {
                self.snapshots.insert(index, obj.fields.clone());
            }
        }
        self.engine.update(index, fields.clone())?;
        self.ops.push(TxOp::Update { index, fields });
        Ok(())
    }

    pub fn delete(&mut self, index: u64) -> io::Result<()> {
        self.check_active()?;
        if !self.snapshots.contains_key(&index) {
            if let Some(obj) = self.engine.get(index)? {
                self.snapshots.insert(index, obj.fields.clone());
            }
        }
        self.engine.delete(index)?;
        self.ops.push(TxOp::Delete { index });
        Ok(())
    }

    pub fn commit(mut self) -> io::Result<()> {
        self.check_active()?;
        self.engine.checkpoint()?;
        self.state = TxState::Committed;
        Ok(())
    }

    pub fn rollback(mut self) -> io::Result<()> {
        self.check_active()?;

        for op in self.ops.iter().rev() {
            match op {
                TxOp::Insert { index, .. } => {
                    let _ = self.engine.delete(*index);
                }
                TxOp::Update { index, .. } => {
                    if let Some(original) = self.snapshots.get(index) {
                        let _ = self.engine.update(*index, original.clone());
                    }
                }
                TxOp::Delete { index } => {
                    if let Some(original) = self.snapshots.get(index) {
                        let _ = self.engine.create(original.clone());
                    }
                }
            }
        }
        self.state = TxState::Aborted;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::config::StoreConfig;
    use crate::store::engine::StoreEngine;

    fn make_engine(name: &str) -> SharedStoreEngine {
        let dir = std::env::temp_dir().join(format!("coral_tx_test_{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let config = StoreConfig::minimal("test", &dir);
        let engine = StoreEngine::open("test", name, config).unwrap();
        SharedStoreEngine::new(engine)
    }

    #[test]
    fn transaction_commit() {
        let engine = make_engine("tx_commit");
        let mut tx = Transaction::begin(engine.clone());
        let idx = tx
            .insert(vec![("name".into(), StoredValue::String("alice".into()))])
            .unwrap();
        tx.commit().unwrap();

        let obj = engine.get(idx).unwrap();
        assert!(obj.is_some());
    }

    #[test]
    fn transaction_rollback_insert() {
        let engine = make_engine("tx_rollback_ins");
        let mut tx = Transaction::begin(engine.clone());
        let idx = tx
            .insert(vec![("name".into(), StoredValue::String("bob".into()))])
            .unwrap();
        tx.rollback().unwrap();

        let obj = engine.get(idx).unwrap();
        assert!(obj.is_none() || obj.unwrap().is_deleted());
    }

    #[test]
    fn transaction_rollback_update() {
        let engine = make_engine("tx_rollback_upd");
        let idx = engine
            .create(vec![("score".into(), StoredValue::Int(100))])
            .unwrap();

        let mut tx = Transaction::begin(engine.clone());
        tx.update(idx, vec![("score".into(), StoredValue::Int(200))])
            .unwrap();
        tx.rollback().unwrap();

        let obj = engine.get(idx).unwrap().unwrap();
        let score = obj
            .fields
            .iter()
            .find(|(k, _)| k == "score")
            .unwrap()
            .1
            .clone();
        assert_eq!(score, StoredValue::Int(100));
    }
}
