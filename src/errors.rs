use thiserror::Error;

/// Custom error type for the library.
#[derive(Error, Debug)]
pub enum PicoError {
    /// Indicates that the current page is full and cannot accept more data.
    #[error("Page Full: Cannot write here. Move to the next page")]
    PageFull {},
}
