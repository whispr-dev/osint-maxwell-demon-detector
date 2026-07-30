use thiserror::Error;

#[derive(Debug, Error)]
pub enum EntropyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("column not found: {0}")]
    ColumnNotFound(String),

    #[error("failed to parse numeric value '{value}' in column '{column}'")]
    ParseFloat { column: String, value: String },

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("empty input")]
    EmptyInput,

    #[error("insufficient data: needed at least {needed}, found {found}")]
    InsufficientData { needed: usize, found: usize },

    #[error("variance is non-positive")]
    NonPositiveVariance,

    #[error("matrix is singular or near-singular")]
    SingularMatrix,
}
