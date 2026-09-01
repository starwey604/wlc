/// A source location, using one-based line and column numbers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub offset: usize,
    pub length: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: Span,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schema {
    pub version: Spanned<u32>,
    /// Globally reserved message or enum IDs.
    pub reserved_ids: Vec<Spanned<u16>>,
    pub declarations: Vec<Declaration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Declaration {
    Message(Message),
    Enum(Enum),
}

impl Declaration {
    pub fn name(&self) -> &Spanned<String> {
        match self {
            Self::Message(message) => &message.name,
            Self::Enum(enumeration) => &enumeration.name,
        }
    }

    pub fn id(&self) -> &Spanned<u16> {
        match self {
            Self::Message(message) => &message.id,
            Self::Enum(enumeration) => &enumeration.id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub name: Spanned<String>,
    pub id: Spanned<u16>,
    /// Field numbers that cannot be reused.
    pub reserved_numbers: Vec<Spanned<u16>>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Enum {
    pub name: Spanned<String>,
    pub id: Spanned<u16>,
    /// Enum values that cannot be reused.
    pub reserved_numbers: Vec<Spanned<i32>>,
    pub values: Vec<EnumValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    pub cardinality: Cardinality,
    pub ty: Spanned<String>,
    pub name: Spanned<String>,
    pub number: Spanned<u16>,
    pub default: Option<Spanned<Literal>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cardinality {
    Optional,
    Required,
    Repeated,
    /// One optional, length-delimited occurrence containing exactly `count`
    /// fixed-width numeric elements.
    Packed(u16),
    /// One required, length-delimited occurrence containing exactly `count`
    /// fixed-width numeric elements.
    RequiredPacked(u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumValue {
    pub name: Spanned<String>,
    pub number: Spanned<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Literal {
    Integer(IntegerLiteral),
    Boolean(bool),
    String(String),
}

/// A decimal integer literal kept losslessly until its declared type is known.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerLiteral {
    pub negative: bool,
    pub magnitude: u64,
}

impl IntegerLiteral {
    pub fn as_i32(self) -> Option<i32> {
        if self.negative {
            let limit = i32::MAX as u64 + 1;
            if self.magnitude > limit {
                None
            } else if self.magnitude == limit {
                Some(i32::MIN)
            } else {
                Some(-(self.magnitude as i32))
            }
        } else {
            i32::try_from(self.magnitude).ok()
        }
    }

    pub fn as_i64(self) -> Option<i64> {
        if self.negative {
            let limit = i64::MAX as u64 + 1;
            if self.magnitude > limit {
                None
            } else if self.magnitude == limit {
                Some(i64::MIN)
            } else {
                Some(-(self.magnitude as i64))
            }
        } else {
            i64::try_from(self.magnitude).ok()
        }
    }

    pub fn as_u32(self) -> Option<u32> {
        (!self.negative)
            .then(|| u32::try_from(self.magnitude).ok())
            .flatten()
    }

    pub fn as_u64(self) -> Option<u64> {
        (!self.negative).then_some(self.magnitude)
    }
}

impl std::fmt::Display for IntegerLiteral {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negative && self.magnitude != 0 {
            write!(formatter, "-{}", self.magnitude)
        } else {
            write!(formatter, "{}", self.magnitude)
        }
    }
}
