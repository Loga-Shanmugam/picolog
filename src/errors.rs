use thiserror::Error;

#[derive(Error, Debug)]
pub enum PicoError {
    #[error("Page Full: Cannot write here. Move to the next page")]
    PageFull {},
}
