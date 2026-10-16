//! Encode an OS filename for SQLx's SQLite URL parser without changing its identity.
use std::path::Path;

/// SQLx splits a SQLite URL on `?` and percent-decodes the filename once. Encode
/// every non-unreserved byte (including `%` and Windows backslashes), preserving
/// canonical `\\?\` paths rather than mistaking their question mark for options.
/// Invalid UTF-8 filenames remain invalid when SQLx decodes them; never silently
/// replace an OS filename with a lossy alternative. This does not add URI options.
pub fn sqlite_file_url(path: &Path) -> String {
    let mut url = String::from("sqlite://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            url.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(&mut url, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
    use std::str::FromStr;

    #[test]
    fn sqlx_round_trip_preserves_windows_prefix_and_literal_uri_characters() {
        for raw in [
            r"\\?\C:\synthetic space\100%20 名.sqlite",
            r"C:\synthetic\100%25.sqlite",
            "/synthetic/100%20 ?# 名.sqlite",
        ] {
            let path = Path::new(raw);
            let url = sqlite_file_url(path);
            assert!(!url.contains('?'));
            assert!(!url.contains('#'));
            assert_eq!(
                SqliteConnectOptions::from_str(&url).unwrap().get_filename(),
                path
            );
        }
    }

    #[tokio::test]
    async fn canonical_filename_is_the_file_initialized_and_queried() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("literal%20 space 名.sqlite");
        let url = sqlite_file_url(&path);
        assert_eq!(
            SqliteConnectOptions::from_str(&url).unwrap().get_filename(),
            path
        );
        crate::adapters::sqlite::initialize_database(&url)
            .await
            .unwrap();
        assert!(path.is_file());
        assert!(!root.join("literal  space 名.sqlite").exists());
        let mut connection = SqliteConnection::connect_with(
            &SqliteConnectOptions::new().filename(&path).read_only(true),
        )
        .await
        .unwrap();
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            version,
            crate::adapters::sqlite::CURRENT_DATABASE_SCHEMA_VERSION
        );
        connection.close().await.unwrap();
    }
}
