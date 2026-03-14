use super::binary::StoredValue;
use super::engine::StoreEngine;
use std::io;

#[derive(Debug, Clone)]
pub enum QueryPlan {
    SeqScan {
        filter: Option<QueryFilter>,
    },
    IndexLookup {
        field: String,
        value: StoredValue,
    },
    IndexRange {
        field: String,
        low: Option<StoredValue>,
        high: Option<StoredValue>,
    },
}

#[derive(Debug, Clone)]
pub struct QueryFilter {
    pub field: String,
    pub op: FilterOp,
    pub value: StoredValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub indices: Vec<u64>,
    pub plan_used: QueryPlan,
}

pub struct QueryPlanner;

impl QueryPlanner {
    pub fn plan(engine: &StoreEngine, filter: &QueryFilter) -> QueryPlan {
        let indexed = engine.indexed_fields();

        match filter.op {
            FilterOp::Eq => {
                if indexed.contains(&filter.field) {
                    QueryPlan::IndexLookup {
                        field: filter.field.clone(),
                        value: filter.value.clone(),
                    }
                } else {
                    QueryPlan::SeqScan {
                        filter: Some(filter.clone()),
                    }
                }
            }
            FilterOp::Lt | FilterOp::Lte | FilterOp::Gt | FilterOp::Gte => {
                if indexed.contains(&filter.field) {
                    let (low, high) = match filter.op {
                        FilterOp::Lt | FilterOp::Lte => {
                            (None, Some(filter.value.clone()))
                        }
                        FilterOp::Gt | FilterOp::Gte => {
                            (Some(filter.value.clone()), None)
                        }
                        _ => unreachable!(),
                    };
                    QueryPlan::IndexRange {
                        field: filter.field.clone(),
                        low,
                        high,
                    }
                } else {
                    QueryPlan::SeqScan {
                        filter: Some(filter.clone()),
                    }
                }
            }
            FilterOp::NotEq => QueryPlan::SeqScan {
                filter: Some(filter.clone()),
            },
        }
    }
}

