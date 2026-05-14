use std::collections::HashSet;

pub struct GenericParam {
    pub name: &'static str,
    pub parent: Option<&'static [&'static XenoType]>,
}

pub struct XenoType {
    pub name: &'static str,
    pub documentation: Option<&'static str>,
    pub generic_params: Option<&'static [&'static GenericParam]>,
    pub parents: Option<&'static [&'static XenoType]>,
}

pub fn is_type_compatible(
    candidate: &XenoType,
    target: &XenoType,
    visited: &mut HashSet<*const XenoType>,
) -> bool {
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

    if let Some(parents) = candidate.parents {
        if parents
            .iter()
            .any(|parent| is_type_compatible(parent, target, visited))
        {
            return true;
        }
    }

    false
}

static ANY_PARENT: &[&XenoType] = &[&ANY];
static NUM_PARENT: &[&XenoType] = &[&NUMBER];
static INT_PARENT: &[&XenoType] = &[&INTEGER];
static STR_PARENT: &[&XenoType] = &[&STRING];
static IP_PARENT: &[&XenoType] = &[&IP];

static TYPE: XenoType = XenoType {
    name: "TYPE",
    documentation: Some("The TYPE type represents a type in the XenoType system."),
    generic_params: None,
    parents: None,
};

static ANY: XenoType = XenoType {
    name: "any",
    documentation: Some(
        "The any type represents a value of any type. It is used for dynamic typing and can hold values of any type, including primitive types, complex types, and even other any types.",
    ),
    generic_params: None,
    parents: None,
};

static BOOL: XenoType = XenoType {
    name: "bool",
    documentation: Some(
        "The boolean type represents a value that can be either true (1) or false (0).",
    ),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static NUMBER: XenoType = XenoType {
    name: "number",
    documentation: Some("The number type represents a numeric value."),
    generic_params: None,
    parents: Some(ANY_PARENT),
};

static INTEGER: XenoType = XenoType {
    name: "integer",
    documentation: Some(
        "The integer type represents a whole number. Generalizes i128, u128 and bigint.",
    ),
    generic_params: None,
    parents: Some(NUM_PARENT),
};

static I4: XenoType = XenoType {
    name: "i4",
    documentation: Some("The i4 type represents a 4-bit integer."),
    generic_params: None,
    parents: Some(&[&I8]),
};

static I8: XenoType = XenoType {
    name: "i8",
    documentation: Some("The i8 type represents an 8-bit integer."),
    generic_params: None,
    parents: Some(&[&I16]),
};

static I16: XenoType = XenoType {
    name: "i16",
    documentation: Some("The i16 type represents a 16-bit integer."),
    generic_params: None,
    parents: Some(&[&I32]),
};

static I32: XenoType = XenoType {
    name: "i32",
    documentation: Some("The i32 type represents a 32-bit integer."),
    generic_params: None,
    parents: Some(&[&I64]),
};

static I64: XenoType = XenoType {
    name: "i64",
    documentation: Some("The i64 type represents a 64-bit integer."),
    generic_params: None,
    parents: Some(&[&I128]),
};

static I128: XenoType = XenoType {
    name: "i128",
    documentation: Some("The i128 type represents a 128-bit integer."),
    generic_params: None,
    parents: Some(INT_PARENT),
};

static U4: XenoType = XenoType {
    name: "u4",
    documentation: Some("The u4 type represents a 4-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[&U8]),
};

static U8: XenoType = XenoType {
    name: "u8",
    documentation: Some("The u8 type represents an 8-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[&U16]),
};

static U16: XenoType = XenoType {
    name: "u16",
    documentation: Some("The u16 type represents a 16-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[&U32]),
};

static U32: XenoType = XenoType {
    name: "u32",
    documentation: Some("The u32 type represents a 32-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[&U64]),
};

static U64: XenoType = XenoType {
    name: "u64",
    documentation: Some("The u64 type represents a 64-bit unsigned integer."),
    generic_params: None,
    parents: Some(&[&U128]),
};

static U128: XenoType = XenoType {
    name: "u128",
    documentation: Some("The u128 type represents a 128-bit unsigned integer."),
    generic_params: None,
    parents: Some(INT_PARENT),
};

static F32: XenoType = XenoType {
    name: "f32",
    documentation: Some("The f32 type represents a 32-bit floating point number."),
    generic_params: None,
    parents: Some(&[&F64]),
};

static F64: XenoType = XenoType {
    name: "f64",
    documentation: Some("The f64 type represents a 64-bit floating point number."),
    generic_params: None,
    parents: Some(NUM_PARENT),
};

static BIGINT: XenoType = XenoType {
    name: "bigint",
    documentation: Some("The bigint type represents an arbitrary size integer."),
    generic_params: None,
    parents: Some(INT_PARENT),
};

static DECIMAL: XenoType = XenoType {
    name: "decimal",
    documentation: Some(
        "The decimal type represents a fixed-point decimal number with arbitrary precision.",
    ),
    generic_params: None,
    parents: Some(NUM_PARENT),
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

static STRING: XenoType = XenoType {
    name: "string",
    documentation: Some("The string type represents a sequence of characters."),
    generic_params: None,
    parents: Some(ANY_PARENT),
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
    parents: Some(ANY_PARENT),
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
    name: "dict",
    documentation: Some(
        "The dict type represents a collection of key-value pairs, where each key is unique and maps to a corresponding value.",
    ),
    generic_params: Some(&[
        &GenericParam { name: "K", parent: None },
        &GenericParam { name: "V", parent: None },
    ]),
    parents: Some(ANY_PARENT),
};

#[rustfmt::skip]
pub static BUILTIN_TYPES: &[&XenoType] = &[
    &TYPE,
    &ANY,
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
];

pub static NUMBER_TYPES: &[&XenoType] = &[&NUMBER];
pub static LENGTH_TYPES: &[&XenoType] = &[&STRING, &BINARY, &DICT];
