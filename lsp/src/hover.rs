use std::collections::HashMap;

use xenomorph_common::{
    parser::{Annotation, Declaration, Expr, SimpleType, Type},
    TokenData,
};

/// Finds the generic arguments attached to the exact type reference under the
/// cursor and converts them to source-like Xenomorph text.
pub fn type_arguments_at_token(
    ast: &[Declaration<'_>],
    target: &TokenData<'_>,
) -> Option<Vec<String>> {
    ast.iter().find_map(|declaration| match declaration {
        Declaration::Type { ty, .. } => find_arguments_in_xeno_type(ty, target),
        Declaration::Import { .. } | Declaration::Custom { .. } => None,
    })
}

/// Formats one declaration layer for hover display. Supplied generic arguments
/// replace the declaration's generic parameters throughout the rendered body.
pub fn format_type_declaration(
    declaration: &Declaration<'_>,
    supplied_arguments: &[String],
) -> Option<String> {
    let Declaration::Type {
        name, generics, ty, ..
    } = declaration
    else {
        return None;
    };

    let generics = generics.as_deref().unwrap_or(&[]);
    let substitutions = generics
        .iter()
        .zip(supplied_arguments)
        .map(|((name, _), argument)| (name.v, argument.as_str()))
        .collect::<HashMap<_, _>>();

    let generic_display = format_declaration_generics(generics, supplied_arguments);
    let body = format_type(&ty.0, &substitutions);
    let annotations = if ty.1.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            ty.1.iter()
                .map(|annotation| format_annotation(annotation, &substitutions))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };

    Some(format!(
        "type {}{generic_display} = {body}{annotations};",
        name.v
    ))
}

