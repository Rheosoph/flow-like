use std::fmt;

/// Errors produced while fitting or applying an ordinal model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrdinalError {
    /// The training matrix had no rows, or no columns.
    EmptyTrainingSet,
    /// Records and targets disagreed on the number of rows.
    LengthMismatch { records: usize, targets: usize },
    /// Ordinal models need at least two ordered levels to have anything to separate.
    TooFewClasses { found: usize },
    /// A target rank was outside `0..n_classes`.
    RankOutOfRange { rank: usize, n_classes: usize },
    /// A feature row did not match the width the model was fitted on.
    FeatureWidthMismatch { expected: usize, found: usize },
    /// A hyperparameter was outside its valid range.
    InvalidParameter { name: &'static str, reason: String },
    /// The training data produced a non-finite value the optimizer cannot recover from.
    NonFinite { context: &'static str },
    /// The normal equations were not positive definite, so the ridge solve failed.
    NotPositiveDefinite,
    /// An error raised by linfa itself.
    ///
    /// `linfa::traits::Fit` requires `E: From<linfa::error::Error>`, so this variant is what lets
    /// the ordinal estimators plug into linfa's trait machinery.
    Linfa(String),
}

impl From<linfa::error::Error> for OrdinalError {
    fn from(error: linfa::error::Error) -> Self {
        OrdinalError::Linfa(error.to_string())
    }
}

impl fmt::Display for OrdinalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrdinalError::EmptyTrainingSet => {
                write!(
                    f,
                    "training set is empty: need at least one row and one feature"
                )
            }
            OrdinalError::LengthMismatch { records, targets } => {
                write!(f, "records has {records} rows but targets has {targets}")
            }
            OrdinalError::TooFewClasses { found } => write!(
                f,
                "ordinal models need at least 2 ordered levels, found {found}"
            ),
            OrdinalError::RankOutOfRange { rank, n_classes } => write!(
                f,
                "target rank {rank} is outside the {n_classes} known levels"
            ),
            OrdinalError::FeatureWidthMismatch { expected, found } => write!(
                f,
                "model was fitted on {expected} features but received {found}"
            ),
            OrdinalError::InvalidParameter { name, reason } => {
                write!(f, "invalid parameter `{name}`: {reason}")
            }
            OrdinalError::NonFinite { context } => write!(
                f,
                "{context} produced a non-finite value; scale the features and try a larger penalty"
            ),
            OrdinalError::NotPositiveDefinite => write!(
                f,
                "the penalized normal equations were not positive definite; increase the penalty"
            ),
            OrdinalError::Linfa(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for OrdinalError {}

pub type Result<T> = std::result::Result<T, OrdinalError>;
