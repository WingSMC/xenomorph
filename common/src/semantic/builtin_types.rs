use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
pub enum XenoTraitKind {
    /// A trait implemented by semantic types and inherited through parents.
    Semantic,
    /// A structural trait implemented by source struct declarations.
    Struct,
    /// A structural trait implemented by sum types whose members satisfy the
    /// embedded trait.
    Sum(&'static XenoTrait),
    /// A trait implemented by every annotation argument expression.
    Expression,
    /// A trait implemented by literal annotation arguments.
    Literal,
    /// A semantic type constraint that only literal arguments may satisfy.
    LiteralType,
    /// A trait implemented only by regular-expression literal arguments.
    RegexLiteral,
    /// A trait implemented by bare identifier arguments.
    Identifier,
    /// A trait implemented by references to known types.
    Type,
    /// A trait implemented by nested annotation arguments.
    Annotation,
}

#[derive(Debug)]
pub struct XenoTrait {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub kind: XenoTraitKind,
    pub parents: Option<&'static [&'static XenoTrait]>,
}

impl XenoTrait {
    /// Returns whether this trait is, or transitively inherits from, `target`.
    pub fn is_or_inherits(&self, target: &XenoTrait) -> bool {
        self.is_or_inherits_inner(target, &mut HashSet::new())
    }

    fn is_or_inherits_inner(
        &self,
        target: &XenoTrait,
        visited: &mut HashSet<*const XenoTrait>,
    ) -> bool {
        if std::ptr::eq(self, target) || self.name == target.name {
            return true;
        }
        if !visited.insert(self as *const XenoTrait) {
            return false;
        }

        self.parents
            .unwrap_or(&[])
            .iter()
            .any(|parent| parent.is_or_inherits_inner(target, visited))
    }
}

pub static NUMERIC: XenoTrait = XenoTrait {
    name: "Numeric",
    documentation: Some("Implemented by numeric value types."),
    kind: XenoTraitKind::Semantic,
    parents: None,
};

pub static HAS_LENGTH: XenoTrait = XenoTrait {
    name: "HasLength",
    documentation: Some("Implemented by values whose elements or characters can be counted."),
    kind: XenoTraitKind::Semantic,
    parents: None,
};

pub static KEY_TRAIT: XenoTrait = XenoTrait {
    name: "KeyTrait",
    documentation: Some("Implemented by types that can be used as dictionary keys."),
    kind: XenoTraitKind::Semantic,
    parents: None,
};

pub static STRUCT: XenoTrait = XenoTrait {
    name: "Struct",
    documentation: Some("Implemented by source struct declarations and their aliases."),
    kind: XenoTraitKind::Struct,
    parents: None,
};

pub static EXPRESSION: XenoTrait = XenoTrait {
    name: "Expression",
    documentation: Some("Implemented by every annotation argument expression."),
    kind: XenoTraitKind::Expression,
    parents: None,
};

pub static LITERAL: XenoTrait = XenoTrait {
    name: "Literal",
    documentation: Some("Implemented by literal annotation arguments."),
    kind: XenoTraitKind::Literal,
    parents: Some(&[&EXPRESSION]),
};

pub static NUMBER_LITERAL: XenoTrait = XenoTrait {
    name: "NumberLiteral",
    documentation: Some("Implemented by integer and floating-point literals."),
    kind: XenoTraitKind::LiteralType,
    parents: Some(&[&LITERAL]),
};

pub static INTEGER_LITERAL: XenoTrait = XenoTrait {
    name: "IntegerLiteral",
    documentation: Some("Implemented by integer literals."),
    kind: XenoTraitKind::LiteralType,
    parents: Some(&[&NUMBER_LITERAL]),
};

pub static STRING_LITERAL: XenoTrait = XenoTrait {
    name: "StringLiteral",
    documentation: Some("Implemented by string literals."),
    kind: XenoTraitKind::LiteralType,
    parents: Some(&[&LITERAL]),
};

pub static REGEX_LITERAL: XenoTrait = XenoTrait {
    name: "RegexLiteral",
    documentation: Some("Implemented by regular-expression literals."),
    kind: XenoTraitKind::RegexLiteral,
    parents: Some(&[&LITERAL]),
};

pub static BOOL_LITERAL: XenoTrait = XenoTrait {
    name: "BoolLiteral",
    documentation: Some("Implemented by boolean literals."),
    kind: XenoTraitKind::LiteralType,
    parents: Some(&[&LITERAL]),
};

