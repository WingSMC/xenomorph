#[allow(dead_code)]
trait Node {
    fn get_src(&self) -> &str;
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub enum RootNode {
    Import,
    Validator,
    TypeDef,
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub struct TypeDef<'src> {
    pub name: &'src str,
    pub value: Expr<'src>,
}

#[derive(Debug, PartialEq, Clone)]
#[allow(dead_code)]
pub enum Expr<'src> {
    TypeDef {
        name: &'src str,
        value: Box<Expr<'src>>,
    },
    List(Vec<Expr<'src>>),
    Set(Vec<Expr<'src>>),
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


use std::fmt;
impl<'src> fmt::Display for Expr<'src> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        macro_rules! expr_fmt {
            ($content: expr, $str: expr, $mapper: expr) => {
                {
                    let content_str = $content
                        .iter()
                        .map($mapper)
                        .collect::<Vec<String>>()
                        .join(", ");

                    write!(f, $str, content_str)
                }
            }
        }


        match self {
            Expr::List(elements) => expr_fmt!(elements, "[{}]", |e| e.to_string()),
            Expr::Set(elements) => expr_fmt!(elements, "set [{}]", |e| e.to_string()),
            Expr::Struct(fields) => expr_fmt!(fields, "{{{}}}", |(name, value)| format!("{}: {}", name, value)),
            Expr::Enum(variants) => expr_fmt!(variants, "enum {{{}}}", |(name, value)| format!("{}: {}", name, value)),

            Expr::TypeDef { name, value } => write!(f, "type {} = {}", name, value),
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

