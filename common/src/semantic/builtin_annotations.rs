use crate::semantic::{
    XenoConstraint, ANNOTATION, ANY, BOOL_LITERAL, EXPRESSION, HAS_LENGTH, IDENTIFIER,
    INTEGER_LITERAL, LITERAL, NUMBER_LITERAL, NUMERIC, REGEX_LITERAL, STRING, STRING_LITERAL,
    TYPE_REFERENCE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XenoAnnotationKind {
    Transformation,
    Validation,
    ComplexValidation,
    Meta,
}

#[derive(Debug)]
pub struct XenoParam {
    pub name: &'static str,
    pub constraint: XenoConstraint,
}

#[derive(Debug)]
pub struct XenoAnnotation {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub kind: XenoAnnotationKind,
    /// Parameter zero is the implicit annotated value. Remaining parameters
    /// correspond to arguments written inside the annotation parentheses.
    pub params: &'static [&'static XenoParam],
    /// Repeats the final parameter for additional arguments.
    pub variadic: bool,
}

impl XenoAnnotation {
    pub fn target_parameter(&self) -> Option<&'static XenoParam> {
        self.params.first().copied()
    }

    pub fn explicit_parameters(&self) -> &'static [&'static XenoParam] {
        self.params.get(1..).unwrap_or(&[])
    }

    pub fn parameter_at(&self, index: usize) -> Option<&'static XenoParam> {
        let params = self.explicit_parameters();
        params.get(index).copied().or_else(|| {
            if self.variadic {
                params.last().copied()
            } else {
                None
            }
        })
    }
}

pub static ANY_TARGET_PARAM: XenoParam = XenoParam {
    name: "self",
    constraint: XenoConstraint::Type(&ANY),
};
pub static NUMERIC_TARGET_PARAM: XenoParam = XenoParam {
    name: "self",
    constraint: XenoConstraint::Trait(&NUMERIC),
};
pub static HAS_LENGTH_TARGET_PARAM: XenoParam = XenoParam {
    name: "self",
    constraint: XenoConstraint::Trait(&HAS_LENGTH),
};
pub static STRING_TARGET_PARAM: XenoParam = XenoParam {
    name: "self",
    constraint: XenoConstraint::Type(&STRING),
};
pub static NUMBER_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&NUMBER_LITERAL),
};
pub static INTEGER_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&INTEGER_LITERAL),
};
pub static REGEX_PATTERN_PARAM: XenoParam = XenoParam {
    name: "pattern",
    constraint: XenoConstraint::Trait(&REGEX_LITERAL),
};
pub static EXPRESSION_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&EXPRESSION),
};
pub static CONDITION_PARAM: XenoParam = XenoParam {
    name: "condition",
    constraint: XenoConstraint::Trait(&EXPRESSION),
};

