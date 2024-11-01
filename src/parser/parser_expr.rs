#[derive(Debug, PartialEq, Clone)]
pub enum Expr<'src> {
    TypeDef {
        name: &'src str,
        t: Box<Expr<'src>>,
    },
    List(Vec<Expr<'src>>),
    Struct(Vec<(String, Expr<'src>)>),

    Enum(Vec<(String, Expr<'src>)>),
    Identifier(&'src str),
    Number(i64),
    StringLiteral(&'src str),
    Boolean(bool),
    Union(Box<Expr<'src>>, Box<Expr<'src>>),
    Intersection(Box<Expr<'src>>, Box<Expr<'src>>),
    Difference(Box<Expr<'src>>, Box<Expr<'src>>),
    SymmetricDifference(Box<Expr<'src>>, Box<Expr<'src>>),
}

pub enum ComplexType {
    List,
    Struct,
    Enum,
}

use std::fmt;
impl<'src> fmt::Display for Expr<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::TypeDef { name, t } => write!(f, "type {} = {}", name, t),
            Expr::List(elements) => {
                let elements_str = elements
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<String>>()
                    .join(", ");
                write!(f, "[{}]", elements_str)
            }
            Expr::Struct(fields) => {
                let fields_str = fields
                    .iter()
                    .map(|(name, value)| format!("{}: {}", name, value))
                    .collect::<Vec<String>>()
                    .join(", ");
                write!(f, "{{{}}}", fields_str)
            }
            Expr::Enum(variants) => {
                let variants_str = variants
                    .iter()
                    .map(|(name, value)| format!("{}: {}", name, value))
                    .collect::<Vec<String>>()
                    .join(", ");
                write!(f, "enum {{{}}}", variants_str)
            }
            Expr::Identifier(ident) => write!(f, "{}", ident),
            Expr::Number(num) => write!(f, "{}", num),
            Expr::StringLiteral(s) => write!(f, "\"{}\"", s),
            Expr::Boolean(b) => write!(f, "{}", b),
            Expr::Union(left, right) => write!(f, "{} | {}", left, right),
            Expr::Intersection(left, right) => write!(f, "{} * {}", left, right),
            Expr::Difference(left, right) => write!(f, "{} \\ {}", left, right),
            Expr::SymmetricDifference(left, right) => write!(f, "{} <> {}", left, right),
        }
    }
}