fn stored_value_cmp(a: &StoredValue, b: &StoredValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (StoredValue::Int(x), StoredValue::Int(y)) => Some(x.cmp(y)),
        (StoredValue::Float(x), StoredValue::Float(y)) => x.partial_cmp(y),
        (StoredValue::String(x), StoredValue::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn matches_filter(fields: &[(String, StoredValue)], filter: &QueryFilter) -> bool {
    let field_val = fields.iter().find(|(k, _)| *k == filter.field);
    match field_val {
        Some((_, val)) => match filter.op {
            FilterOp::Eq => val == &filter.value,
            FilterOp::NotEq => val != &filter.value,
            FilterOp::Lt => stored_value_cmp(val, &filter.value) == Some(std::cmp::Ordering::Less),
            FilterOp::Lte => matches!(
                stored_value_cmp(val, &filter.value),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            FilterOp::Gt => {
                stored_value_cmp(val, &filter.value) == Some(std::cmp::Ordering::Greater)
            }
            FilterOp::Gte => matches!(
                stored_value_cmp(val, &filter.value),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
        },
        None => false,
    }
}

pub fn execute_plan(engine: &mut StoreEngine, plan: &QueryPlan) -> io::Result<Vec<u64>> {
    match plan {
        QueryPlan::SeqScan { filter } => {
            let all = engine.all();
            if let Some(f) = filter {
                let mut results = Vec::new();
                for idx in all {
                    if let Some(obj) = engine.get(idx)? {
                        if matches_filter(&obj.fields, f) {
                            results.push(idx);
                        }
                    }
                }
                Ok(results)
            } else {
                Ok(all)
            }
        }
        QueryPlan::IndexLookup { field, value } => engine.find_by_field(field, value),
        QueryPlan::IndexRange {
            field, low, high, ..
        } => {
            match (low, high) {
                (Some(lo), Some(hi)) => engine.find_by_field_range(field, lo, hi),
                (Some(lo), None) => engine.find_by_field_range(
                    field,
                    lo,
                    &StoredValue::Int(i64::MAX),
                ),
                (None, Some(hi)) => engine.find_by_field_range(
                    field,
                    &StoredValue::Int(i64::MIN),
                    hi,
                ),
                (None, None) => Ok(engine.all()),
            }
        }
    }
}

pub fn query(engine: &mut StoreEngine, filter: &QueryFilter) -> io::Result<QueryResult> {
    let plan = QueryPlanner::plan(engine, filter);
    let indices = execute_plan(engine, &plan)?;
    Ok(QueryResult {
        indices,
        plan_used: plan,
    })
}

#[derive(Debug)]
pub enum AggregateOp {
    Count,
    Sum(String),
    Min(String),
    Max(String),
}

pub fn aggregate(
    engine: &mut StoreEngine,
    filter: Option<&QueryFilter>,
    op: &AggregateOp,
) -> io::Result<StoredValue> {
    let indices = if let Some(f) = filter {
        let plan = QueryPlanner::plan(engine, f);
        execute_plan(engine, &plan)?
    } else {
        engine.all()
    };

    match op {
        AggregateOp::Count => Ok(StoredValue::Int(indices.len() as i64)),
        AggregateOp::Sum(field) => {
            let mut sum = 0.0f64;
            for idx in &indices {
                if let Some(obj) = engine.get(*idx)? {
                    for (k, v) in &obj.fields {
                        if k == field {
                            match v {
                                StoredValue::Int(n) => sum += *n as f64,
                                StoredValue::Float(n) => sum += n,
                                _ => {}
                            }
                        }
                    }
                }
            }
            Ok(StoredValue::Float(sum))
        }
        AggregateOp::Min(field) => find_extremum(engine, &indices, field, true),
        AggregateOp::Max(field) => find_extremum(engine, &indices, field, false),
    }
}

fn find_extremum(
    engine: &mut StoreEngine,
    indices: &[u64],
    field: &str,
    find_min: bool,
) -> io::Result<StoredValue> {
    let mut best: Option<StoredValue> = None;
    for idx in indices {
        if let Some(obj) = engine.get(*idx)? {
            for (k, v) in &obj.fields {
                if k == field {
                    best = Some(match &best {
                        None => v.clone(),
                        Some(current) => {
                            if let Some(ord) = stored_value_cmp(v, current) {
                                if (find_min && ord == std::cmp::Ordering::Less)
                                    || (!find_min && ord == std::cmp::Ordering::Greater)
                                {
                                    v.clone()
                                } else {
                                    current.clone()
                                }
                            } else {
                                current.clone()
                            }
                        }
                    });
                }
            }
        }
    }
    Ok(best.unwrap_or(StoredValue::Unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_eq_match() {
        let fields = vec![
            ("name".to_string(), StoredValue::String("alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        let filter = QueryFilter {
            field: "name".to_string(),
            op: FilterOp::Eq,
            value: StoredValue::String("alice".to_string()),
        };
        assert!(matches_filter(&fields, &filter));
    }

    #[test]
    fn filter_gt_match() {
        let fields = vec![("age".to_string(), StoredValue::Int(30))];
        let filter = QueryFilter {
            field: "age".to_string(),
            op: FilterOp::Gt,
            value: StoredValue::Int(20),
        };
        assert!(matches_filter(&fields, &filter));
    }

    #[test]
    fn filter_missing_field() {
        let fields = vec![("name".to_string(), StoredValue::String("bob".to_string()))];
        let filter = QueryFilter {
            field: "age".to_string(),
            op: FilterOp::Eq,
            value: StoredValue::Int(25),
        };
        assert!(!matches_filter(&fields, &filter));
    }

    #[test]
    fn stored_value_ordering() {
        assert_eq!(
            stored_value_cmp(&StoredValue::Int(10), &StoredValue::Int(20)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            stored_value_cmp(&StoredValue::String("a".into()), &StoredValue::String("b".into())),
            Some(std::cmp::Ordering::Less)
        );
    }
}
