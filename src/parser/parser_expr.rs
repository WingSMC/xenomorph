use crate::tokens::TokenData;
use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum Declaration<'src> {
    TypeDecl {
        name: &'src TokenData<'src>,
        t: Vec<Expr<'src>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum NumberType<'src> {
    Int(i64, &'src TokenData<'src>),
    Float(f64, &'src TokenData<'src>),
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub enum Literal<'src> {
    Number(NumberType<'src>),
    String(String, &'src TokenData<'src>),
    Boolean(bool, &'src TokenData<'src>),
}

type BinaryExpr<'src> = Box<(Expr<'src>, Expr<'src>)>;
type KeyValExpr<'src> = (&'src TokenData<'src>, SimpleType<'src>);
type SimpleType<'src> = Vec<Expr<'src>>;

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub enum Expr<'src> {
    Identifier(&'src TokenData<'src>),
    Literal(Literal<'src>),

    List(Vec<SimpleType<'src>>),
    Struct(Vec<KeyValExpr<'src>>),
    Enum(Vec<KeyValExpr<'src>>),

    Union(BinaryExpr<'src>),
    Intersection(BinaryExpr<'src>),
    Difference(BinaryExpr<'src>),
    SymmetricDifference(BinaryExpr<'src>),
    Or(BinaryExpr<'src>),

    Annotation(&'src TokenData<'src>, SimpleType<'src>),
}

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

fn format_binary_expr<'src>(
    f: &mut fmt::Formatter<'_>,
    op: &str,
    expr: &BinaryExpr,
) -> fmt::Result {
    write!(f, "({} {} {})", expr.0, op, expr.1)
}

fn format_vector_expr<'src>(f: &mut fmt::Formatter<'_>, expr: &Vec<Expr<'src>>) -> fmt::Result {
    for (_, item) in expr.iter().enumerate() {
        write!(f, " {}", item)?;
    }
    write!(f, "")
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

            Expr::Union(binary_exp) => format_binary_expr(f, "+", binary_exp),
            Expr::Intersection(binary_exp) => format_binary_expr(f, "&", binary_exp),
            Expr::Difference(binary_exp) => format_binary_expr(f, "\\", binary_exp),
            Expr::SymmetricDifference(binary_exp) => format_binary_expr(f, "<>", binary_exp),
            Expr::Or(binary_exp) => format_binary_expr(f, "|", binary_exp),
            Expr::Annotation(id, params) => {
                write!(f, "@{}(", id.v)?;
                format_vector_expr(f, params)?;
                write!(f, ")")
            }
        }
    }
}
