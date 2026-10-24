//! yyeditor's codecs; record bytes are never transcoded for fixed record copying.
use crate::{Result, error};
use yy_encoding::{Encoding, EscapeMode, decode_all, encode_all};

pub fn parse(name: &str) -> Result<Encoding> {
    Encoding::from_name(name).ok_or_else(|| error(format!("unknown encoding: {name}")))
}
pub fn text(encoding: Encoding, bytes: &[u8]) -> Result<String> {
    let (decoded, stats) = decode_all(encoding, bytes, true);
    if stats.invalid > 0 || stats.literal_escapes > 0 {
        return Err(error(
            "invalid text in character/numeric field; use CH/BI for raw bytes",
        ));
    }
    String::from_utf8(decoded).map_err(Into::into)
}
pub fn bytes(encoding: Encoding, text: &str) -> Result<Vec<u8>> {
    encode_all(encoding, text.as_bytes(), EscapeMode::Reject)
        .map_err(|_| error(format!("text cannot be represented in {}", encoding.spec())))
}
