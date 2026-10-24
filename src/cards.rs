//! Explicit DFSORT subset. Unknown operands are errors, never ignored.
use crate::{
    Result, encoding, error,
    keys::{Field, Format, Key, Value},
};
use yy_encoding::Encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sort,
    Merge,
    Copy,
}
#[derive(Clone, Debug)]
pub enum Expr {
    Constant(bool),
    Compare(Field, Op, Value),
    CompareFields(Field, Op, Field),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
#[derive(Clone, Copy, Debug)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
impl Op {
    pub fn matches(self, order: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            Self::Eq => order == Equal,
            Self::Ne => order != Equal,
            Self::Lt => order == Less,
            Self::Le => order != Greater,
            Self::Gt => order == Greater,
            Self::Ge => order != Less,
        }
    }
}
#[derive(Clone, Debug)]
pub enum Piece {
    Field(usize, usize),
    Literal(Vec<u8>),
    Position(usize),
    Blank(usize),
}
#[derive(Clone, Debug, Default)]
pub struct Transform {
    pub pieces: Vec<Piece>,
    pub overlay: bool,
    pub padding: Vec<u8>,
}
impl Transform {
    pub fn output_length(&self, input: usize) -> Result<usize> {
        let mut cursor = 0usize;
        let mut length = if self.overlay { input } else { 0 };
        for piece in &self.pieces {
            let size = match piece {
                Piece::Position(p) => {
                    if *p == 0 {
                        return Err(error("output positions are one based"));
                    }
                    if !self.overlay && *p - 1 < cursor {
                        return Err(error(
                            "BUILD output columns must not overlap or move backwards",
                        ));
                    }
                    cursor = *p - 1;
                    continue;
                }
                Piece::Field(_, n) | Piece::Blank(n) => *n,
                Piece::Literal(v) => v.len(),
            };
            if (cursor > length || matches!(piece, Piece::Blank(_))) && self.padding.len() != 1 {
                return Err(error("blank padding requires a single-byte encoded space"));
            }
            cursor = cursor
                .checked_add(size)
                .ok_or_else(|| error("reformat length overflow"))?;
            length = length.max(cursor);
        }
        Ok(length)
    }
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub mode: Mode,
    pub keys: Vec<Key>,
    pub include: Option<Expr>,
    pub omit: Option<Expr>,
    pub inrec: Option<Transform>,
    pub outrec: Option<Transform>,
    pub sums: Option<Vec<Field>>,
    pub skip: u64,
    pub take: Option<u64>,
    pub outfils: Vec<Outfil>,
}
impl Default for Plan {
    fn default() -> Self {
        Self {
            mode: Mode::Sort,
            keys: Vec::new(),
            include: None,
            omit: None,
            inrec: None,
            outrec: None,
            sums: None,
            skip: 0,
            take: None,
            outfils: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outfil {
    pub names: Vec<String>,
    pub include: Option<Expr>,
    pub omit: Option<Expr>,
    pub save: bool,
    pub build: Option<Transform>,
    pub start: u64,
    pub end: Option<u64>,
    pub sample: (u64, u64),
    pub accept: Option<u64>,
    pub split: Option<(u64, bool)>,
}
pub fn ddname(s: &str) -> Result<String> {
    let name = s.to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 8
        || !name.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_uppercase() || matches!(b, b'@' | b'#' | b'$') || i > 0 && b.is_ascii_digit()
        })
    {
        return Err(error(format!("invalid DD name: {s}")));
    }
    Ok(name)
}
fn outfil(t: &[String], enc: Encoding) -> Result<Outfil> {
    let mut group = Outfil {
        names: vec!["SORTOUT".into()],
        include: None,
        omit: None,
        save: false,
        build: None,
        start: 1,
        end: None,
        sample: (1, 1),
        accept: None,
        split: None,
    };
    let mut seen = std::collections::HashSet::new();
    let mut i = 0;
    while i < t.len() {
        let name = t[i].to_ascii_uppercase();
        let canonical = match name.as_str() {
            "OUTREC" | "OVERLAY" => "BUILD",
            "SPLITBY" | "SPLIT1R" => "SPLIT",
            _ => &name,
        };
        if !seen.insert(canonical.to_owned()) {
            return Err(error(format!("duplicate OUTFIL operand: {name}")));
        }
        i += 1;
        if matches!(name.as_str(), "SAVE" | "SPLIT") {
            if name == "SAVE" {
                group.save = true;
            } else {
                group.split = Some((1, false));
            }
            continue;
        }
        if t.get(i).map(String::as_str) != Some("=") {
            return Err(error(format!("OUTFIL {name} requires =")));
        }
        i += 1;
        let start = i;
        if t.get(i).map(String::as_str) == Some("(") {
            let mut depth = 0;
            loop {
                let token = t.get(i).ok_or_else(|| error("unclosed OUTFIL operand"))?;
                if token == "(" {
                    depth += 1;
                }
                if token == ")" {
                    depth -= 1;
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
        } else {
            if i >= t.len() {
                return Err(error("missing OUTFIL value"));
            }
            i += 1;
        }
        let value = unwrap(&t[start..i]);
        let number = || -> Result<u64> {
            if value.len() != 1 {
                return Err(error("OUTFIL operand requires one number"));
            }
            let n = value[0].parse::<u64>()?;
            if n == 0 {
                return Err(error("OUTFIL record counts must be positive"));
            }
            Ok(n)
        };
        match name.as_str() {
            "FNAMES" => {
                if value.is_empty() {
                    return Err(error("empty FNAMES"));
                }
                group.names = value.iter().map(|s| ddname(s)).collect::<Result<_>>()?;
            }
            "INCLUDE" => group.include = Some(condition(value, enc)?),
            "OMIT" => group.omit = Some(condition(value, enc)?),
            "BUILD" | "OUTREC" | "OVERLAY" => {
                group.build = Some(transform(value, enc, name == "OVERLAY")?)
            }
            "STARTREC" => group.start = number()?,
            "ENDREC" => group.end = Some(number()?),
            "ACCEPT" => group.accept = Some(number()?),
            "SAMPLE" => {
                if !(1..=2).contains(&value.len()) {
                    return Err(error("SAMPLE requires n or (n,m)"));
                }
                let n = value[0].parse::<u64>()?;
                let m = if value.len() == 2 {
                    value[1].parse()?
                } else {
                    1
                };
                if n < 2 || m == 0 || m > n {
                    return Err(error("SAMPLE requires n>=2 and 1<=m<=n"));
                }
                group.sample = (n, m);
            }
            "SPLITBY" | "SPLIT1R" => group.split = Some((number()?, name == "SPLIT1R")),
            _ => return Err(error(format!("unsupported OUTFIL operand: {name}"))),
        }
    }
    if group.end.is_some_and(|n| n < group.start) {
        return Err(error("ENDREC must be >= STARTREC"));
    }
    if group.include.is_some() && group.omit.is_some() {
        return Err(error("OUTFIL INCLUDE and OMIT are mutually exclusive"));
    }
    if group.save && (group.include.is_some() || group.omit.is_some()) {
        return Err(error("SAVE cannot be combined with INCLUDE/OMIT"));
    }
    if group.names.len() > 64 {
        return Err(error("maximum 64 OUTFIL destinations"));
    }
    Ok(group)
}
pub fn tokens(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            current.push(ch);
            if quoted && chars.peek() == Some(&'\'') {
                current.push(chars.next().unwrap());
            } else {
                quoted = !quoted;
            }
            continue;
        }
        if !quoted && (ch == ',' || ch.is_whitespace() || matches!(ch, '(' | ')' | '=' | ':')) {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            if matches!(ch, '(' | ')' | '=' | ':') {
                out.push(ch.to_string());
            }
        } else {
            current.push(ch);
        }
    }
    if quoted {
        return Err(error("unterminated quoted control-card literal"));
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}
fn integer(s: &str) -> Result<usize> {
    let n = s.parse()?;
    if n == 0 {
        return Err(error("positions and lengths are one based and nonzero"));
    }
    Ok(n)
}
fn field(t: &[String]) -> Result<Field> {
    if t.len() != 3 {
        return Err(error("field requires position,length,format"));
    }
    Ok(Field {
        position: integer(&t[0])?,
        length: integer(&t[1])?,
        format: Format::parse(&t[2])?,
    })
}
fn literal(s: &str, enc: Encoding) -> Result<Option<Vec<u8>>> {
    if let Some(s) = s.strip_prefix("C'").or_else(|| s.strip_prefix("c'")) {
        return Ok(Some(encoding::bytes(
            enc,
            &s.strip_suffix('\'')
                .ok_or_else(|| error("bad character literal"))?
                .replace("''", "'"),
        )?));
    }
    if let Some(s) = s.strip_prefix("X'").or_else(|| s.strip_prefix("x'")) {
        let s = s
            .strip_suffix('\'')
            .ok_or_else(|| error("bad hex literal"))?;
        if s.len() % 2 != 0 || !s.is_ascii() {
            return Err(error("hex literal requires byte pairs"));
        }
        return Ok(Some(
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ));
    }
    Ok(None)
}
fn unwrap(mut t: &[String]) -> &[String] {
    loop {
        if t.first().map(String::as_str) != Some("(") || t.last().map(String::as_str) != Some(")") {
            break;
        }
        let mut depth = 0;
        let mut all = true;
        for (i, v) in t.iter().enumerate() {
            if v == "(" {
                depth += 1;
            }
            if v == ")" {
                depth -= 1;
            }
            if depth == 0 && i + 1 < t.len() {
                all = false;
                break;
            }
        }
        if !all {
            break;
        }
        t = &t[1..t.len() - 1];
    }
    t
}
pub fn condition(t: &[String], enc: Encoding) -> Result<Expr> {
    condition_at(t, enc, None, 0)
}
fn condition_at(
    t: &[String],
    enc: Encoding,
    default: Option<Format>,
    level: usize,
) -> Result<Expr> {
    if level > 64 {
        return Err(error("condition nesting/chain exceeds 64 levels"));
    }
    let t = unwrap(t);
    if t.len() == 1 {
        if t[0].eq_ignore_ascii_case("ALL") {
            return Ok(Expr::Constant(true));
        }
        if t[0].eq_ignore_ascii_case("NONE") {
            return Ok(Expr::Constant(false));
        }
    }
    for op in ["OR", "AND"] {
        let mut depth = 0i32;
        for (i, v) in t.iter().enumerate() {
            if v == "(" {
                depth += 1;
            }
            if v == ")" {
                depth -= 1;
            }
            if depth == 0 && v.eq_ignore_ascii_case(op) {
                let a = Box::new(condition_at(&t[..i], enc, default, level + 1)?);
                let b = Box::new(condition_at(&t[i + 1..], enc, default, level + 1)?);
                return Ok(if op == "OR" {
                    Expr::Or(a, b)
                } else {
                    Expr::And(a, b)
                });
            }
        }
    }
    fn compare_field(t: &[String], default: Option<Format>) -> Result<(Field, usize)> {
        if t.len() < 2 {
            return Err(error("compare field requires position,length[,format]"));
        }
        let explicit = t.get(2).and_then(|s| Format::parse(s).ok());
        let format = explicit
            .or(default)
            .ok_or_else(|| error("missing compare field format; use FORMAT="))?;
        Ok((
            Field {
                position: integer(&t[0])?,
                length: integer(&t[1])?,
                format,
            },
            if explicit.is_some() { 3 } else { 2 },
        ))
    }
    let (f, used) = compare_field(t, default)?;
    let op = match t.get(used).map(|s| s.to_ascii_uppercase()).as_deref() {
        Some("EQ") => Op::Eq,
        Some("NE") => Op::Ne,
        Some("LT") => Op::Lt,
        Some("LE") => Op::Le,
        Some("GT") => Op::Gt,
        Some("GE") => Op::Ge,
        _ => return Err(error("unsupported condition operator")),
    };
    let rhs = &t[used + 1..];
    if rhs.len() > 1 {
        let (other, count) = compare_field(rhs, default)?;
        if count != rhs.len() {
            return Err(error("unexpected tokens after compare field"));
        }
        let text = |f: Format| matches!(f, Format::CH | Format::Text);
        if text(f.format) != text(other.format) || text(f.format) && f.format != other.format {
            return Err(error("incompatible comparison field formats"));
        }
        if f.format == Format::CH && f.length != other.length {
            return Err(error(
                "CH field-to-field comparisons require equal declared lengths",
            ));
        }
        return Ok(Expr::CompareFields(f, op, other));
    }
    let value = rhs
        .first()
        .ok_or_else(|| error("missing comparison value"))?;
    let value = if let Some(bytes) = literal(value, enc)? {
        crate::keys::decode(&bytes, f.format, enc)?
    } else {
        Value::Number(crate::keys::Number::parse(value)?)
    };
    if matches!(
        (&value, f.format),
        (Value::Number(_), Format::CH | Format::Text)
    ) {
        return Err(error(
            "character comparisons require C'...' or X'...' literals",
        ));
    }
    Ok(Expr::Compare(f, op, value))
}
fn transform(t: &[String], enc: Encoding, overlay: bool) -> Result<Transform> {
    let t = unwrap(t);
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if t.get(i + 1).map(String::as_str) == Some(":") {
            pieces.push(Piece::Position(integer(&t[i])?));
            i += 2;
            if i >= t.len() || t.get(i + 1).map(String::as_str) == Some(":") {
                return Err(error(
                    "output column requires a following field or constant",
                ));
            }
        }
        if let Some(bytes) = literal(&t[i], enc)? {
            pieces.push(Piece::Literal(bytes));
            i += 1;
        } else if let Some(n) = t[i].strip_suffix('X').or_else(|| t[i].strip_suffix('x')) {
            pieces.push(Piece::Blank(if n.is_empty() { 1 } else { integer(n)? }));
            i += 1;
        } else {
            if i + 1 >= t.len() {
                return Err(error("BUILD requires position,length pairs or constants"));
            }
            pieces.push(Piece::Field(integer(&t[i])?, integer(&t[i + 1])?));
            i += 2;
        }
    }
    if pieces.is_empty() {
        return Err(error("empty BUILD is unsupported"));
    }
    let transform = Transform {
        pieces,
        overlay,
        padding: encoding::bytes(enc, " ")?,
    };
    // Check static size/column arithmetic without allocating the output.
    transform.output_length(if overlay { usize::MAX } else { 0 })?;
    Ok(transform)
}
fn condition_statement(t: &[String], enc: Encoding) -> Result<Expr> {
    if t.len() == 1 {
        return condition(t, enc);
    }
    let mut default = None;
    let mut expression = None;
    let mut i = 0;
    while i < t.len() {
        let name = t[i].to_ascii_uppercase();
        if t.get(i + 1).map(String::as_str) != Some("=") {
            return Err(error("INCLUDE/OMIT requires COND= and optional FORMAT="));
        }
        i += 2;
        let value = t.get(i).ok_or_else(|| error("missing condition operand"))?;
        if name == "FORMAT" {
            if default.is_some() {
                return Err(error("duplicate FORMAT"));
            }
            default = Some(Format::parse(value)?);
            i += 1;
        } else if name == "COND" {
            if expression.is_some() {
                return Err(error("duplicate COND"));
            }
            let start = i;
            if value == "(" {
                let mut depth = 0;
                loop {
                    let token = t.get(i).ok_or_else(|| error("unclosed COND"))?;
                    if token == "(" {
                        depth += 1;
                    }
                    if token == ")" {
                        depth -= 1;
                    }
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
            } else {
                i += 1;
            }
            expression = Some(&t[start..i]);
        } else {
            return Err(error(format!("unsupported condition operand: {name}")));
        }
    }
    condition_at(
        expression.ok_or_else(|| error("missing COND"))?,
        enc,
        default,
        0,
    )
}
pub fn parse(text: &str, enc: Encoding) -> Result<Plan> {
    let mut statements = Vec::new();
    let mut pending = String::new();
    let mut depth = 0i32;
    let mut quoted = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        if pending.len() + line.len() > 1024 * 1024 {
            return Err(error("control statement exceeds 1 MiB"));
        }
        pending.push(' ');
        pending.push_str(line);
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\'' {
                if quoted && chars.peek() == Some(&'\'') {
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            } else if !quoted {
                if ch == '(' {
                    depth += 1;
                }
                if ch == ')' {
                    depth -= 1;
                }
            }
        }
        if depth < 0 {
            return Err(error("unbalanced control statement"));
        }
        if !quoted && depth == 0 && !line.ends_with(',') {
            statements.push(std::mem::take(&mut pending));
        }
    }
    if !pending.trim().is_empty() {
        return Err(error("incomplete control statement"));
    }
    let mut p = Plan::default();
    let mut seen = std::collections::HashSet::new();
    for statement in statements {
        let t = tokens(&statement)?;
        if t.is_empty() {
            continue;
        }
        let name = t[0].to_ascii_uppercase();
        if !seen.insert(name.clone()) && name != "OUTFIL" {
            return Err(error(format!("duplicate statement: {name}")));
        }
        let tail = &t[1..];
        match name.as_str() {
            "OUTFIL" => {
                if p.outfils.len() >= 64 {
                    return Err(error("maximum 64 OUTFIL groups"));
                }
                p.outfils.push(outfil(tail, enc)?);
            }
            "SORT" | "MERGE" => {
                p.mode = if name == "SORT" {
                    Mode::Sort
                } else {
                    Mode::Merge
                };
                if tail.len() == 3
                    && tail[0].eq_ignore_ascii_case("FIELDS")
                    && tail[1] == "="
                    && tail[2].eq_ignore_ascii_case("COPY")
                {
                    p.mode = Mode::Copy;
                    continue;
                }
                if tail.len() < 3 || !tail[0].eq_ignore_ascii_case("FIELDS") || tail[1] != "=" {
                    return Err(error("SORT/MERGE requires FIELDS=(...)"));
                }
                let t = unwrap(&tail[2..]);
                if t.is_empty() || !t.len().is_multiple_of(4) {
                    return Err(error("sort keys require position,length,format,A|D"));
                }
                for chunk in t.as_chunks::<4>().0 {
                    let descending = match chunk[3].to_ascii_uppercase().as_str() {
                        "A" => false,
                        "D" => true,
                        _ => return Err(error("sort sequence must be A or D")),
                    };
                    p.keys.push(Key {
                        field: field(&chunk[..3])?,
                        descending,
                    });
                }
            }
            "INCLUDE" | "OMIT" => {
                let expr = condition_statement(tail, enc)?;
                if name == "INCLUDE" {
                    p.include = Some(expr);
                } else {
                    p.omit = Some(expr);
                }
            }
            "INREC" | "OUTREC" => {
                if tail.len() < 3
                    || !matches!(
                        tail[0].to_ascii_uppercase().as_str(),
                        "BUILD" | "FIELDS" | "OVERLAY"
                    )
                    || tail[1] != "="
                {
                    return Err(error("INREC/OUTREC requires BUILD/FIELDS/OVERLAY=(...)"));
                }
                let v = transform(&tail[2..], enc, tail[0].eq_ignore_ascii_case("OVERLAY"))?;
                if name == "INREC" {
                    p.inrec = Some(v);
                } else {
                    p.outrec = Some(v);
                }
            }
            "SUM" => {
                if tail.len() < 3 || !tail[0].eq_ignore_ascii_case("FIELDS") || tail[1] != "=" {
                    return Err(error(
                        "SUM requires FIELDS=NONE or (position,length,format,...)",
                    ));
                }
                let t = unwrap(&tail[2..]);
                let mut fields = Vec::new();
                if !(t.len() == 1 && t[0].eq_ignore_ascii_case("NONE")) {
                    if t.is_empty() || !t.len().is_multiple_of(3) {
                        return Err(error("SUM requires position,length,format triples"));
                    }
                    for chunk in t.as_chunks::<3>().0 {
                        fields.push(field(chunk)?);
                    }
                }
                p.sums = Some(fields);
            }
            "OPTION" => {
                let mut i = 0;
                while i < tail.len() {
                    match tail[i].to_ascii_uppercase().as_str() {
                        "COPY" => {
                            p.mode = Mode::Copy;
                            i += 1;
                        }
                        "EQUALS" => i += 1,
                        "SKIPREC" | "STOPAFT" => {
                            if tail.get(i + 1).map(String::as_str) != Some("=") {
                                return Err(error("OPTION requires =value"));
                            }
                            let n = tail
                                .get(i + 2)
                                .ok_or_else(|| error("missing OPTION value"))?
                                .parse()?;
                            if tail[i].eq_ignore_ascii_case("SKIPREC") {
                                p.skip = n;
                            } else {
                                p.take = Some(n);
                            }
                            i += 3;
                        }
                        _ => return Err(error(format!("unsupported OPTION: {}", tail[i]))),
                    }
                }
            }
            "END" if tail.is_empty() => {}
            _ => return Err(error(format!("unsupported DFSORT statement: {name}"))),
        }
    }
    if seen.contains("SORT") && seen.contains("MERGE") {
        return Err(error("SORT and MERGE are mutually exclusive"));
    }
    if p.mode != Mode::Copy && p.keys.is_empty() {
        return Err(error("SORT or MERGE with keys is required"));
    }
    if p.mode == Mode::Copy && p.sums.is_some() {
        return Err(error("SUM requires SORT/MERGE"));
    }
    Ok(p)
}
