use std::collections::HashSet;

use crate::{
    parser::{Annotation, Declaration, Expr, Literal, SimpleType, Type, XenoType},
    TokenData,
};

fn simple_references(ty: &SimpleType, references: &mut Vec<TokenData>) {
    match ty {
        SimpleType::Identifier(token, arguments) | SimpleType::Array(token, arguments) => {
            references.push(*token);
            for argument in arguments.as_deref().unwrap_or_default() {
                simple_references(argument, references);
            }
        }
        SimpleType::Optional(inner) => simple_references(inner, references),
        SimpleType::Literal(literal) => literal_references(literal, references),
    }
}

fn literal_references(literal: &Literal, references: &mut Vec<TokenData>) {
    if let Some(token) = literal.cast_target() {
        references.push(token);
    }
}

fn type_references(ty: &Type, references: &mut Vec<TokenData>) {
    match ty {
        Type::Simple(simple) => simple_references(simple, references),
        Type::Tuple(types) | Type::Sum(types) | Type::Intersection(types) => {
            for ty in types {
                simple_references(ty, references);
            }
        }
        Type::Struct(fields) | Type::Enum(fields) => {
            for (_, ty, _) in fields {
                simple_references(ty, references);
            }
        }
        Type::Set(set) => {
            if let Some(ty) = &set.element_type {
                simple_references(ty, references);
            }
            for literal in set.values.as_deref().unwrap_or_default() {
                literal_references(literal, references);
            }
        }
    }
}

fn annotation_references(annotation: &Annotation, references: &mut Vec<TokenData>) {
    for parameter in &annotation.params {
        match parameter {
            Expr::Type(ty) => type_references(ty, references),
            Expr::Annotation(annotation) => annotation_references(annotation, references),
            Expr::Regex(_) => {}
        }
    }
}

fn all_type_references((ty, annotations): &XenoType) -> Vec<TokenData> {
    let mut references = vec![];
    type_references(ty, &mut references);
    for annotation in annotations {
        annotation_references(annotation, &mut references);
    }
    references
}

/// References in a type expression, excluding keys, literals, and annotation names.
pub fn collect_type_references(
    ty: &XenoType,
    name: &str,
    references: &mut Vec<TokenData>,
    source: &str,
) {
    references.extend(
        all_type_references(ty)
            .into_iter()
            .filter(|token| token.text(source) == name),
    );
}

fn declaration_references(declaration: &Declaration, source: &str) -> Vec<TokenData> {
    let Declaration::Type { ty, generics, .. } = declaration else {
        return vec![];
    };
    let generics = generics.as_deref().unwrap_or_default();
    let mut references: Vec<_> = generics
        .iter()
        .filter_map(|(_, constraint)| *constraint)
        .collect();
    references.extend(all_type_references(ty).into_iter().filter(|token| {
        !generics
            .iter()
            .any(|(parameter, _)| parameter.text(source) == token.text(source))
    }));
    references
}

pub fn declaration_dependencies(declaration: &Declaration, source: &str) -> HashSet<String> {
    declaration_references(declaration, source)
        .into_iter()
        .map(|token| token.text(source).into())
        .collect()
}

/// Type references and import selections with declaration-local generics excluded.
pub fn collect_static_type_references(
    ast: &[Declaration],
    name: &str,
    source: &str,
) -> Vec<TokenData> {
    let mut references = vec![];
    for declaration in ast {
        if let Declaration::Import { names, .. } = declaration {
            references.extend(
                names
                    .iter()
                    .filter(|token| token.text(source) == name)
                    .copied(),
            );
        }
        references.extend(
            declaration_references(declaration, source)
                .into_iter()
                .filter(|token| token.text(source) == name),
        );
    }
    references
}

pub fn collect_source_type_occurrences(
    ast: &[Declaration],
    name: &str,
    source: &str,
) -> Vec<TokenData> {
    let mut references = collect_static_type_references(ast, name, source);
    references.extend(ast.iter().filter_map(|declaration| match declaration {
        Declaration::Type { name: token, .. } if token.text(source) == name => Some(*token),
        _ => None,
    }));
    references
}