pub static STRING_LITERAL_SUM: XenoTrait = XenoTrait {
    name: "Sum<StringLiteral>",
    documentation: Some("Implemented by sum types whose members are all string literals."),
    kind: XenoTraitKind::Sum(&STRING_LITERAL),
    parents: None,
};

pub static LITERAL_SUM: XenoTrait = XenoTrait {
    name: "Sum<Literal>",
    documentation: Some("Implemented by sum types whose members are all literals."),
    kind: XenoTraitKind::Sum(&LITERAL),
    parents: None,
};

pub static IDENTIFIER: XenoTrait = XenoTrait {
    name: "Identifier",
    documentation: Some("Implemented by bare identifier arguments."),
    kind: XenoTraitKind::Identifier,
    parents: Some(&[&EXPRESSION]),
};

pub static TYPE_REFERENCE: XenoTrait = XenoTrait {
    name: "Type",
    documentation: Some("Implemented by references to known types."),
    kind: XenoTraitKind::Type,
    parents: Some(&[&IDENTIFIER]),
};

pub static ANNOTATION: XenoTrait = XenoTrait {
    name: "Annotation",
    documentation: Some("Implemented by nested annotation arguments."),
    kind: XenoTraitKind::Annotation,
    parents: Some(&[&EXPRESSION]),
};

#[derive(Debug)]
pub struct GenericParam {
    pub name: &'static str,
    pub constraint: Option<XenoConstraint>,
}

#[derive(Debug, Clone, Copy)]
pub enum XenoConstraint {
    Type(&'static XenoType),
    Trait(&'static XenoTrait),
}

impl XenoConstraint {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Type(required) => required.name,
            Self::Trait(required) => required.name,
        }
    }
}

#[derive(Debug)]
pub struct XenoType {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub generic_params: Option<&'static [&'static GenericParam]>,
    pub parents: Option<&'static [XenoParent]>,
}

