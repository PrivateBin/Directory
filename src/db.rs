#![allow(clippy::must_use_candidate, clippy::needless_borrow)]
use diesel::SqliteConnection;

#[database("directory")]
pub struct DirectoryDbConn(SqliteConnection);
