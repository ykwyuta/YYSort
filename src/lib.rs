pub mod cards;
pub mod encoding;
pub mod engine;
pub mod keys;
pub mod outputs;
pub mod records;
pub mod resources;
mod routing;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    std::io::Error::other(message.into()).into()
}
