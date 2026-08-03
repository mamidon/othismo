//! What a token is.

use crate::span::Span;

/// A lexed token: a kind and where it came from, and nothing else.
///
/// No payload. Identifiers aren't `String`s and numbers aren't `f64`s — a
/// literal's *value* is decoded on demand from its span by
/// [`crate::literal_value`]. The language server retokenizes the whole buffer
/// on every keystroke, so zero allocation per token matters more than
/// convenience; and a tree of spans over the original text is losslessness,
/// where a tree of decoded values isn't.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Token {
        Token { kind, span }
    }

    /// The source text this token covers.
    pub fn text<'src>(&self, source: &'src str) -> &'src str {
        self.span.text(source)
    }

    pub fn is_trivia(&self) -> bool {
        self.kind.is_trivia()
    }
}

/// Every token the language has.
///
/// Keywords are their own variants rather than `Keyword(Keyword)`, so the
/// parser matches on one enum.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TokenKind {
    // ---- Trivia -----------------------------------------------------------
    // Emitted, but skipped by `Tokens::significant`. §1 says comments produce
    // no tokens; the parser needs a lossless tree. Both hold if comments are
    // tokens the grammar never sees.
    /// Also covers a leading byte-order mark.
    Whitespace,
    LineComment,
    BlockComment,

    // ---- Comments the grammar cares about ---------------------------------
    /// `///`. Attaches to the following declaration (§1, §14).
    DocComment,

    // ---- Names ------------------------------------------------------------
    Ident,

    // ---- Keywords ---------------------------------------------------------
    // Reserved, not contextual (§1). `for` and `in` have no construct yet (§4
    // declines every loop but `while`) and are reserved anyway.
    As,
    Break,
    Continue,
    Else,
    Export,
    False,
    Fn,
    For,
    If,
    Import,
    In,
    Let,
    Match,
    Mut,
    Return,
    Struct,
    True,
    Type,
    While,

    // ---- Literals ---------------------------------------------------------
    /// `42`, `0xff`, `0b1010`, `1_000u32`
    Int,
    /// `1.5`, `.5`, `1e10`, `1.0f32`
    Float,
    /// `"…"`
    Str,
    /// `r"…"`
    RawStr,
    /// `"""…"""`
    MultilineStr,
    /// `'x'`
    Char,

    // ---- Delimiters -------------------------------------------------------
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    // ---- Punctuation ------------------------------------------------------
    Comma,
    Semicolon,
    Colon,
    /// `::` — lexed, but no construct uses it yet (§13 owns paths).
    ColonColon,
    Dot,
    /// `..` — lexed, but ranges are deferred (§2).
    DotDot,
    /// `...` — lexed, but nothing uses it yet.
    DotDotDot,
    /// `->`
    Arrow,

    // ---- Operators --------------------------------------------------------
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    AmpAmp,
    PipePipe,
    Shl,
    Shr,
    Eq,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,

    // ---- Recovery and end -------------------------------------------------
    /// Text that isn't a token. Always accompanied by a diagnostic.
    Unknown,
    /// An empty token at the end of the source, so the parser's lookahead has
    /// no special case.
    Eof,
}

impl TokenKind {
    pub fn is_trivia(&self) -> bool {
        matches!(
            self,
            TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
        )
    }

    pub fn is_keyword(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            As | Break
                | Continue
                | Else
                | Export
                | False
                | Fn
                | For
                | If
                | Import
                | In
                | Let
                | Match
                | Mut
                | Return
                | Struct
                | True
                | Type
                | While
        )
    }

    pub fn is_literal(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Int | Float | Str | RawStr | MultilineStr | Char | True | False
        )
    }

    /// The reserved word this text spells, if any.
    pub fn keyword(text: &str) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match text {
            "as" => As,
            "break" => Break,
            "continue" => Continue,
            "else" => Else,
            "export" => Export,
            "false" => False,
            "fn" => Fn,
            "for" => For,
            "if" => If,
            "import" => Import,
            "in" => In,
            "let" => Let,
            "match" => Match,
            "mut" => Mut,
            "return" => Return,
            "struct" => Struct,
            "true" => True,
            "type" => Type,
            "while" => While,
            _ => return None,
        })
    }

    /// Whether a token of this kind could be the end of an expression.
    ///
    /// The whole of §1's "lexing and left context": a `.` followed by a digit
    /// begins a float literal *unless* the preceding significant token could
    /// end an expression, in which case it's field access. This is the only
    /// lexical decision that depends on anything but the cursor.
    pub fn can_end_expression(&self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Ident
                | Int
                | Float
                | Str
                | RawStr
                | MultilineStr
                | Char
                | True
                | False
                | RParen
                | RBracket
                | RBrace
        )
    }

    /// How to name this token in a message.
    pub fn describe(&self) -> &'static str {
        use TokenKind::*;
        match self {
            Whitespace => "whitespace",
            LineComment => "a line comment",
            BlockComment => "a block comment",
            DocComment => "a doc comment",
            Ident => "an identifier",
            Int => "an integer literal",
            Float => "a float literal",
            Str | RawStr | MultilineStr => "a string literal",
            Char => "a character literal",
            Unknown => "unrecognized text",
            Eof => "end of file",
            other => other.spelling().unwrap_or("a token"),
        }
    }

    /// The literal source text of a token whose spelling is fixed.
    pub fn spelling(&self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            As => "as",
            Break => "break",
            Continue => "continue",
            Else => "else",
            Export => "export",
            False => "false",
            Fn => "fn",
            For => "for",
            If => "if",
            Import => "import",
            In => "in",
            Let => "let",
            Match => "match",
            Mut => "mut",
            Return => "return",
            Struct => "struct",
            True => "true",
            Type => "type",
            While => "while",
            LParen => "(",
            RParen => ")",
            LBrace => "{",
            RBrace => "}",
            LBracket => "[",
            RBracket => "]",
            Comma => ",",
            Semicolon => ";",
            Colon => ":",
            ColonColon => "::",
            Dot => ".",
            DotDot => "..",
            DotDotDot => "...",
            Arrow => "->",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Amp => "&",
            Pipe => "|",
            Caret => "^",
            Tilde => "~",
            Bang => "!",
            AmpAmp => "&&",
            PipePipe => "||",
            Shl => "<<",
            Shr => ">>",
            Eq => "=",
            EqEq => "==",
            BangEq => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            PlusEq => "+=",
            MinusEq => "-=",
            StarEq => "*=",
            SlashEq => "/=",
            PercentEq => "%=",
            AmpEq => "&=",
            PipeEq => "|=",
            CaretEq => "^=",
            ShlEq => "<<=",
            ShrEq => ">>=",
            _ => return None,
        })
    }
}
