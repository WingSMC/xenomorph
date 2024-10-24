use crate::tokens::{NumberType, Token};

enum ParserNode {
    Type,
    Annotation,

}

pub(crate) fn parse(tokens: &Vec<Token>) {
    println!("{:?}", tokens);

    let mut it = tokens.iter().peekable();
}


use std::iter::Peekable;
use std::vec::IntoIter;

#[derive(Debug, Clone)]
pub enum TypeExpr<'src> {
    // Basic types
    Bool,
    Number(NumberType),
    String,
    
    // Complex types
    List(Box<TypeExpr<'src>>),  // For homogeneous lists like string[]
    Tuple(Vec<TypeExpr<'src>>), // For heterogeneous lists [string, number]
    Set(Box<TypeExpr<'src>>),   // For set types
    
    // Struct and enum related
    Identifier(&'src str),
    Struct(Vec<StructField<'src>>),
    Enum(Vec<EnumVariant<'src>>),
    
    // Operations
    Union(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>),      // +
    Intersection(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>), // *
    Difference(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>),   // \
    SymmetricDiff(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>), // <>
}

#[derive(Debug, Clone)]
pub struct StructField<'src> {
    pub name: &'src str,
    pub type_expr: TypeExpr<'src>,
    pub validators: Vec<Validator<'src>>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant<'src> {
    pub name: &'src str,
    pub value: Option<EnumValue<'src>>,
}

#[derive(Debug, Clone)]
pub enum EnumValue<'src> {
    Number(i64),
    String(&'src str),
    Tuple(Vec<TypeExpr<'src>>),
}

#[derive(Debug, Clone)]
pub enum Validator<'src> {
    Regex(&'src str),
    Length(RangeExpr),
    MinLength(u64),
    MaxLength(u64),
    Min(f64),
    Max(f64),
    If {
        condition: Box<Validator<'src>>,
        then_validators: Vec<Validator<'src>>,
        else_validators: Option<Vec<Validator<'src>>>,
    },
    FieldRef(&'src str),
    Equal(Box<Validator<'src>>),
    NotEqual(Box<Validator<'src>>),
}

#[derive(Debug, Clone)]
pub struct RangeExpr {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub inclusive_start: bool,
    pub inclusive_end: bool,
}

pub struct Parser<'src> {
    tokens: Peekable<IntoIter<Token<'src>>>,
}

impl<'src> Parser<'src> {
    pub fn new(tokens: Vec<Token<'src>>) -> Self {
        Parser {
            tokens: tokens.into_iter().peekable(),
        }
    }

    pub fn parse_type_declaration(&mut self) -> Result<TypeExpr<'src>, String> {
        self.expect_token(Token::Type)?;
        let name = self.expect_identifier()?;
        self.expect_token(Token::Eq)?;
        let type_expr = self.parse_type_expr()?;
        Ok(type_expr)
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.peek_token() {
            Some(Token::LBracket(_)) => self.parse_list_or_tuple(),
            Some(Token::LCurly(_)) => self.parse_struct_or_enum(),
            Some(Token::Set(_)) => self.parse_set(),
            Some(Token::Identifier(_)) => {
                let id = self.expect_identifier()?;
                Ok(TypeExpr::Identifier(id))
            }
            _ => self.parse_primitive_type(),
        }
    }

    fn parse_list_or_tuple(&mut self) -> Result<TypeExpr<'src>, String> {
        self.expect_token(Token::LBracket)?;
        let mut elements = Vec::new();
        
        while !matches!(self.peek_token(), Some(Token::RBracket(_))) {
            elements.push(self.parse_type_expr()?);
            
            match self.peek_token() {
                Some(Token::Comma(_)) => {
                    self.next_token();
                }
                Some(Token::RBracket(_)) => break,
                _ => return Err("Expected comma or right bracket".to_string()),
            }
        }
        
        self.expect_token(Token::RBracket)?;

        if elements.len() == 1 {
            Ok(TypeExpr::List(Box::new(elements.remove(0))))
        } else {
            Ok(TypeExpr::Tuple(elements))
        }
    }

    fn parse_struct_or_enum(&mut self) -> Result<TypeExpr<'src>, String> {
        self.expect_token(Token::LCurly)?;
        let mut fields = Vec::new();
        
        while !matches!(self.peek_token(), Some(Token::RCurly(_))) {
            let name = self.expect_identifier()?;
            
            match self.peek_token() {
                Some(Token::Colon(_)) => {
                    // Struct field
                    self.next_token();
                    let type_expr = self.parse_type_expr()?;
                    let validators = self.parse_validators()?;
                    fields.push(StructField {
                        name,
                        type_expr,
                        validators,
                    });
                }
                Some(Token::LParen(_)) => {
                    // Enum variant with tuple
                    self.next_token();
                    let mut tuple_types = Vec::new();
                    while !matches!(self.peek_token(), Some(Token::RParen(_))) {
                        tuple_types.push(self.parse_type_expr()?);
                        if let Some(Token::Comma(_)) = self.peek_token() {
                            self.next_token();
                        }
                    }
                    self.expect_token(Token::RParen)?;
                    fields.push(EnumVariant {
                        name,
                        value: Some(EnumValue::Tuple(tuple_types)),
                    });
                }
                Some(Token::Comma(_)) | Some(Token::RCurly(_)) => {
                    // Simple enum variant
                    fields.push(EnumVariant {
                        name,
                        value: None,
                    });
                }
                _ => return Err("Expected : or , or ) in struct/enum declaration".to_string()),
            }
            
            if let Some(Token::Comma(_)) = self.peek_token() {
                self.next_token();
            }
        }
        
        self.expect_token(Token::RCurly)?;
        
        // Determine if it's a struct or enum based on the field types
        let is_enum = fields.iter().all(|f| matches!(f, EnumVariant { .. }));
        if is_enum {
            Ok(TypeExpr::Enum(fields.into_iter().map(|f| {
                match f {
                    EnumVariant { name, value } => EnumVariant { name, value },
                    _ => unreachable!(),
                }
            }).collect()))
        } else {
            Ok(TypeExpr::Struct(fields.into_iter().map(|f| {
                match f {
                    StructField { name, type_expr, validators } => 
                        StructField { name, type_expr, validators },
                    _ => unreachable!(),
                }
            }).collect()))
        }
    }

    fn parse_validators(&mut self) -> Result<Vec<Validator<'src>>, String> {
        let mut validators = Vec::new();
        
        while let Some(Token::At(_)) = self.peek_token() {
            self.next_token(); // consume @
            validators.push(self.parse_validator()?);
        }
        
        Ok(validators)
    }

    fn parse_validator(&mut self) -> Result<Validator<'src>, String> {
        match self.peek_token() {
            Some(Token::Identifier(td)) => {
                self.next_token();
                match td.v {
                    "len" => self.parse_length_validator(),
                    "min" => self.parse_min_validator(),
                    "max" => self.parse_max_validator(),
                    "if" => self.parse_if_validator(),
                    _ => Err(format!("Unknown validator: {}", td.v)),
                }
            }
            Some(Token::Regex(td)) => {
                self.next_token();
                Ok(Validator::Regex(td.v))
            }
            _ => Err("Expected validator".to_string()),
        }
    }

    fn next_token(&mut self) -> Option<Token<'src>> {
        self.tokens.next()
    }

    fn peek_token(&mut self) -> Option<&Token<'src>> {
        self.tokens.peek()
    }

    fn expect_token(&mut self, expected: Token<'src>) -> Result<(), String> {
        match self.next_token() {
            Some(token) if token == expected => Ok(()),
            Some(token) => Err(format!("Expected {:?}, got {:?}", expected, token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    fn expect_identifier(&mut self) -> Result<&'src str, String> {
        match self.next_token() {
            Some(Token::Identifier(td)) => Ok(td.v),
            Some(token) => Err(format!("Expected identifier, got {:?}", token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }
}use crate::tokens::{NumberType, Token};
use std::iter::Peekable;
use std::vec::IntoIter;

#[derive(Debug, Clone)]
pub enum TypeExpr<'src> {
    // Basic types
    Bool,
    Number(NumberType),
    String,
    
    // Complex types
    List(Box<TypeExpr<'src>>),  // For homogeneous lists like string[]
    Tuple(Vec<TypeExpr<'src>>), // For heterogeneous lists [string, number]
    Set(Box<TypeExpr<'src>>),   // For set types
    
    // Struct and enum related
    Identifier(&'src str),
    Struct(Vec<StructField<'src>>),
    Enum(Vec<EnumVariant<'src>>),
    
    // Operations
    Union(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>),      // +
    Intersection(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>), // *
    Difference(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>),   // \
    SymmetricDiff(Box<TypeExpr<'src>>, Box<TypeExpr<'src>>), // <>
}

#[derive(Debug, Clone)]
pub struct StructField<'src> {
    pub name: &'src str,
    pub type_expr: TypeExpr<'src>,
    pub validators: Vec<Validator<'src>>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant<'src> {
    pub name: &'src str,
    pub value: Option<EnumValue<'src>>,
}

#[derive(Debug, Clone)]
pub enum EnumValue<'src> {
    Number(i64),
    String(&'src str),
    Tuple(Vec<TypeExpr<'src>>),
}

#[derive(Debug, Clone)]
pub enum Validator<'src> {
    Regex(&'src str),
    Length(RangeExpr),
    MinLength(u64),
    MaxLength(u64),
    Min(f64),
    Max(f64),
    If {
        condition: Box<Validator<'src>>,
        then_validators: Vec<Validator<'src>>,
        else_validators: Option<Vec<Validator<'src>>>,
    },
    FieldRef(&'src str),
    Equal(Box<Validator<'src>>),
    NotEqual(Box<Validator<'src>>),
}

#[derive(Debug, Clone)]
pub struct RangeExpr {
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub inclusive_start: bool,
    pub inclusive_end: bool,
}

pub struct Parser<'src> {
    tokens: Peekable<IntoIter<Token<'src>>>,
}

impl<'src> Parser<'src> {
    pub fn new(tokens: Vec<Token<'src>>) -> Self {
        Parser {
            tokens: tokens.into_iter().peekable(),
        }
    }

    pub fn parse_type_declaration(&mut self) -> Result<TypeExpr<'src>, String> {
        // Expect: type <identifier> = <type_expr>
        match self.next_token() {
            Some(Token::Type(_)) => (),
            Some(token) => return Err(format!("Expected 'type', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        let name = self.expect_identifier()?;
        
        match self.next_token() {
            Some(Token::Eq(_)) => (),
            Some(token) => return Err(format!("Expected '=', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        let type_expr = self.parse_type_expr()?;
        Ok(type_expr)
    }

    fn parse_type_expr(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.peek_token() {
            Some(Token::LBracket(_)) => self.parse_list_or_tuple(),
            Some(Token::LCurly(_)) => self.parse_struct_or_enum(),
            Some(Token::Set(_)) => self.parse_set(),
            Some(Token::Identifier(_)) => {
                let id = self.expect_identifier()?;
                Ok(TypeExpr::Identifier(id))
            }
            _ => self.parse_primitive_type(),
        }
    }

    fn parse_list_or_tuple(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.next_token() {
            Some(Token::LBracket(_)) => (),
            Some(token) => return Err(format!("Expected '[', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        let mut elements = Vec::new();
        
        while let Some(token) = self.peek_token() {
            if matches!(token, Token::RBracket(_)) {
                break;
            }

            elements.push(self.parse_type_expr()?);
            
            match self.peek_token() {
                Some(Token::Comma(_)) => {
                    self.next_token();
                }
                Some(Token::RBracket(_)) => break,
                Some(token) => return Err(format!("Expected ',' or ']', got {:?}", token)),
                None => return Err("Unexpected end of input".to_string()),
            }
        }
        
        match self.next_token() {
            Some(Token::RBracket(_)) => (),
            Some(token) => return Err(format!("Expected ']', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        if elements.len() == 1 {
            Ok(TypeExpr::List(Box::new(elements.remove(0))))
        } else {
            Ok(TypeExpr::Tuple(elements))
        }
    }

    fn parse_struct_or_enum(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.next_token() {
            Some(Token::LCurly(_)) => (),
            Some(token) => return Err(format!("Expected '{{', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        let mut fields = Vec::new();
        
        while let Some(token) = self.peek_token() {
            if matches!(token, Token::RCurly(_)) {
                break;
            }

            let name = self.expect_identifier()?;
            
            match self.peek_token() {
                Some(Token::Colon(_)) => {
                    // Struct field
                    self.next_token();
                    let type_expr = self.parse_type_expr()?;
                    let validators = self.parse_validators()?;
                    fields.push(StructField {
                        name,
                        type_expr,
                        validators,
                    });
                }
                Some(Token::LParen(_)) => {
                    // Enum variant with tuple
                    self.next_token();
                    let mut tuple_types = Vec::new();
                    while let Some(token) = self.peek_token() {
                        if matches!(token, Token::RParen(_)) {
                            break;
                        }
                        tuple_types.push(self.parse_type_expr()?);
                        if let Some(Token::Comma(_)) = self.peek_token() {
                            self.next_token();
                        }
                    }
                    match self.next_token() {
                        Some(Token::RParen(_)) => (),
                        Some(token) => return Err(format!("Expected ')', got {:?}", token)),
                        None => return Err("Unexpected end of input".to_string()),
                    }
                    fields.push(EnumVariant {
                        name,
                        value: Some(EnumValue::Tuple(tuple_types)),
                    });
                }
                Some(Token::Comma(_)) | Some(Token::RCurly(_)) => {
                    // Simple enum variant
                    fields.push(EnumVariant {
                        name,
                        value: None,
                    });
                }
                Some(token) => return Err(format!("Expected ':', '(' or ',', got {:?}", token)),
                None => return Err("Unexpected end of input".to_string()),
            }
            
            if let Some(Token::Comma(_)) = self.peek_token() {
                self.next_token();
            }
        }
        
        match self.next_token() {
            Some(Token::RCurly(_)) => (),
            Some(token) => return Err(format!("Expected '}}', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }
        
        // Determine if it's a struct or enum based on the field types
        let is_enum = fields.iter().all(|f| matches!(f, EnumVariant { .. }));
        if is_enum {
            Ok(TypeExpr::Enum(fields.into_iter().map(|f| {
                match f {
                    EnumVariant { name, value } => EnumVariant { name, value },
                    _ => unreachable!(),
                }
            }).collect()))
        } else {
            Ok(TypeExpr::Struct(fields.into_iter().map(|f| {
                match f {
                    StructField { name, type_expr, validators } => 
                        StructField { name, type_expr, validators },
                    _ => unreachable!(),
                }
            }).collect()))
        }
    }

    fn parse_set(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.next_token() {
            Some(Token::Set(_)) => (),
            Some(token) => return Err(format!("Expected 'set', got {:?}", token)),
            None => return Err("Unexpected end of input".to_string()),
        }

        let inner = self.parse_type_expr()?;
        Ok(TypeExpr::Set(Box::new(inner)))
    }

    fn parse_primitive_type(&mut self) -> Result<TypeExpr<'src>, String> {
        match self.next_token() {
            Some(Token::Identifier(td)) => {
                match td.v {
                    "bool" => Ok(TypeExpr::Bool),
                    "string" => Ok(TypeExpr::String),
                    // Handle number types
                    "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" => {
                        let (signed, size) = match td.v {
                            "i8" => (true, 8),
                            "i16" => (true, 16),
                            "i32" => (true, 32),
                            "i64" => (true, 64),
                            "u8" => (false, 8),
                            "u16" => (false, 16),
                            "u32" => (false, 32),
                            "u64" => (false, 64),
                            "f32" => return Ok(TypeExpr::Number(NumberType::Float(32))),
                            "f64" => return Ok(TypeExpr::Number(NumberType::Float(64))),
                            _ => unreachable!(),
                        };
                        Ok(TypeExpr::Number(NumberType::Int(signed, size as u8)))
                    }
                    _ => Ok(TypeExpr::Identifier(td.v)),
                }
            }
            Some(token) => Err(format!("Expected type name, got {:?}", token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    fn parse_validators(&mut self) -> Result<Vec<Validator<'src>>, String> {
        let mut validators = Vec::new();
        
        while let Some(Token::At(_)) = self.peek_token() {
            self.next_token(); // consume @
            validators.push(self.parse_validator()?);
        }
        
        Ok(validators)
    }

    fn parse_validator(&mut self) -> Result<Validator<'src>, String> {
        match self.next_token() {
            Some(Token::Identifier(td)) => {
                match td.v {
                    "len" => self.parse_length_validator(),
                    "min" => self.parse_min_validator(),
                    "max" => self.parse_max_validator(),
                    "if" => self.parse_if_validator(),
                    _ => Err(format!("Unknown validator: {}", td.v)),
                }
            }
            Some(Token::Regex(td)) => Ok(Validator::Regex(td.v)),
            Some(token) => Err(format!("Expected validator, got {:?}", token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    // Helper methods
    fn next_token(&mut self) -> Option<Token<'src>> {
        self.tokens.next()
    }

    fn peek_token(&mut self) -> Option<&Token<'src>> {
        self.tokens.peek()
    }

    fn expect_identifier(&mut self) -> Result<&'src str, String> {
        match self.next_token() {
            Some(Token::Identifier(td)) => Ok(td.v),
            Some(token) => Err(format!("Expected identifier, got {:?}", token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    // To be implemented
    fn parse_length_validator(&mut self) -> Result<Validator<'src>, String> {
        Err("Length validator not implemented yet".to_string())
    }

    fn parse_min_validator(&mut self) -> Result<Validator<'src>, String> {
        Err("Min validator not implemented yet".to_string())
    }

    fn parse_max_validator(&mut self) -> Result<Validator<'src>, String> {
        Err("Max validator not implemented yet".to_string())
    }

    fn parse_if_validator(&mut self) -> Result<Validator<'src>, String> {
        Err("If validator not implemented yet".to_string())
    }
}