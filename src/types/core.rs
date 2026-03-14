use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Primitive {
    Int,
    Float,
    Bool,
    String,
    Bytes,
    Unit,
    None,
    Any,
    Actor,
}

impl fmt::Display for Primitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Primitive::Int => write!(f, "Int"),
            Primitive::Float => write!(f, "Float"),
            Primitive::Bool => write!(f, "Bool"),
            Primitive::String => write!(f, "String"),
            Primitive::Bytes => write!(f, "Bytes"),
            Primitive::Unit => write!(f, "Unit"),
            Primitive::None => write!(f, "None"),
            Primitive::Any => write!(f, "Any"),
            Primitive::Actor => write!(f, "Actor"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(pub u32);

impl fmt::Display for TypeVarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum TypeId {
    Primitive(Primitive),

    List(Box<TypeId>),

    Map(Box<TypeId>, Box<TypeId>),

    Func(Vec<TypeId>, Box<TypeId>),

    Placeholder(u32),

    TypeVar(TypeVarId),

    Adt(String, Vec<TypeId>),

    Store(String),

    Error(Vec<String>),

    #[default]
    Unknown,
}

impl TypeId {
    pub fn is_numeric(&self) -> bool {
        matches!(self, TypeId::Primitive(Primitive::Int | Primitive::Float))
    }

    pub fn is_var(&self) -> bool {
        matches!(self, TypeId::TypeVar(_))
    }

    pub fn is_concrete(&self) -> bool {
        match self {
            TypeId::Primitive(_)
            | TypeId::Unknown
            | TypeId::Placeholder(_)
            | TypeId::Store(_)
            | TypeId::Error(_) => true,
            TypeId::Adt(_, args) => args.iter().all(|a| a.is_concrete()),
            TypeId::TypeVar(_) => false,
            TypeId::List(elem) => elem.is_concrete(),
            TypeId::Map(k, v) => k.is_concrete() && v.is_concrete(),
            TypeId::Func(params, ret) => {
                params.iter().all(|p| p.is_concrete()) && ret.is_concrete()
            }
        }
    }

    pub fn contains_unknown(&self) -> bool {
        match self {
            TypeId::Unknown => true,
            TypeId::List(elem) => elem.contains_unknown(),
            TypeId::Map(k, v) => k.contains_unknown() || v.contains_unknown(),
            TypeId::Func(params, ret) => {
                params.iter().any(|p| p.contains_unknown()) || ret.contains_unknown()
            }
            TypeId::Adt(_, args) => args.iter().any(|a| a.contains_unknown()),
            _ => false,
        }
    }

    pub fn list_element_type(&self) -> Option<&TypeId> {
        match self {
            TypeId::List(elem) => Some(elem),
            _ => None,
        }
    }

    pub fn map_types(&self) -> Option<(&TypeId, &TypeId)> {
        match self {
            TypeId::Map(k, v) => Some((k, v)),
            _ => None,
        }
    }

    pub fn is_list(&self) -> bool {
        matches!(self, TypeId::List(_))
    }

    pub fn is_map(&self) -> bool {
        matches!(self, TypeId::Map(_, _))
    }

    pub fn is_collection(&self) -> bool {
        self.is_list() || self.is_map()
    }

    pub fn is_callable(&self) -> bool {
        matches!(self, TypeId::Func(_, _))
    }

    pub fn return_type(&self) -> Option<&TypeId> {
        match self {
            TypeId::Func(_, ret) => Some(ret),
            _ => None,
        }
    }

    pub fn param_types(&self) -> Option<&[TypeId]> {
        match self {
            TypeId::Func(params, _) => Some(params),
            _ => None,
        }
    }
}

pub fn format_type(ty: &TypeId) -> String {
    match ty {
        TypeId::Primitive(p) => p.to_string(),
        TypeId::List(elem) => format!("[{}]", format_type(elem)),
        TypeId::Map(k, v) => format!("{{{}: {}}}", format_type(k), format_type(v)),
        TypeId::Func(args, ret) => {
            let args_s: Vec<String> = args.iter().map(format_type).collect();
            format!("fn({}) -> {}", args_s.join(", "), format_type(ret))
        }
        TypeId::Placeholder(id) => format!("${}", id),
        TypeId::TypeVar(id) => format!("{}", id),
        TypeId::Adt(name, args) => {
            if args.is_empty() {
                name.clone()
            } else {
                let args_s: Vec<String> = args.iter().map(format_type).collect();
                format!("{}[{}]", name, args_s.join(", "))
            }
        }
        TypeId::Store(name) => name.clone(),
        TypeId::Error(segments) => {
            if segments.is_empty() {
                "Error".into()
            } else {
                format!("Error[{}]", segments.join("."))
            }
        }
        TypeId::Unknown => "_".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_primitive_types() {
        assert_eq!(format_type(&TypeId::Primitive(Primitive::Int)), "Int");
        assert_eq!(format_type(&TypeId::Primitive(Primitive::Float)), "Float");
        assert_eq!(format_type(&TypeId::Primitive(Primitive::Bool)), "Bool");
    }

    #[test]
    fn format_composite_types() {
        let list_int = TypeId::List(Box::new(TypeId::Primitive(Primitive::Int)));
        assert_eq!(format_type(&list_int), "[Int]");

        let map_str_int = TypeId::Map(
            Box::new(TypeId::Primitive(Primitive::String)),
            Box::new(TypeId::Primitive(Primitive::Int)),
        );
        assert_eq!(format_type(&map_str_int), "{String: Int}");
    }

    #[test]
    fn format_function_types() {
        let fn_ty = TypeId::Func(
            vec![
                TypeId::Primitive(Primitive::Int),
                TypeId::Primitive(Primitive::Int),
            ],
            Box::new(TypeId::Primitive(Primitive::Int)),
        );
        assert_eq!(format_type(&fn_ty), "fn(Int, Int) -> Int");
    }

    #[test]
    fn is_numeric_checks() {
        assert!(TypeId::Primitive(Primitive::Int).is_numeric());
        assert!(TypeId::Primitive(Primitive::Float).is_numeric());
        assert!(!TypeId::Primitive(Primitive::Bool).is_numeric());
        assert!(!TypeId::Primitive(Primitive::String).is_numeric());
    }

    #[test]
    fn is_concrete_checks() {
        assert!(TypeId::Primitive(Primitive::Int).is_concrete());
        assert!(!TypeId::TypeVar(TypeVarId(0)).is_concrete());

        let list_var = TypeId::List(Box::new(TypeId::TypeVar(TypeVarId(0))));
        assert!(!list_var.is_concrete());

        let list_int = TypeId::List(Box::new(TypeId::Primitive(Primitive::Int)));
        assert!(list_int.is_concrete());
    }
}