pub static MIN: XenoAnnotation = XenoAnnotation {
    name: "min",
    documentation: Some("Specifies the minimum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: &[&NUMERIC_TARGET_PARAM, &NUMBER_VALUE_PARAM],
    variadic: false,
};

pub static MAX: XenoAnnotation = XenoAnnotation {
    name: "max",
    documentation: Some("Specifies the maximum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: &[&NUMERIC_TARGET_PARAM, &NUMBER_VALUE_PARAM],
    variadic: false,
};

pub static GT: XenoAnnotation = XenoAnnotation {
    name: "gt",
    documentation: Some("Specifies that some numeric value must be greater than the parameter."),
    kind: XenoAnnotationKind::Validation,
    params: &[&NUMERIC_TARGET_PARAM, &NUMBER_VALUE_PARAM],
    variadic: false,
};

pub static LT: XenoAnnotation = XenoAnnotation {
    name: "lt",
    documentation: Some("Specifies that some numeric value must be less than the parameter."),
    kind: XenoAnnotationKind::Validation,
    params: &[&NUMERIC_TARGET_PARAM, &NUMBER_VALUE_PARAM],
    variadic: false,
};

pub static LEN: XenoAnnotation = XenoAnnotation {
    name: "len",
    documentation: Some("Specifies the exact length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: &[&HAS_LENGTH_TARGET_PARAM, &INTEGER_VALUE_PARAM],
    variadic: false,
};

pub static MINLEN: XenoAnnotation = XenoAnnotation {
    name: "minlen",
    documentation: Some("Specifies the minimum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: &[&HAS_LENGTH_TARGET_PARAM, &INTEGER_VALUE_PARAM],
    variadic: false,
};

pub static MAXLEN: XenoAnnotation = XenoAnnotation {
    name: "maxlen",
    documentation: Some("Specifies the maximum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: &[&HAS_LENGTH_TARGET_PARAM, &INTEGER_VALUE_PARAM],
    variadic: false,
};

pub static MATCH: XenoAnnotation = XenoAnnotation {
    name: "match",
    documentation: Some(
        "Requires a string or a type derived from string to match one regular expression.",
    ),
    kind: XenoAnnotationKind::Validation,
    params: &[&STRING_TARGET_PARAM, &REGEX_PATTERN_PARAM],
    variadic: false,
};

pub static IF: XenoAnnotation = XenoAnnotation {
    name: "if",
    documentation: Some("Applies or removes **validation** depending on the condition."),
    kind: XenoAnnotationKind::ComplexValidation,
    params: &[&ANY_TARGET_PARAM, &CONDITION_PARAM, &EXPRESSION_VALUE_PARAM],
    variadic: false,
};

pub static ELSEIF : XenoAnnotation = XenoAnnotation {
	name: "elseif",
	documentation: Some("Applies or removes validation depending on the condition, used after an `@if` or another `@elseif`."),
	kind: XenoAnnotationKind::ComplexValidation,
	params: &[&ANY_TARGET_PARAM, &CONDITION_PARAM, &EXPRESSION_VALUE_PARAM],
    variadic: false,
};

pub static ELSE: XenoAnnotation = XenoAnnotation {
    name: "else",
    documentation: Some(
        "Applies validation if previous `@if` and `@elseif` conditions were not met.",
    ),
    kind: XenoAnnotationKind::ComplexValidation,
    params: &[&ANY_TARGET_PARAM, &EXPRESSION_VALUE_PARAM],
    variadic: false,
};

pub static BUILTIN_ANNOTATIONS: &[&XenoAnnotation] = &[
    &MIN, &MAX, &GT, &LT, &LEN, &MINLEN, &MAXLEN, &MATCH, &IF, &ELSEIF, &ELSE,
];

// Re-export the basic parameter traits from this module's API surface. These
// aliases also keep the trait vocabulary discoverable beside annotations.
pub static STRING_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&STRING_LITERAL),
};
pub static BOOL_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&BOOL_LITERAL),
};
pub static LITERAL_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&LITERAL),
};
pub static IDENTIFIER_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&IDENTIFIER),
};
pub static TYPE_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&TYPE_REFERENCE),
};
pub static ANNOTATION_VALUE_PARAM: XenoParam = XenoParam {
    name: "value",
    constraint: XenoConstraint::Trait(&ANNOTATION),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_target_is_excluded_from_written_parameters() {
        assert!(matches!(
            MATCH.target_parameter().map(|parameter| parameter.constraint),
            Some(XenoConstraint::Type(required)) if std::ptr::eq(required, &STRING)
        ));
        assert_eq!(MATCH.explicit_parameters().len(), 1);
        assert_eq!(
            MATCH.parameter_at(0).map(|parameter| parameter.name),
            Some("pattern")
        );
        assert!(MATCH.parameter_at(1).is_none());
    }

    #[test]
    fn variadic_lookup_repeats_only_the_last_written_parameter() {
        static VARIADIC: XenoAnnotation = XenoAnnotation {
            name: "variadic",
            documentation: None,
            kind: XenoAnnotationKind::Meta,
            params: &[&ANY_TARGET_PARAM, &STRING_VALUE_PARAM],
            variadic: true,
        };

        assert_eq!(
            VARIADIC.parameter_at(3).map(|parameter| parameter.name),
            Some("value")
        );
    }
}