fn format_declaration_generics(
    generics: &[(&TokenData<'_>, Option<&TokenData<'_>>)],
    supplied_arguments: &[String],
) -> String {
    if generics.is_empty() {
        return String::new();
    }

    format!(
        "<{}>",
        generics
            .iter()
            .enumerate()
            .map(|(index, (name, constraint))| {
                supplied_arguments.get(index).cloned().unwrap_or_else(|| {
                    constraint.map_or_else(
                        || name.v.to_string(),
                        |constraint| format!("{}: {}", name.v, constraint.v),
                    )
                })
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn find_arguments_in_xeno_type(
    ty: &(Type<'_>, Vec<Annotation<'_>>),
    target: &TokenData<'_>,
) -> Option<Vec<String>> {
    find_arguments_in_type(&ty.0, target).or_else(|| {
        ty.1.iter()
            .find_map(|annotation| find_arguments_in_annotation(annotation, target))
    })
}

fn find_arguments_in_type(ty: &Type<'_>, target: &TokenData<'_>) -> Option<Vec<String>> {
    match ty {
        Type::Simple(simple) => find_arguments_in_simple_type(simple, target),
        Type::Tuple(items) | Type::Sum(items) | Type::Intersection(items) => items
            .iter()
            .find_map(|simple| find_arguments_in_simple_type(simple, target)),
        Type::Set(set) => set
            .element_type
            .as_ref()
            .and_then(|element| find_arguments_in_simple_type(element, target)),
        Type::Struct(fields) | Type::Enum(fields) => fields
            .iter()
            .find_map(|(_, simple, _)| find_arguments_in_simple_type(simple, target)),
    }
}

fn find_arguments_in_simple_type(
    simple: &SimpleType<'_>,
    target: &TokenData<'_>,
) -> Option<Vec<String>> {
    let (identifier, arguments) = match simple {
        SimpleType::Identifier(identifier, arguments)
        | SimpleType::OptionalIdentifier(identifier, arguments)
        | SimpleType::Array(identifier, arguments)
        | SimpleType::OptionalArray(identifier, arguments) => (*identifier, arguments.as_deref()),
        SimpleType::Literal(_) | SimpleType::OptionalLiteral(_) => return None,
    };

    if same_token(identifier, target) {
        return Some(
            arguments
                .unwrap_or(&[])
                .iter()
                .map(|argument| format_simple_type(argument, &HashMap::new()))
                .collect(),
        );
    }

    arguments
        .unwrap_or(&[])
        .iter()
        .find_map(|argument| find_arguments_in_simple_type(argument, target))
}

fn find_arguments_in_annotation(
    annotation: &Annotation<'_>,
    target: &TokenData<'_>,
) -> Option<Vec<String>> {
    annotation
        .params
        .iter()
        .find_map(|expression| match expression {
            Expr::Regex(_) => None,
            Expr::Annotation(annotation) => find_arguments_in_annotation(annotation, target),
            Expr::Type(ty) => find_arguments_in_type(ty, target),
        })
}

fn same_token(left: &TokenData<'_>, right: &TokenData<'_>) -> bool {
    std::ptr::eq(left, right)
        || left.l == right.l && left.c == right.c && left.v.len() == right.v.len()
}

fn format_type(ty: &Type<'_>, substitutions: &HashMap<&str, &str>) -> String {
    match ty {
        Type::Simple(simple) => format_simple_type(simple, substitutions),
        Type::Tuple(items) => format!("[{}]", format_simple_types(items, substitutions, ", ")),
        Type::Set(set) => {
            let element = set
                .element_type
                .as_ref()
                .map(|element| format!("<{}>", format_simple_type(element, substitutions)))
                .unwrap_or_default();
            let values = set.values.as_ref().map_or_else(String::new, |values| {
                format!(
                    "[{}]",
                    values
                        .iter()
                        .map(|literal| literal.source_text())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });
            format!("set{element}{values}")
        }
        Type::Struct(fields) => format_fields("", fields, substitutions),
        Type::Enum(fields) => format_fields("enum ", fields, substitutions),
        Type::Sum(items) => format!("| {}", format_simple_types(items, substitutions, " | ")),
        Type::Intersection(items) => {
            format!("& {}", format_simple_types(items, substitutions, " & "))
        }
    }
}

fn format_fields(
    prefix: &str,
    fields: &[(&TokenData<'_>, SimpleType<'_>, Option<&TokenData<'_>>)],
    substitutions: &HashMap<&str, &str>,
) -> String {
    if fields.is_empty() {
        return format!("{prefix}{{}}");
    }

    let fields = fields
        .iter()
        .map(|(name, ty, _)| format!("  {}: {},", name.v, format_simple_type(ty, substitutions)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{prefix}{{\n{fields}\n}}")
}

fn format_simple_types(
    types: &[SimpleType<'_>],
    substitutions: &HashMap<&str, &str>,
    separator: &str,
) -> String {
    types
        .iter()
        .map(|ty| format_simple_type(ty, substitutions))
        .collect::<Vec<_>>()
        .join(separator)
}

fn format_simple_type(ty: &SimpleType<'_>, substitutions: &HashMap<&str, &str>) -> String {
    match ty {
        SimpleType::Literal(literal) => literal.source_text(),
        SimpleType::OptionalLiteral(literal) => format!("?{}", literal.source_text()),
        SimpleType::Identifier(identifier, arguments) => {
            format_named_type(identifier.v, arguments.as_deref(), substitutions)
        }
        SimpleType::OptionalIdentifier(identifier, arguments) => format!(
            "?{}",
            format_named_type(identifier.v, arguments.as_deref(), substitutions)
        ),
        SimpleType::Array(identifier, arguments) => format!(
            "{}[]",
            format_named_type(identifier.v, arguments.as_deref(), substitutions)
        ),
        SimpleType::OptionalArray(identifier, arguments) => format!(
            "?{}[]",
            format_named_type(identifier.v, arguments.as_deref(), substitutions)
        ),
    }
}

fn format_named_type(
    name: &str,
    arguments: Option<&[SimpleType<'_>]>,
    substitutions: &HashMap<&str, &str>,
) -> String {
    if arguments.is_none() {
        if let Some(substitution) = substitutions.get(name) {
            return (*substitution).to_string();
        }
    }

    match arguments {
        None => name.to_string(),
        Some(arguments) => format!(
            "{name}<{}>",
            format_simple_types(arguments, substitutions, ", ")
        ),
    }
}

fn format_annotation(annotation: &Annotation<'_>, substitutions: &HashMap<&str, &str>) -> String {
    if annotation.params.is_empty() {
        return format!("@{}", annotation.ident.v);
    }

    format!(
        "@{}({})",
        annotation.ident.v,
        annotation
            .params
            .iter()
            .map(|expression| format_expression(expression, substitutions))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn format_expression(expression: &Expr<'_>, substitutions: &HashMap<&str, &str>) -> String {
    match expression {
        Expr::Regex(token) => token.v.to_string(),
        Expr::Annotation(annotation) => format_annotation(annotation, substitutions),
        Expr::Type(ty) => format_type(ty, substitutions),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::{lexer::Lexer, parser::Parser};

    fn parse(source: &str) -> Vec<Declaration<'_>> {
        let tokens = Box::leak(Box::new(
            Lexer::tokenize(Box::leak(source.to_string().into_boxed_str()))
                .expect("hover fixture should lex"),
        ));
        let (ast, diagnostics) = Parser::parse(tokens);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:#?}"
        );
        ast
    }

    #[test]
    fn specialized_hover_substitutes_generics_in_one_declaration_layer() {
        let ast = parse(
            "type Ty<T> = { field1: T, field2: Ty2[], nested: dict<string, T> } @ann1 @ann2(P1, P2, \"asd\");",
        );

        assert_eq!(
            format_type_declaration(&ast[0], &["string".to_string()]).as_deref(),
            Some(
                "type Ty<string> = {\n  field1: string,\n  field2: Ty2[],\n  nested: dict<string, string>,\n} @ann1 @ann2(P1, P2, \"asd\");"
            )
        );
    }

    #[test]
    fn unspecialized_hover_preserves_generic_constraints() {
        let ast = parse("type Ty<T: HasLength> = T @minlen(1);");

        assert_eq!(
            format_type_declaration(&ast[0], &[]).as_deref(),
            Some("type Ty<T: HasLength> = T @minlen(1);")
        );
    }

    #[test]
    fn hover_finds_arguments_on_the_exact_nested_reference() {
        let ast = parse("type Use = dict<Ty<string>, Ty<u8>>;");
        let Declaration::Type { ty, .. } = &ast[0] else {
            panic!("expected a type declaration");
        };
        let Type::Simple(SimpleType::Identifier(_, Some(arguments))) = &ty.0 else {
            panic!("expected a specialized type");
        };
        let SimpleType::Identifier(second_ty, _) = &arguments[1] else {
            panic!("expected a nested type reference");
        };

        assert_eq!(
            type_arguments_at_token(&ast, second_ty),
            Some(vec!["u8".to_string()])
        );
    }
}
