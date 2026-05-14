use std::sync::LazyLock;

use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};
use xenomorph_common::semantic::{
    XenoAnnotation, XenoParameterType, XenoType, BUILTIN_ANNOTATIONS, BUILTIN_TYPES,
};

pub fn create_completion_item(
    label: &str,
    detail: Option<&str>,
    kind: CompletionItemKind,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: detail.map(|d| d.to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: detail.map(|d| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: d.to_string(),
            })
        }),
        ..Default::default()
    }
}

pub static BUILTIN_ANNOTATION_COMPLETIONS: LazyLock<Vec<CompletionItem>> = LazyLock::new(|| {
    BUILTIN_ANNOTATIONS
        .iter()
        .map(|annotation| create_annotation_completion_item(annotation))
        .collect()
});

fn create_annotation_completion_item(annotation: &XenoAnnotation) -> CompletionItem {
    let signature = format_annotation_signature(annotation);

    CompletionItem {
        label: annotation.name.to_string(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(signature.clone()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format_annotation_documentation(annotation, &signature),
        })),
        ..Default::default()
    }
}

pub fn format_annotation_documentation(annotation: &XenoAnnotation, signature: &str) -> String {
    let mut documentation = format!("```xenomorph\n{}\n```", signature);

    if let Some(applicable_to) = annotation.applicable_to {
        documentation.push_str(&format!(
            "\n\n**Applicable to:** `{}`",
            format_type_list(applicable_to).join("` | `")
        ));
    }

    if let Some(body) = annotation.documentation {
        documentation.push_str("\n\n");
        documentation.push_str(body);
    }

    documentation
}

pub fn format_annotation_signature(annotation: &XenoAnnotation) -> String {
    let params = annotation
        .params
        .unwrap_or(&[])
        .iter()
        .map(|param| {
            format!(
                "{}: {}",
                param.name,
                format_parameter_type(param.param_type)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!("@{}({})", annotation.name, params)
}

fn format_parameter_type(parameter_type: XenoParameterType) -> String {
    match parameter_type {
        XenoParameterType::None => "never".to_string(),
        XenoParameterType::NumberLiteral => "number".to_string(),
        XenoParameterType::IntegerLiteral => "integer".to_string(),
        XenoParameterType::StringLiteral => "string".to_string(),
        XenoParameterType::BoolLiteral => "bool".to_string(),
        XenoParameterType::FieldReference => "field reference".to_string(),
        XenoParameterType::AnyLiteral => "literal".to_string(),
        XenoParameterType::Expression => "expression".to_string(),
        XenoParameterType::Identifier => "identifier".to_string(),
        XenoParameterType::Type => "type".to_string(),
        XenoParameterType::Annotation => "annotation".to_string(),
        XenoParameterType::List(item_types) => format!(
            "[{}]",
            item_types
                .iter()
                .map(|item_type| format_parameter_type(*item_type))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn format_type_list(types: &[&XenoType]) -> Vec<&'static str> {
    types.iter().map(|xeno_type| xeno_type.name).collect()
}

pub static BUILTIN_TYPE_COMPLETIONS: LazyLock<Vec<CompletionItem>> = LazyLock::new(|| {
    BUILTIN_TYPES
        .iter()
        .map(|t| {
            create_completion_item(
                t.name,
                t.documentation,
                if t.name.contains("color") || t.name.contains("Color") {
                    CompletionItemKind::COLOR
                } else {
                    CompletionItemKind::CLASS
                },
            )
        })
        .collect()
});
