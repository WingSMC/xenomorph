use crate::semantic::{XenoType, LENGTH_TYPES, NUMBER_TYPES};

pub enum XenoAnnotationKind {
    Transformation,
    Validation,
    ComplexValidation,
    Meta,
}

#[derive(Clone, Copy, PartialEq)]
pub enum XenoParameterType {
    None,
    NumberLiteral,
    IntegerLiteral,
    StringLiteral,
    BoolLiteral,
    FieldReference,
    AnyLiteral,
    Expression,
    Identifier,
    Type,
    Annotation,
    List(&'static [XenoParameterType]),
}

pub struct XenoParam {
    pub name: &'static str,
    pub param_type: XenoParameterType,
}

pub struct XenoAnnotation {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub kind: XenoAnnotationKind,
    pub params: Option<&'static [&'static XenoParam]>,
    pub applicable_to: Option<&'static [&'static XenoType]>,
}

pub static NUMBER_VALUE_PARAM: &[&XenoParam] = &[&XenoParam {
    name: "value",
    param_type: XenoParameterType::NumberLiteral,
}];
pub static INTEGER_VALUE_PARAM: &[&XenoParam] = &[&XenoParam {
    name: "value",
    param_type: XenoParameterType::IntegerLiteral,
}];
pub static EXPRESSION_VALUE_PARAM: &[&XenoParam] = &[&XenoParam {
    name: "value",
    param_type: XenoParameterType::Expression,
}];
pub static CONDITION_PARAM: &[&XenoParam] = &[
    &XenoParam {
        name: "condition",
        param_type: XenoParameterType::Expression,
    },
    &XenoParam {
        name: "value",
        param_type: XenoParameterType::Expression,
    },
];

pub static MIN: XenoAnnotation = XenoAnnotation {
    name: "min",
    documentation: Some("Specifies the minimum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&NUMBER_VALUE_PARAM),
    applicable_to: Some(NUMBER_TYPES),
};

pub static MAX: XenoAnnotation = XenoAnnotation {
    name: "max",
    documentation: Some("Specifies the maximum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&NUMBER_VALUE_PARAM),
    applicable_to: Some(NUMBER_TYPES),
};

pub static GT: XenoAnnotation = XenoAnnotation {
    name: "gt",
    documentation: Some("Specifies that some numeric value must be greater than the parameter."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&NUMBER_VALUE_PARAM),
    applicable_to: Some(NUMBER_TYPES),
};

pub static LT: XenoAnnotation = XenoAnnotation {
    name: "lt",
    documentation: Some("Specifies that some numeric value must be less than the parameter."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&NUMBER_VALUE_PARAM),
    applicable_to: Some(&NUMBER_TYPES),
};

pub static LEN: XenoAnnotation = XenoAnnotation {
    name: "len",
    documentation: Some("Specifies the exact length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&INTEGER_VALUE_PARAM),
    applicable_to: Some(LENGTH_TYPES),
};

pub static MINLEN: XenoAnnotation = XenoAnnotation {
    name: "minlen",
    documentation: Some("Specifies the minimum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&INTEGER_VALUE_PARAM),
    applicable_to: Some(LENGTH_TYPES),
};

pub static MAXLEN: XenoAnnotation = XenoAnnotation {
    name: "maxlen",
    documentation: Some("Specifies the maximum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&INTEGER_VALUE_PARAM),
    applicable_to: Some(LENGTH_TYPES),
};

pub static IF: XenoAnnotation = XenoAnnotation {
    name: "if",
    documentation: Some("Applies or removes **validation** depending on the condition."),
    kind: XenoAnnotationKind::ComplexValidation,
    params: Some(&CONDITION_PARAM),
    applicable_to: None,
};

pub static ELSEIF : XenoAnnotation = XenoAnnotation {
	name: "elseif",
	documentation: Some("Applies or removes validation depending on the condition, used after an `@if` or another `@elseif`."),
	kind: XenoAnnotationKind::ComplexValidation,
	params: Some(&CONDITION_PARAM),
    applicable_to: None,
};

pub static ELSE: XenoAnnotation = XenoAnnotation {
    name: "else",
    documentation: Some(
        "Applies validation if previous `@if` and `@elseif` conditions were not met.",
    ),
    kind: XenoAnnotationKind::ComplexValidation,
    params: Some(&EXPRESSION_VALUE_PARAM),
    applicable_to: None,
};

pub static BUILTIN_ANNOTATIONS: &[&'static XenoAnnotation] = &[
    &MIN, &MAX, &GT, &LT, &LEN, &MINLEN, &MAXLEN, &IF, &ELSEIF, &ELSE,
];
