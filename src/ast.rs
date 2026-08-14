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
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Enum {
    pub name: Spanned<String>,
    pub id: Spanned<u16>,
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
    Repeated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnumValue {
    pub name: Spanned<String>,
    pub number: Spanned<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Literal {
    Integer(i64),
    Boolean(bool),
    String(String),
}
