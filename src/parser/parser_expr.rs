use crate::tokens::TokenData;

pub type BinaryExpr<'src> = Box<(Expr<'src>, Expr<'src>)>;
pub type KeyValExpr<'src> = (&'src TokenData<'src>, AnonymType<'src>);
pub type AnonymType<'src> = Vec<Expr<'src>>;
pub type TypeList<'src> = Vec<AnonymType<'src>>;

#[derive(Debug, Clone, PartialEq)]
pub enum Declaration<'src> {
    TypeDecl {
        name: &'src TokenData<'src>,
        t: Vec<Expr<'src>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberType<'src> {
    Int(i64, &'src TokenData<'src>),
    Float(f64, &'src TokenData<'src>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal<'src> {
    Number(NumberType<'src>),
    String(String, &'src TokenData<'src>),
    Boolean(bool, &'src TokenData<'src>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryExprType {
    Union,
    Intersection,
    Difference,
    SymmetricDifference,
    Or,
    Xor,
    Range,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr<'src> {
    Identifier(&'src TokenData<'src>),
    Literal(Literal<'src>),
    Regex(&'src TokenData<'src>),
    Annotation(&'src TokenData<'src>, TypeList<'src>),
    Not(Box<Expr<'src>>),
    FieldAccess(&'src TokenData<'src>),
    BinaryExpr(BinaryExprType, BinaryExpr<'src>),

    List(TypeList<'src>),
    Set(TypeList<'src>),
    Struct(Vec<KeyValExpr<'src>>),
    Enum(Vec<KeyValExpr<'src>>),
}


use std::fmt;
impl<'src> fmt::Display for Declaration<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Declaration::TypeDecl { name, t } => {
                write!(f, "type {} = ", name.v)?;
                for (i, item) in t.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "")
            }
        }
    }
}

impl<'src> fmt::Display for NumberType<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NumberType::Int(n, _) => write!(f, "{}: Int", n),
            NumberType::Float(x, _) => write!(f, "{}: Float", x),
        }
    }
}

impl<'src> fmt::Display for Literal<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Number(num) => write!(f, "{}", num),
            Literal::String(s, _) => write!(f, "\"{}\"", s),
            Literal::Boolean(b, _) => write!(f, "{}", b),
        }
    }
}

impl<'src> fmt::Display for BinaryExprType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryExprType::Union => write!(f, "+"),
            BinaryExprType::Intersection => write!(f, "*"),
            BinaryExprType::Difference => write!(f, "-"),
            BinaryExprType::SymmetricDifference => write!(f, "<>"),
            BinaryExprType::Or => write!(f, "|"),
            BinaryExprType::Xor => write!(f, "^"),
            BinaryExprType::Range => write!(f, ".."),
            BinaryExprType::Add => write!(f, "+"),
            BinaryExprType::Remove => write!(f, "-"),
        }
    }
}

impl<'src> fmt::Display for Expr<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Identifier(token) => write!(f, "{}", token.v),
            Expr::Literal(lit) => write!(f, "{}", lit),

            Expr::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?
                    }
                    format_vector_expr(f, item)?;
                }
                write!(f, "]")
            }

            Expr::Struct(fields) => {
                write!(f, "{{\n")?;
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?
                    }
                    write!(f, "{}:", key.v)?;
                    format_vector_expr(f, value)?;
                    write!(f, ",\n")?;
                }
                write!(f, "}}\n")
            }

            Expr::Enum(variants) => {
                write!(f, "enum {{")?;
                for (i, (key, value)) in variants.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?
                    }
                    write!(f, "{}({:?})", key.v, value)?;
                }
                write!(f, "}}")
            }

            Expr::BinaryExpr(t, binary_exp) => format_binary_expr(f, t, binary_exp),
            Expr::Annotation(id, params) => {
                write!(f, "@{}(", id.v)?;
                for (i, item) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?
                    }
                    format_vector_expr(f, item)?;
                }
                write!(f, ")")
            }

            Expr::Regex(token) => write!(f, "{}", token.v),
            Expr::Not(expr) => write!(f, "!{}", expr),
            Expr::FieldAccess(token) => write!(f, "${}", token.v),
            Expr::Set(items) => {
                write!(f, "{{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?
                    }
                    format_vector_expr(f, item)?;
                }
                write!(f, "}}")
            }
        }
    }
}

fn format_binary_expr<'src>(
    f: &mut fmt::Formatter<'_>,
    t: &BinaryExprType,
    expr: &BinaryExpr,
) -> fmt::Result {
    write!(f, "({} {} {})", expr.0, t, expr.1)
}

fn format_vector_expr<'src>(f: &mut fmt::Formatter<'_>, expr: &Vec<Expr<'src>>) -> fmt::Result {
    for (_, item) in expr.iter().enumerate() {
        write!(f, " {}", item)?;
    }
    write!(f, "")
}
 