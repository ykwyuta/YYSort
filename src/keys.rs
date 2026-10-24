use crate::{Result, encoding, error};
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, Zero};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use yy_encoding::Encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    CH,
    Text,
    BI,
    FI,
    PD,
    ZD,
    Number,
}
impl Format {
    pub fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_uppercase().as_str() {
            "CH" => Ok(Self::CH),
            "TEXT" => Ok(Self::Text),
            "BI" => Ok(Self::BI),
            "FI" => Ok(Self::FI),
            "PD" => Ok(Self::PD),
            "ZD" => Ok(Self::ZD),
            "NUM" | "NUMBER" => Ok(Self::Number),
            _ => Err(error(format!("unsupported field format: {name}"))),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Field {
    pub position: usize,
    pub length: usize,
    pub format: Format,
}
#[derive(Clone, Debug)]
pub struct Key {
    pub field: Field,
    pub descending: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Number {
    pub value: BigInt,
    pub scale: usize,
}
impl Number {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (negative, s) = if let Some(s) = s.strip_prefix('-') {
            (true, s)
        } else {
            (false, s.strip_prefix('+').unwrap_or(s))
        };
        let mut parts = s.split('.');
        let whole = parts.next().unwrap_or("");
        let fraction = parts.next().unwrap_or("");
        if parts.next().is_some()
            || whole.len() + fraction.len() == 0
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
        {
            return Err(error(
                "invalid decimal number (exponents and grouping separators are not accepted)",
            ));
        }
        let mut value = BigInt::parse_bytes(format!("{whole}{fraction}").as_bytes(), 10)
            .ok_or_else(|| error("invalid number"))?;
        if negative {
            value = -value;
        }
        let mut number = Self {
            value,
            scale: fraction.len(),
        };
        number.normalize();
        Ok(number)
    }
    fn normalize(&mut self) {
        if self.value.is_zero() {
            self.scale = 0;
            return;
        }
        while self.scale > 0 && (&self.value % 10u8).is_zero() {
            self.value /= 10u8;
            self.scale -= 1;
        }
    }
    pub fn add(&mut self, other: &Self) {
        if self.scale < other.scale {
            self.value *= BigInt::from(10u8).pow((other.scale - self.scale) as u32);
            self.scale = other.scale;
        }
        self.value += &other.value * BigInt::from(10u8).pow((self.scale - other.scale) as u32);
        self.normalize();
    }
    pub fn display(&self) -> String {
        let mut s = self.value.abs().to_string();
        if self.scale > 0 {
            if s.len() <= self.scale {
                s = format!("{}{}", "0".repeat(self.scale + 1 - s.len()), s);
            }
            s.insert(s.len() - self.scale, '.');
        }
        if self.value.sign() == Sign::Minus {
            s.insert(0, '-');
        }
        s
    }
}
impl Ord for Number {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.scale == other.scale {
            return self.value.cmp(&other.value);
        }
        let scale = self.scale.max(other.scale);
        (&self.value * BigInt::from(10u8).pow((scale - self.scale) as u32))
            .cmp(&(&other.value * BigInt::from(10u8).pow((scale - other.scale) as u32)))
    }
}
impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Value {
    Bytes(Vec<u8>),
    Number(Number),
}
impl Value {
    pub fn size(&self) -> usize {
        match self {
            Self::Bytes(v) => v.capacity(),
            Self::Number(n) => n.value.bits() as usize / 8 + 64,
        }
    }
}
pub fn decode(data: &[u8], format: Format, enc: Encoding) -> Result<Value> {
    if data.is_empty() {
        return Err(error("empty sort/numeric field"));
    }
    let n = match format {
        Format::CH => return Ok(Value::Bytes(data.to_vec())),
        Format::Text => return Ok(Value::Bytes(encoding::text(enc, data)?.into_bytes())),
        Format::Number => return Ok(Value::Number(Number::parse(&encoding::text(enc, data)?)?)),
        Format::BI => BigInt::from_bytes_be(Sign::Plus, data),
        Format::FI => BigInt::from_signed_bytes_be(data),
        Format::PD => {
            let mut digits = Vec::with_capacity(data.len() * 2);
            for (i, b) in data.iter().enumerate() {
                let high = b >> 4;
                let low = b & 15;
                if high > 9 {
                    return Err(error("invalid packed decimal digit"));
                }
                digits.push(b'0' + high);
                if i + 1 < data.len() {
                    if low > 9 {
                        return Err(error("invalid packed decimal digit"));
                    }
                    digits.push(b'0' + low);
                }
            }
            let sign = data[data.len() - 1] & 15;
            if !(10..=15).contains(&sign) {
                return Err(error("invalid packed decimal sign"));
            }
            let n =
                BigInt::parse_bytes(&digits, 10).ok_or_else(|| error("invalid packed decimal"))?;
            if matches!(sign, 11 | 13) { -n } else { n }
        }
        Format::ZD => {
            let mut digits = Vec::with_capacity(data.len());
            for (i, b) in data.iter().enumerate() {
                let zone = b >> 4;
                let digit = b & 15;
                if digit > 9
                    || (i + 1 < data.len() && zone != 15)
                    || (i + 1 == data.len() && !(10..=15).contains(&zone))
                {
                    return Err(error("invalid EBCDIC zoned decimal"));
                }
                digits.push(b'0' + digit);
            }
            let n =
                BigInt::parse_bytes(&digits, 10).ok_or_else(|| error("invalid zoned decimal"))?;
            if matches!(data[data.len() - 1] >> 4, 11 | 13) {
                -n
            } else {
                n
            }
        }
    };
    Ok(Value::Number(Number { value: n, scale: 0 }))
}
pub fn encode_number(n: &Number, format: Format, length: usize, enc: Encoding) -> Result<Vec<u8>> {
    if format == Format::Number {
        return encoding::bytes(enc, &n.display());
    }
    if n.scale != 0 {
        return Err(error(
            "fractional value cannot be written to an integer field",
        ));
    }
    let negative = n.value.sign() == Sign::Minus;
    let mut result = match format {
        Format::BI => {
            if negative {
                return Err(error("negative sum in unsigned binary field"));
            }
            n.value.to_bytes_be().1
        }
        Format::FI => n.value.to_signed_bytes_be(),
        Format::PD | Format::ZD => {
            let size = if format == Format::PD {
                length * 2 - 1
            } else {
                length
            };
            let s = n.value.abs().to_string();
            if s.len() > size {
                return Err(error("SUM decimal overflow"));
            }
            let digits = format!("{}{}", "0".repeat(size - s.len()), s).into_bytes();
            if format == Format::ZD {
                let mut out: Vec<_> = digits.iter().map(|b| 0xf0 + (b - b'0')).collect();
                out[length - 1] =
                    ((if negative { 0xd } else { 0xc }) << 4) + (digits[length - 1] - b'0');
                return Ok(out);
            }
            let mut nibbles: Vec<_> = digits.iter().map(|b| b - b'0').collect();
            nibbles.push(if negative { 13 } else { 12 });
            return Ok(nibbles
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| p[0] * 16 + p[1])
                .collect());
        }
        _ => return Err(error("SUM requires BI, FI, PD, ZD or CSV NUM")),
    };
    if result.len() > length {
        return Err(error("SUM binary overflow"));
    }
    let pad = if format == Format::FI && negative {
        255
    } else {
        0
    };
    if result.len() < length {
        let mut out = vec![pad; length - result.len()];
        out.append(&mut result);
        result = out;
    }
    if format == Format::FI && (result[0] & 128 != 0) != negative {
        return Err(error("SUM signed binary overflow"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_and_large_decimal_formats_keep_exact_values() {
        for format in [Format::BI, Format::FI, Format::PD, Format::ZD] {
            for text in ["-123456", "-1", "0", "1", "123456"] {
                let n = Number::parse(text).unwrap();
                if format == Format::BI && text.starts_with('-') {
                    assert!(encode_number(&n, format, 8, Encoding::Utf8).is_err());
                    continue;
                }
                let bytes = encode_number(&n, format, 8, Encoding::Utf8).unwrap();
                assert_eq!(
                    decode(&bytes, format, Encoding::Utf8).unwrap(),
                    Value::Number(n)
                );
            }
        }
        let a = Number::parse("-1.00").unwrap();
        let b = Number::parse("-0.99999").unwrap();
        assert!(a < b);
        let mut a = Number::parse("999999999999999999999999999999999999999999.25").unwrap();
        a.add(&Number::parse("0.75").unwrap());
        assert_eq!(a.display(), "1000000000000000000000000000000000000000000");
        assert!(Number::parse("NaN").is_err());
        assert!(Number::parse("1e9").is_err());
        assert!(decode(&[0x1a, 0x2c], Format::PD, Encoding::Utf8).is_err());
    }
}
