use crate::error::CcbdError;
use rusqlite::Error as SqlError;

pub(crate) fn is_constraint_error(err: &SqlError) -> bool {
    matches!(
        err,
        SqlError::SqliteFailure(sqlite_err, _)
            if sqlite_err.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

pub(crate) fn is_unique_constraint_error(err: &SqlError) -> bool {
    matches!(
        err,
        SqlError::SqliteFailure(sqlite_err, _)
            if sqlite_err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

pub(crate) fn map_db_error(context: &str, err: SqlError) -> CcbdError {
    CcbdError::DbConstraintViolation(format!("{context}: {err}"))
}
