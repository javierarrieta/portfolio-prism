use rocket::http::Status;

/// Why a write was refused before it ever reached a database constraint.
///
/// The variants are named after the **cause**, not after the HTTP status, because
/// two callers now consume this type: the Rocket handlers map it to the status
/// code each one returned before this seam existed (the `From` impl below is that
/// mapping, and it is the record of the equivalence), while the staged-import
/// path maps the same values to machine-readable error codes for an agent.
///
/// `Database` is the only variant that means "the database said no and we have no
/// more specific story". A constraint violation that a caller could have
/// prevented belongs in a named variant, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteError {
    /// Another asset in this portfolio already uses this symbol.
    SymbolExists,
    /// Another asset in this portfolio already carries this ISIN.
    IsinExists,
    /// The ISIN is present but is not 12 alphanumeric characters with a valid
    /// check digit.
    IsinInvalid,
    /// A `SPLIT` with a zero or negative quantity or price. A split ratio of
    /// zero is not a ratio.
    InvalidSplit,
    /// The transaction date falls outside 1900–2100.
    DateOutOfRange,
    /// The database refused the write for a reason with no user-facing meaning.
    Database,
}

impl From<WriteError> for Status {
    fn from(err: WriteError) -> Status {
        match err {
            // The symbol collision answered 400 and the ISIN collision answered
            // 409 before the extraction; that asymmetry is pre-existing and is
            // preserved deliberately rather than tidied, because tidying it is a
            // breaking change to the live API and belongs in its own commit.
            WriteError::SymbolExists => Status::BadRequest,
            WriteError::IsinExists => Status::Conflict,
            WriteError::IsinInvalid => Status::BadRequest,
            WriteError::InvalidSplit => Status::BadRequest,
            WriteError::DateOutOfRange => Status::BadRequest,
            WriteError::Database => Status::InternalServerError,
        }
    }
}
