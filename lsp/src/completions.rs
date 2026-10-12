use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};
use xenomorph_common::semantic::{OwnedAnnotation, OwnedConstraint, OwnedSemanticType};

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

pub fn create_owned_annotation_completion_item(annotation: &OwnedAnnotation) -> CompletionItem {
    let signature = format_owned_annotation_signature(annotation);

    CompletionItem {
        label: annotation.name.to_string(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(signature.clone()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format_owned_annotation_documentation(annotation, &signature),
        })),
        ..Default::default()
    }
}

pub fn format_owned_annotation_documentation(
    annotation: &OwnedAnnotation,
    signature: &str,
) -> String {
    let mut documentation = format!("```xenomorph\n{}\n```", signature);

    if let Some(target) = annotation.target_parameter() {
        documentation.push_str(&format!(
            "\n\n**Applicable {}:** `{}`",
            match target.constraint {
                OwnedConstraint::Type(_) => "type",
                OwnedConstraint::Trait(_) => "trait",
            },
            target.constraint.name()
        ));
    }

    if let Some(body) = &annotation.documentation {
        documentation.push_str("\n\n");
        documentation.push_str(body);
    }

    documentation
}

pub fn format_owned_annotation_signature(annotation: &OwnedAnnotation) -> String {
    let explicit_parameters = annotation.explicit_parameters();
    let params = explicit_parameters
        .iter()
        .enumerate()
        .map(|(index, param)| {
            if annotation.variadic && index + 1 == explicit_parameters.len() {
                format!("...{}", format_constraint(&param.constraint))
            } else {
                format_constraint(&param.constraint)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!("@{}({})", annotation.name, params)
}

fn format_constraint(required: &OwnedConstraint) -> String {
    required.name().to_string()
}

pub fn create_owned_type_completion_item(t: &OwnedSemanticType) -> CompletionItem {
    create_completion_item(
        &t.name,
        t.documentation.as_deref(),
        if t.name.contains("color") || t.name.contains("Color") {
            CompletionItemKind::COLOR
        } else {
            CompletionItemKind::CLASS
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::semantic::{
        MetadataGraph, XenoAnnotation, XenoAnnotationKind, XenoConstraint, XenoParam,
        ANY_TARGET_PARAM, EXPRESSION,
    };

    static FIRST: XenoParam = XenoParam {
        name: "first",
        constraint: XenoConstraint::Trait(&EXPRESSION),
    };
    static REST: XenoParam = XenoParam {
        name: "rest",
        constraint: XenoConstraint::Trait(&EXPRESSION),
    };
    static VARIADIC: XenoAnnotation = XenoAnnotation {
        name: "annotation",
        documentation: None,
        kind: XenoAnnotationKind::Meta,
        params: &[&ANY_TARGET_PARAM, &FIRST, &REST],
        variadic: true,
    };

    fn format_annotation_signature(annotation: &XenoAnnotation) -> String {
        let graph = MetadataGraph::from_descriptors(&[], &[annotation]);
        format_owned_annotation_signature(&graph.annotations[annotation.name])
    }

    #[test]
    fn variadic_annotation_signature_marks_only_the_repeated_parameter() {
        assert_eq!(
            format_annotation_signature(&VARIADIC),
            "@annotation(Expression, ...Expression)"
        );
    }

    #[test]
    fn lombok_signature_uses_concise_variadic_notation() {
        static LOMBOK: XenoParam = XenoParam {
            name: "decorator",
            constraint: XenoConstraint::Trait(&EXPRESSION),
        };
        static ANNOTATION: XenoAnnotation = XenoAnnotation {
            name: "Lombok",
            documentation: None,
            kind: XenoAnnotationKind::Meta,
            params: &[&ANY_TARGET_PARAM, &LOMBOK],
            variadic: true,
        };

        assert_eq!(
            format_annotation_signature(&ANNOTATION),
            "@Lombok(...Expression)"
        );
    }

    #[test]
    fn owned_annotation_completion_uses_copied_metadata() {
        let graph = MetadataGraph::from_descriptors(&[], &[&VARIADIC]);
        let annotation = graph.annotations["annotation"].clone();
        drop(graph);
        let completion = create_owned_annotation_completion_item(&annotation);
        assert_eq!(
            completion.detail.as_deref(),
            Some("@annotation(Expression, ...Expression)")
        );
        assert_eq!(completion.label, "annotation");
        assert!(format_owned_annotation_documentation(&annotation, "")
            .contains("**Applicable type:** `any`"));
    }
}