#[derive(Debug, Clone, Copy)]
pub enum XenoParent {
    Type(&'static XenoType),
    Trait(&'static XenoTrait),
}

impl XenoType {
    pub fn implements(&self, xeno_trait: &XenoTrait) -> bool {
        if self.name == NEVER.name {
            return true;
        }
        self.implements_inner(xeno_trait, &mut HashSet::new())
    }

    fn implements_inner(
        &self,
        xeno_trait: &XenoTrait,
        visited: &mut HashSet<*const XenoType>,
    ) -> bool {
        if !visited.insert(self as *const XenoType) {
            return false;
        }

        self.parents
            .unwrap_or(&[])
            .iter()
            .any(|parent| match parent {
                XenoParent::Trait(parent_trait) => parent_trait.is_or_inherits(xeno_trait),
                XenoParent::Type(parent_type) => parent_type.implements_inner(xeno_trait, visited),
            })
    }
}

pub fn is_type_compatible(
    candidate: &XenoType,
    target: &XenoType,
    visited: &mut HashSet<*const XenoType>,
) -> bool {
    if candidate.name == NEVER.name {
        return true;
    }

    if target.name == ANY.name {
        return true;
    }

    if candidate.name == ANY.name {
        return false;
    }

    if std::ptr::eq(candidate, target) || candidate.name == target.name {
        return true;
    }

    let candidate_ptr = candidate as *const XenoType;
    if !visited.insert(candidate_ptr) {
        return false;
    }

    candidate.parents.unwrap_or(&[]).iter().any(|parent| {
        matches!(parent, XenoParent::Type(parent_type) if is_type_compatible(parent_type, target, visited))
    })
}

static ANY_PARENT: &[XenoParent] = &[XenoParent::Type(&ANY)];
static BOOL_PARENT: &[XenoParent] = &[XenoParent::Type(&ANY), XenoParent::Trait(&BOOL_LITERAL)];
static NUMBER_PARENT: &[XenoParent] = &[
    XenoParent::Type(&ANY),
    XenoParent::Trait(&NUMERIC),
    XenoParent::Trait(&NUMBER_LITERAL),
    XenoParent::Trait(&KEY_TRAIT),
];
static INTEGER_PARENT: &[XenoParent] = &[
    XenoParent::Type(&NUMBER),
    XenoParent::Trait(&INTEGER_LITERAL),
];
static STR_PARENT: &[XenoParent] = &[XenoParent::Type(&STRING)];
static STRING_PARENT: &[XenoParent] = &[
    XenoParent::Type(&ANY),
    XenoParent::Trait(&HAS_LENGTH),
    XenoParent::Trait(&STRING_LITERAL),
    XenoParent::Trait(&KEY_TRAIT),
];
static IP_PARENT: &[XenoParent] = &[XenoParent::Type(&IP)];
static LENGTH_PARENT: &[XenoParent] = &[XenoParent::Type(&ANY), XenoParent::Trait(&HAS_LENGTH)];

static TYPE: XenoType = XenoType {
    name: "TYPE",
    documentation: Some("The TYPE type represents a type in the XenoType system."),
    generic_params: None,
    parents: None,
};

/// The bottom type. Its universal parentage is resolved by `TypeHierarchy`
/// instead of being materialized as a leaked static parent slice.
pub static NEVER: XenoType = XenoType {
    name: "NEVER",
    documentation: Some("The NEVER type has no values and is a subtype of every other type."),
    generic_params: None,
    parents: None,
};

pub static ANY: XenoType = XenoType {
    name: "any",
    documentation: Some(
        "The any type represents a value of any type. It is used for dynamic typing and can hold values of any type, including primitive types, complex types, and even other any types.",
    ),
    generic_params: None,
    parents: None,
};

static NULL: XenoType = XenoType {
    name: "null",
    documentation: Some("The null type represents the absence of a value."),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static BOOL: XenoType = XenoType {
    name: "bool",
    documentation: Some(
        "The boolean type represents a value that can be either true (1) or false (0).",
    ),
    generic_params: None,
    parents: Some(BOOL_PARENT),
};

static NUMBER: XenoType = XenoType {
    name: "number",
    documentation: Some("The number type represents a numeric value."),
    generic_params: None,
    parents: Some(NUMBER_PARENT),
};

static INTEGER: XenoType = XenoType {
    name: "integer",
    documentation: Some(
        "The integer type represents a whole number. Generalizes i128, u128 and bigint.",
    ),
    generic_params: None,
    parents: Some(INTEGER_PARENT),
};

static I4: XenoType = XenoType {
    name: "i4",
    documentation: Some("The i4 type represents a 4-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&I8)]),
};

static I8: XenoType = XenoType {
    name: "i8",
    documentation: Some("The i8 type represents an 8-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&I16)]),
};

static I16: XenoType = XenoType {
    name: "i16",
    documentation: Some("The i16 type represents a 16-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&I32)]),
};

static I32: XenoType = XenoType {
    name: "i32",
    documentation: Some("The i32 type represents a 32-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&I64)]),
};

static I64: XenoType = XenoType {
    name: "i64",
    documentation: Some("The i64 type represents a 64-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&I128)]),
};

static I128: XenoType = XenoType {
    name: "i128",
    documentation: Some("The i128 type represents a 128-bit integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&INTEGER)]),
};

static U4: XenoType = XenoType {
    name: "u4",
    documentation: Some("The u4 type represents a 4-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&U8), XenoParent::Type(&I8)]),
};

static U8: XenoType = XenoType {
    name: "u8",
    documentation: Some("The u8 type represents an 8-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&U16), XenoParent::Type(&I16)]),
};

static U16: XenoType = XenoType {
    name: "u16",
    documentation: Some("The u16 type represents a 16-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&U32), XenoParent::Type(&I32)]),
};

static U32: XenoType = XenoType {
    name: "u32",
    documentation: Some("The u32 type represents a 32-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&U64), XenoParent::Type(&I64)]),
};

static U64: XenoType = XenoType {
    name: "u64",
    documentation: Some("The u64 type represents a 64-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&U128), XenoParent::Type(&I128)]),
};

static U128: XenoType = XenoType {
    name: "u128",
    documentation: Some("The u128 type represents a 128-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&INTEGER)]),
};

static F32: XenoType = XenoType {
    name: "f32",
    documentation: Some("The f32 type represents a 32-bit floating point number."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&F64)]),
};

static F64: XenoType = XenoType {
    name: "f64",
    documentation: Some("The f64 type represents a 64-bit floating point number."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&NUMBER)]),
};

static BIGINT: XenoType = XenoType {
    name: "bigint",
    documentation: Some("The bigint type represents an arbitrary size integer."),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&INTEGER)]),
};

static DECIMAL: XenoType = XenoType {
    name: "decimal",
    documentation: Some(
        "The decimal type represents a fixed-point decimal number with arbitrary precision.",
    ),
    generic_params: None,
    parents: Some(&[XenoParent::Type(&NUMBER)]),
};

static DATE: XenoType = XenoType {
    name: "date",
    documentation: Some("The date type represents a calendar date without a time component."),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static DATETIME: XenoType = XenoType {
    name: "datetime",
    documentation: Some(
        "The datetime type represents a specific point in time, including both date and time components.",
    ),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static DURATION: XenoType = XenoType {
    name: "duration",
    documentation: Some(
        "The duration type represents a length of time, typically used for measuring intervals or differences between datetime values.",
    ),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

pub static STRING: XenoType = XenoType {
    name: "string",
    documentation: Some("The string type represents a sequence of characters."),
    generic_params: None,
    parents: Some(STRING_PARENT),
};

static CHAR: XenoType = XenoType {
    name: "char",
    documentation: Some(
        "The char type represents a single character, typically used for representing individual letters, digits, or symbols. This includes Unicode code points. For classic ASCII chars use u8, u16, or u32.",
    ),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static UUID: XenoType = XenoType {
    name: "uuid",
    documentation: Some(
        "The uuid type represents a universally unique identifier (128 bit number) in string format, represented as a 36-character string consisting of hexadecimal digits and hyphens (e.g., 123e456-e89b-12d3-a456-426614174000).",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static REGEX: XenoType = XenoType {
    name: "regex",
    documentation: Some(
        "The regex type represents a regular expression, which is a sequence of characters that defines a search pattern for matching strings.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static IP: XenoType = XenoType {
    name: "ip",
    documentation: Some("The ip type represents either an ipv4 or an ipv6 address."),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static IPV4: XenoType = XenoType {
    name: "ipv4",
    documentation: Some(
        "The ipv4 type represents an IPv4 address in a dot-decimal notation (e.g., 192.168.0.1).",
    ),
    generic_params: None,
    parents: Some(IP_PARENT),
};

static IPV6: XenoType = XenoType {
    name: "ipv6",
    documentation: Some(
        "The ipv6 type represents an IPv6 address in a colon-hexadecimal notation (e.g., 2001:0db8:85a3:0000:0000:8a2e:0370:7334).",
    ),
    generic_params: None,
    parents: Some(IP_PARENT),
};

static HOSTNAME: XenoType = XenoType {
    name: "hostname",
    documentation: Some(
        "The hostname type represents a domain name or an IP address that identifies a host on a network.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static EMAIL: XenoType = XenoType {
    name: "email",
    documentation: Some("The email type represents an email address"),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static URL: XenoType = XenoType {
    name: "url",
    documentation: Some(
        "The url type represents a Uniform Resource Locator, which is a reference to a resource on the internet.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static BINARY: XenoType = XenoType {
    name: "binary",
    documentation: Some(
        "The binary type represents a sequence of bytes, typically used for storing and transmitting raw data.",
    ),
    generic_params: None,
    parents: Some(LENGTH_PARENT),
};

static JSON: XenoType = XenoType {
    name: "json",
    documentation: Some(
        "The json type represents a JSON (JavaScript Object Notation) value, which is a lightweight data-interchange format that is easy for humans to read and write and easy for machines to parse and generate.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static XML: XenoType = XenoType {
    name: "xml",
    documentation: Some(
        "The xml type represents an XML (eXtensible Markup Language) document, which is a markup language that defines a set of rules for encoding documents in a format that is both human-readable and machine-readable.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static YAML: XenoType = XenoType {
    name: "yaml",
    documentation: Some(
        "The yaml type represents a YAML (YAML Ain't Markup Language) document, which is a human-readable data serialization format that is commonly used for configuration files and data exchange between languages with different data structures.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static TOML: XenoType = XenoType {
    name: "toml",
    documentation: Some(
        "The toml type represents a TOML (Tom's Obvious, Minimal Language) document, which is a minimal configuration file format that is easy to read and write due to its simple syntax.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static CSV: XenoType = XenoType {
    name: "csv",
    documentation: Some(
        "The csv type represents a CSV (Comma-Separated Values) file, which is a simple file format used to store tabular data, where each line of the file represents a data record and each record consists of fields separated by commas.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static TSV: XenoType = XenoType {
    name: "tsv",
    documentation: Some(
        "The tsv type represents a TSV (Tab-Separated Values) file, which is a simple file format used to store tabular data, where each line of the file represents a data record and each record consists of fields separated by tabs.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static SEMVER: XenoType = XenoType {
    name: "semver",
    documentation: Some(
        "The semver type represents a semantic version, which is a versioning scheme that uses a three-part version number (major.minor.patch) to indicate the level of changes in a software release.",
    ),
    generic_params: None,
    parents: Some(STR_PARENT),
};

static DICT: XenoType = XenoType {
    name: "Dict",
    documentation: Some(
        "The Dict type represents a collection of key-value pairs, where each key is unique and maps to a corresponding value.",
    ),
    generic_params: Some(&[
        &GenericParam {
            name: "K",
            constraint: Some(XenoConstraint::Trait(&KEY_TRAIT)),
        },
        &GenericParam {
            name: "V",
            constraint: None,
        },
    ]),
    parents: Some(LENGTH_PARENT),
};

static ARRAY_ELEMENT: GenericParam = GenericParam {
    name: "T",
    constraint: None,
};

static ARRAY: XenoType = XenoType {
    name: "array",
    documentation: Some(
        "The array type represents an ordered, variable-length collection. Postfix `T[]` is its shorthand.",
    ),
    generic_params: Some(&[&ARRAY_ELEMENT]),
    parents: Some(LENGTH_PARENT),
};

static STRUCT_PARAMETER: GenericParam = GenericParam {
    name: "T",
    constraint: Some(XenoConstraint::Trait(&STRUCT)),
};

static STRING_LITERAL_KEYS_PARAMETER: GenericParam = GenericParam {
    name: "K",
    constraint: Some(XenoConstraint::Trait(&STRING_LITERAL_SUM)),
};

static UNCONSTRAINED_PARAMETER: GenericParam = GenericParam {
    name: "T",
    constraint: None,
};

static PICK: XenoType = XenoType {
    name: "Pick",
    documentation: Some(
        "Creates a struct containing only the selected fields from another struct.",
    ),
    generic_params: Some(&[&STRUCT_PARAMETER, &STRING_LITERAL_KEYS_PARAMETER]),
    parents: Some(ANY_PARENT),
};

static OMIT: XenoType = XenoType {
    name: "Omit",
    documentation: Some("Creates a struct without the selected fields from another struct."),
    generic_params: Some(&[&STRUCT_PARAMETER, &STRING_LITERAL_KEYS_PARAMETER]),
    parents: Some(ANY_PARENT),
};

static KEYOF: XenoType = XenoType {
    name: "Keyof",
    documentation: Some("Creates a sum type containing a struct's field names as string literals."),
    generic_params: Some(&[&STRUCT_PARAMETER]),
    parents: Some(ANY_PARENT),
};

static REQUIRED: XenoType = XenoType {
    name: "Required",
    documentation: Some(
        "Removes outer optionality from a type. Applying it to a required type is idempotent.",
    ),
    generic_params: Some(&[&UNCONSTRAINED_PARAMETER]),
    parents: Some(ANY_PARENT),
};

static PARTIAL: XenoType = XenoType {
    name: "Partial",
    documentation: Some("Creates a struct in which every field is optional."),
    generic_params: Some(&[&STRUCT_PARAMETER]),
    parents: Some(ANY_PARENT),
};

static COMPLETE: XenoType = XenoType {
    name: "Complete",
    documentation: Some("Creates a struct in which every field is required."),
    generic_params: Some(&[&STRUCT_PARAMETER]),
    parents: Some(ANY_PARENT),
};

/// Type utilities are operations written with generic syntax rather than
/// declared types. A target either spells one natively, like TypeScript's
/// `Pick`, or has to evaluate it into a concrete type.
#[rustfmt::skip]
pub static BUILTIN_TYPE_UTILITIES: &[&XenoType] = &[
    &PICK,
    &OMIT,
    &KEYOF,
    &REQUIRED,
    &PARTIAL,
    &COMPLETE,
];

pub fn is_type_utility(name: &str) -> bool {
    BUILTIN_TYPE_UTILITIES
        .iter()
        .any(|utility| utility.name == name)
}

#[rustfmt::skip]
pub static BUILTIN_TYPES: &[&XenoType] = &[
    &TYPE,
    &NEVER,
    &ANY,
    &NULL,
    &BOOL,
    &NUMBER,
    &INTEGER,
    &I4,
    &I8,
    &I16,
    &I32,
    &I64,
    &I128,
    &U4,
    &U8,
    &U16,
    &U32,
    &U64,
    &U128,
    &F32,
    &F64,
    &BIGINT,
    &DECIMAL,
    &DATE,
    &DATETIME,
    &DURATION,
    &STRING,
    &CHAR,
    &UUID,
    &REGEX,
    &IP,
    &IPV4,
    &IPV6,
    &HOSTNAME,
    &EMAIL,
    &URL,
    &BINARY,
    &JSON,
    &XML,
    &YAML,
    &TOML,
    &CSV,
    &TSV,
    &SEMVER,
    &DICT,
    &ARRAY,
    &PICK,
    &OMIT,
    &KEYOF,
    &REQUIRED,
    &PARTIAL,
    &COMPLETE,
];

pub static BUILTIN_TRAITS: &[&XenoTrait] = &[
    &NUMERIC,
    &HAS_LENGTH,
    &KEY_TRAIT,
    &STRUCT,
    &STRING_LITERAL_SUM,
    &LITERAL_SUM,
    &EXPRESSION,
    &LITERAL,
    &NUMBER_LITERAL,
    &INTEGER_LITERAL,
    &STRING_LITERAL,
    &REGEX_LITERAL,
    &BOOL_LITERAL,
    &IDENTIFIER,
    &TYPE_REFERENCE,
    &ANNOTATION,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_traits_follow_multiple_parent_levels() {
        assert!(STRING.implements(&HAS_LENGTH));
        assert!(STRING.implements(&KEY_TRAIT));
        assert!(ARRAY.implements(&HAS_LENGTH));
        assert!(BINARY.implements(&HAS_LENGTH));
        assert!(DICT.implements(&HAS_LENGTH));
        assert!(UUID.implements(&HAS_LENGTH));
        assert!(U8.implements(&NUMERIC));
        assert!(NUMBER.implements(&KEY_TRAIT));
        assert!(U8.implements(&KEY_TRAIT));
        assert!(U8.implements(&NUMBER_LITERAL));
        assert!(U8.implements(&INTEGER_LITERAL));
        assert!(U8.implements(&LITERAL));
        assert!(U8.implements(&EXPRESSION));
        assert!(F32.implements(&NUMBER_LITERAL));
        assert!(BOOL.implements(&BOOL_LITERAL));
        assert!(STRING.implements(&STRING_LITERAL));
        assert!(!BOOL.implements(&HAS_LENGTH));
        assert!(!BOOL.implements(&KEY_TRAIT));
        assert!(!F32.implements(&INTEGER_LITERAL));
    }

    #[test]
    fn traits_follow_transitive_parentage() {
        assert!(STRING_LITERAL.is_or_inherits(&LITERAL));
        assert!(STRING_LITERAL.is_or_inherits(&EXPRESSION));
        assert!(REGEX_LITERAL.is_or_inherits(&LITERAL));
        assert!(REGEX_LITERAL.is_or_inherits(&EXPRESSION));
        assert!(INTEGER_LITERAL.is_or_inherits(&NUMBER_LITERAL));
        assert!(INTEGER_LITERAL.is_or_inherits(&LITERAL));
        assert!(TYPE_REFERENCE.is_or_inherits(&IDENTIFIER));
        assert!(TYPE_REFERENCE.is_or_inherits(&EXPRESSION));
        assert!(!STRING_LITERAL.is_or_inherits(&NUMBER_LITERAL));
        assert!(!LITERAL.is_or_inherits(&STRING_LITERAL));
    }

    #[test]
    fn unsigned_integers_widen_to_lossless_signed_types() {
        for (unsigned, signed) in [
            (&U4, &I8),
            (&U8, &I16),
            (&U16, &I32),
            (&U32, &I64),
            (&U64, &I128),
        ] {
            assert!(is_type_compatible(unsigned, signed, &mut HashSet::new()));
        }

        assert!(!is_type_compatible(&U8, &I8, &mut HashSet::new()));
        assert!(!is_type_compatible(&U128, &I128, &mut HashSet::new()));
        assert!(!is_type_compatible(&I8, &U8, &mut HashSet::new()));
    }

    #[test]
    fn never_is_the_bottom_static_type() {
        for target in BUILTIN_TYPES {
            assert!(is_type_compatible(&NEVER, target, &mut HashSet::new()));
        }
        for xeno_trait in BUILTIN_TRAITS {
            assert!(NEVER.implements(xeno_trait));
        }
        assert!(!is_type_compatible(&STRING, &NEVER, &mut HashSet::new()));
    }
}
