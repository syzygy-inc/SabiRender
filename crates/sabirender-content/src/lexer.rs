//! PDF 内容ストリームの字句解析（ISO 32000-1 §7.2）。
//! 数、名前、文字列（括弧と 16 進）、配列、辞書、演算子を返す。インライン画像 `BI … ID … EI` はひとかたまりで返す。

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Name(String),
    String(Vec<u8>),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    Operator(String),
    /// `BI` から `EI` まで（画素データを含む）。辞書部は未解釈
    InlineImage(Vec<u8>),
    True,
    False,
    Null,
}

pub struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(c: u8) -> bool {
    matches!(
        c,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lexer<'a> {
    pub fn new(s: &'a [u8]) -> Self {
        Lexer { s, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(c) = self.peek() {
            if is_ws(c) {
                self.pos += 1;
            } else if c == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\n' || c == b'\r' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn next_token(&mut self) -> Option<Token> {
        self.skip_ws_and_comments();
        let c = self.peek()?;
        match c {
            b'[' => {
                self.pos += 1;
                Some(Token::ArrayOpen)
            }
            b']' => {
                self.pos += 1;
                Some(Token::ArrayClose)
            }
            b'<' => {
                if self.s.get(self.pos + 1) == Some(&b'<') {
                    self.pos += 2;
                    Some(Token::DictOpen)
                } else {
                    self.pos += 1;
                    Some(Token::String(self.hex_string()))
                }
            }
            b'>' => {
                if self.s.get(self.pos + 1) == Some(&b'>') {
                    self.pos += 2;
                    Some(Token::DictClose)
                } else {
                    // 孤立した '>' は読み飛ばす
                    self.pos += 1;
                    self.next_token()
                }
            }
            b'(' => {
                self.pos += 1;
                Some(Token::String(self.literal_string()))
            }
            b'/' => {
                self.pos += 1;
                Some(Token::Name(self.name()))
            }
            b'{' | b'}' | b')' => {
                self.pos += 1;
                self.next_token()
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_ws(c) || is_delim(c) {
                        break;
                    }
                    self.pos += 1;
                }
                Some(Token::Number(parse_number(&self.s[start..self.pos])))
            }
            _ => {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_ws(c) || is_delim(c) {
                        break;
                    }
                    self.pos += 1;
                }
                let word = &self.s[start..self.pos];
                match word {
                    b"true" => Some(Token::True),
                    b"false" => Some(Token::False),
                    b"null" => Some(Token::Null),
                    b"BI" => Some(Token::InlineImage(self.inline_image(start))),
                    _ => Some(Token::Operator(String::from_utf8_lossy(word).into_owned())),
                }
            }
        }
    }

    fn name(&mut self) -> String {
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            if is_ws(c) || is_delim(c) {
                break;
            }
            self.pos += 1;
            if c == b'#' {
                if let (Some(h), Some(l)) = (
                    self.s.get(self.pos).and_then(|c| hexval(*c)),
                    self.s.get(self.pos + 1).and_then(|c| hexval(*c)),
                ) {
                    out.push((h << 4) | l);
                    self.pos += 2;
                    continue;
                }
            }
            out.push(c);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn hex_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'>' {
                break;
            }
            if let Some(v) = hexval(c) {
                match hi {
                    None => hi = Some(v),
                    Some(h) => {
                        out.push((h << 4) | v);
                        hi = None;
                    }
                }
            }
        }
        if let Some(h) = hi {
            out.push(h << 4);
        }
        out
    }

    fn literal_string(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut depth = 1;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                b'\\' => {
                    let Some(n) = self.peek() else { break };
                    self.pos += 1;
                    match n {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'\n' => {}
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'0'..=b'7' => {
                            let mut v = (n - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// `BI` の直後から `EI` の直後までを返す。`EI` は空白に挟まれた語として探す
    fn inline_image(&mut self, start: usize) -> Vec<u8> {
        let mut i = self.pos;
        while i + 2 <= self.s.len() {
            if &self.s[i..i + 2] == b"EI"
                && (i == 0 || is_ws(self.s[i - 1]))
                && (i + 2 == self.s.len() || is_ws(self.s[i + 2]) || is_delim(self.s[i + 2]))
            {
                self.pos = i + 2;
                return self.s[start..self.pos].to_vec();
            }
            i += 1;
        }
        self.pos = self.s.len();
        self.s[start..].to_vec()
    }
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// PDF の実数（`.5`、`-.002`、`6.` などを許す。不正なら 0）
fn parse_number(s: &[u8]) -> f64 {
    let mut neg = false;
    let mut i = 0;
    while i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        if s[i] == b'-' {
            neg = !neg;
        }
        i += 1;
    }
    let mut int = 0.0f64;
    while i < s.len() && s[i].is_ascii_digit() {
        int = int * 10.0 + (s[i] - b'0') as f64;
        i += 1;
    }
    let mut frac = 0.0f64;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let mut scale = 0.1;
        while i < s.len() && s[i].is_ascii_digit() {
            frac += (s[i] - b'0') as f64 * scale;
            scale *= 0.1;
            i += 1;
        }
    }
    let v = int + frac;
    if neg {
        -v
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(s: &str) -> Vec<Token> {
        let mut lx = Lexer::new(s.as_bytes());
        let mut v = Vec::new();
        while let Some(t) = lx.next_token() {
            v.push(t);
        }
        v
    }

    #[test]
    fn lexes_numbers_names_strings_and_operators() {
        let t = all("q 1 0 0 1 .5 -.25 cm /F1 12 Tf (a\\)b) Tj <41 42> Tj [1 2] TJ Q");
        assert_eq!(t[0], Token::Operator("q".into()));
        assert_eq!(t[5], Token::Number(0.5));
        assert_eq!(t[6], Token::Number(-0.25));
        assert_eq!(t[8], Token::Name("F1".into()));
        assert_eq!(t[11], Token::String(b"a)b".to_vec()));
        assert_eq!(t[13], Token::String(b"AB".to_vec()));
        assert_eq!(t[15], Token::ArrayOpen);
    }

    #[test]
    fn skips_comments_and_reads_inline_images_as_one_token() {
        let t = all("% comment\n1 g BI /W 1 /H 1 /CS /G /BPC 8 ID \x00 EI 0 g");
        assert_eq!(t[0], Token::Number(1.0));
        assert!(matches!(t[2], Token::InlineImage(_)));
        assert_eq!(t[3], Token::Number(0.0));
        assert_eq!(t[4], Token::Operator("g".into()));
    }

    #[test]
    fn names_decode_hash_escapes() {
        assert_eq!(all("/A#20B")[0], Token::Name("A B".into()));
    }
}
