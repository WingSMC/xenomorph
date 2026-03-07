pub enum XenoAnnotationKind {
    Transformation,
    Validation,
    ComplexValidation,
    Meta,
}

pub struct XenoAnnotation {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub kind: XenoAnnotationKind,
    pub params: Option<&'static [&'static str]>,
}

pub static VALUE_PARAM: [&str; 1] = ["value"];
pub static CONDITION_PARAM: [&str; 2] = ["condition", "value"];

pub static MIN: XenoAnnotation = XenoAnnotation {
    name: "min",
    documentation: Some("Specifies the minimum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&VALUE_PARAM),
};

pub static MAX: XenoAnnotation = XenoAnnotation {
    name: "max",
    documentation: Some("Specifies the maximum value for a numeric type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&VALUE_PARAM),
};

pub static LEN: XenoAnnotation = XenoAnnotation {
    name: "len",
    documentation: Some("Specifies the exact length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&VALUE_PARAM),
};

pub static MINLEN: XenoAnnotation = XenoAnnotation {
    name: "minlen",
    documentation: Some("Specifies the minimum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&VALUE_PARAM),
};

pub static MAXLEN: XenoAnnotation = XenoAnnotation {
    name: "maxlen",
    documentation: Some("Specifies the maximum length for a string or list type."),
    kind: XenoAnnotationKind::Validation,
    params: Some(&VALUE_PARAM),
};

pub static IF: XenoAnnotation = XenoAnnotation {
    name: "if",
    documentation: Some("Applies or removes **validation** depending on the condition."),
    kind: XenoAnnotationKind::ComplexValidation,
    params: Some(&CONDITION_PARAM),
};

pub static ELSEIF : XenoAnnotation = XenoAnnotation {
	name: "elseif",
	documentation: Some("Applies or removes validation depending on the condition, used after an `@if` or another `@elseif`."),
	kind: XenoAnnotationKind::ComplexValidation,
	params: Some(&CONDITION_PARAM),
};

pub static ELSE: XenoAnnotation = XenoAnnotation {
    name: "else",
    documentation: Some(
        "Applies validation if previous `@if` and `@elseif` conditions were not met.",
    ),
    kind: XenoAnnotationKind::ComplexValidation,
    params: Some(&VALUE_PARAM),
};

pub static BUILTIN_ANNOTATIONS: [&'static XenoAnnotation; 8] =
    [&MIN, &MAX, &LEN, &MINLEN, &MAXLEN, &IF, &ELSEIF, &ELSE];
