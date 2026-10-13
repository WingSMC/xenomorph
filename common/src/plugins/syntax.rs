use crate::{parser::*, TokenData};
/// Validates source references before plugin listeners can resolve text.
pub fn validate(source: &str, ast: &[Declaration]) -> Result<(), String> {
    for token in source_tokens(ast) {
        if token.start > token.end
            || source
                .get(token.start as usize..token.end as usize)
                .is_none()
        {
            return Err("Invalid plugin source range".into());
        }
    }
    fn simple(s: &SimpleType) -> Result<(), String> {
        match s {
            SimpleType::Literal(l) => literal(l),
            SimpleType::Optional(s) => simple(s),
            SimpleType::Identifier(_, args) | SimpleType::Array(_, args) => {
                for s in args.as_deref().unwrap_or_default() {
                    simple(s)?;
                }
                Ok(())
            }
        }
    }
    fn literal(l: &Literal) -> Result<(), String> {
        match l {
            Literal::Int(i)
                if (matches!(i.representation.size, IntegerSize::Bits(0))
                    || !integer_fits(&i.value, i.representation)) =>
            {
                return Err("Invalid integer representation".into());
            }
            Literal::Float(f)
                if (!f.representation.signed
                    || !decimal_roundtrips_as(&f.value, f.representation.size)) =>
            {
                return Err("Invalid float representation".into());
            }
            _ => {}
        }
        Ok(())
    }
    fn ty(t: &Type) -> Result<(), String> {
        match t {
            Type::Simple(s) => simple(s)?,
            Type::Tuple(v) | Type::Sum(v) | Type::Intersection(v) => {
                for s in v {
                    simple(s)?
                }
            }
            Type::Struct(v) | Type::Enum(v) => {
                for (_, s, _) in v {
                    simple(s)?
                }
            }
            Type::Set(s) => {
                if let Some(e) = &s.element_type {
                    simple(e)?
                }
                for l in s.values.as_deref().unwrap_or_default() {
                    literal(l)?
                }
            }
        }
        Ok(())
    }
    fn annotation(a: &Annotation) -> Result<(), String> {
        for e in &a.params {
            match e {
                Expr::Type(t) => ty(t)?,
                Expr::Annotation(a) => annotation(a)?,
                Expr::Regex(_) => {}
            }
        }
        Ok(())
    }
    for d in ast {
        if let Declaration::Type { ty: t, .. } = d {
            ty(&t.0)?;
            for a in &t.1 {
                annotation(a)?
            }
        }
    }
    Ok(())
}
pub fn source_tokens(ast: &[Declaration]) -> Vec<TokenData> {
    fn simple(ty: &SimpleType, out: &mut Vec<TokenData>) {
        match ty {
            SimpleType::Optional(inner) => simple(inner, out),
            SimpleType::Identifier(token, args) | SimpleType::Array(token, args) => {
                out.push(*token);
                for arg in args.as_deref().unwrap_or_default() {
                    simple(arg, out);
                }
            }
            SimpleType::Literal(lit) => {
                out.push(lit.token());
                if let Some(cast) = lit.cast_target() {
                    out.push(cast);
                }
            }
        }
    }
    fn ty(t: &Type, out: &mut Vec<TokenData>) {
        match t {
            Type::Simple(s) => simple(s, out),
            Type::Tuple(items) | Type::Sum(items) | Type::Intersection(items) => {
                for item in items {
                    simple(item, out)
                }
            }
            Type::Struct(fields) | Type::Enum(fields) => {
                for (key, value, docs) in fields {
                    out.push(*key);
                    simple(value, out);
                    if let Some(d) = docs {
                        out.push(*d)
                    }
                }
            }
            Type::Set(set) => {
                out.push(set.keyword);
                out.push(set.last_token);
                if let Some(e) = &set.element_type {
                    simple(e, out)
                }
                for l in set.values.as_deref().unwrap_or_default() {
                    out.push(l.token());
                    if let Some(c) = l.cast_target() {
                        out.push(c)
                    }
                }
            }
        }
    }
    fn annotation(a: &Annotation, out: &mut Vec<TokenData>) {
        out.push(a.ident);
        for e in &a.params {
            match e {
                Expr::Regex(t) => out.push(*t),
                Expr::Type(t) => ty(t, out),
                Expr::Annotation(a) => annotation(a, out),
            }
        }
    }
    let mut out = vec![];
    for d in ast {
        match d {
            Declaration::Import { location, .. } => out.push(*location),
            Declaration::Type {
                name,
                generics,
                ty: t,
                from,
                to,
                ..
            } => {
                out.extend([*name, *from, *to]);
                for (n, c) in generics.as_deref().unwrap_or_default() {
                    out.push(*n);
                    if let Some(c) = c {
                        out.push(*c)
                    }
                }
                ty(&t.0, &mut out);
                for a in &t.1 {
                    annotation(a, &mut out)
                }
            }
            Declaration::Custom { name, .. } => {
                if let Some(name) = name {
                    out.push(*name)
                }
            }
        }
    }
    out
}
