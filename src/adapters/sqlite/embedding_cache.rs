//! Disposable derived vectors. Never authoritative memory or execution permission.
use super::SqliteStore;
use crate::error::AppError;
use sha2::{Digest, Sha256};
use sqlx::Row;
const MAX_BATCH: usize = 64;
const MAX_BYTES: usize = 2 * 1024 * 1024;
fn error() -> AppError {
    AppError::Message("derived embedding cache unavailable or invalid; rebuild required".into())
}
impl SqliteStore {
    /// Explicit installation/reset only. Authoritative tables are never touched.
    /// Vector contents are regenerated on the next authorized embedding calls.
    pub async fn rebuild_embedding_cache(&self) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(|_| error())?;
        sqlx::query("DROP TABLE IF EXISTS derived_embedding_cache")
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        sqlx::query("CREATE TABLE derived_embedding_cache(namespace TEXT NOT NULL, profile TEXT NOT NULL, purpose TEXT NOT NULL, content_digest TEXT NOT NULL, dimensions INTEGER NOT NULL, vector BLOB NOT NULL, checksum TEXT NOT NULL, PRIMARY KEY(namespace,profile,purpose,content_digest))").execute(&mut *tx).await.map_err(|_|error())?;
        sqlx::query("DROP TABLE IF EXISTS derived_embedding_cache_state")
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        sqlx::query("CREATE TABLE derived_embedding_cache_state(id INTEGER PRIMARY KEY CHECK(id=1),generation TEXT NOT NULL)").execute(&mut *tx).await.map_err(|_|error())?;
        sqlx::query("INSERT INTO derived_embedding_cache_state(id,generation) VALUES(1,?)")
            .bind(uuid::Uuid::new_v4().to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        tx.commit().await.map_err(|_| error())
    }
    pub(crate) async fn embedding_cache_generation(&self) -> Result<String, AppError> {
        let value:Option<String>=sqlx::query_scalar("SELECT CASE WHEN typeof(generation)='text' AND length(CAST(generation AS BLOB))=36 THEN generation ELSE NULL END FROM derived_embedding_cache_state WHERE id=1").fetch_one(&self.pool).await.map_err(|_|error())?;
        value.ok_or_else(error)
    }
    pub async fn clear_embedding_cache(&self) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.map_err(|_| error())?;
        sqlx::query("DELETE FROM derived_embedding_cache")
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        sqlx::query("UPDATE derived_embedding_cache_state SET generation=? WHERE id=1")
            .bind(uuid::Uuid::new_v4().to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| error())?;
        tx.commit().await.map_err(|_| error())
    }
    pub(crate) async fn load_cached_embeddings(
        &self,
        namespace: &str,
        profile: &str,
        purpose: &str,
        digests: &[String],
        dimensions: usize,
        generation: &str,
    ) -> Result<Option<Vec<Vec<f32>>>, AppError> {
        if digests.len() > MAX_BATCH || dimensions == 0 || dimensions > 8192 {
            return Err(error());
        }
        let mut tx = self.pool.begin().await.map_err(|_| error())?;
        let current:Option<String>=sqlx::query_scalar("SELECT CASE WHEN typeof(generation)='text' AND length(CAST(generation AS BLOB))=36 THEN generation ELSE NULL END FROM derived_embedding_cache_state WHERE id=1").fetch_one(&mut *tx).await.map_err(|_|error())?;
        if current.as_deref() != Some(generation) {
            return Err(error());
        }
        let mut total = 0usize;
        // Every length/type check completes in this snapshot before reading vectors.
        for digest in digests {
            let row=sqlx::query("SELECT typeof(dimensions) AS dt, CASE WHEN typeof(dimensions)='integer' THEN dimensions ELSE NULL END AS dimensions, typeof(vector) AS vt, length(vector) AS bytes, typeof(checksum) AS ct, length(CAST(checksum AS BLOB)) AS cb FROM derived_embedding_cache WHERE namespace=? AND profile=? AND purpose=? AND content_digest=?")
                .bind(namespace).bind(profile).bind(purpose).bind(digest).fetch_optional(&mut *tx).await.map_err(|_|error())?;
            let Some(row) = row else { return Ok(None) };
            let dt: String = row.try_get("dt").map_err(|_| error())?;
            let vt: String = row.try_get("vt").map_err(|_| error())?;
            let ct: String = row.try_get("ct").map_err(|_| error())?;
            let dims: i64 = row.try_get("dimensions").map_err(|_| error())?;
            let bytes: i64 = row.try_get("bytes").map_err(|_| error())?;
            let checksum_bytes: i64 = row.try_get("cb").map_err(|_| error())?;
            if dt != "integer"
                || vt != "blob"
                || ct != "text"
                || dims != dimensions as i64
                || bytes != dimensions as i64 * 4
                || checksum_bytes != 64
            {
                return Err(error());
            }
            total = total.checked_add(bytes as usize).ok_or_else(error)?;
            if total > MAX_BYTES {
                return Err(error());
            }
        }
        let mut result = Vec::with_capacity(digests.len());
        for digest in digests {
            let row=sqlx::query("SELECT vector,checksum FROM derived_embedding_cache WHERE namespace=? AND profile=? AND purpose=? AND content_digest=?")
                .bind(namespace).bind(profile).bind(purpose).bind(digest).fetch_one(&mut *tx).await.map_err(|_|error())?;
            let bytes: Vec<u8> = row.try_get("vector").map_err(|_| error())?;
            let checksum: String = row.try_get("checksum").map_err(|_| error())?;
            if !checksum
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || format!("{:x}", Sha256::digest(&bytes)) != checksum
            {
                return Err(error());
            }
            let vector: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            if vector.len() != dimensions
                || vector.iter().any(|v| !v.is_finite())
                || vector.iter().all(|v| *v == 0.0)
            {
                return Err(error());
            }
            result.push(vector);
        }
        tx.commit().await.map_err(|_| error())?;
        Ok(Some(result))
    }
    pub(crate) async fn store_cached_embeddings(
        &self,
        namespace: &str,
        profile: &str,
        purpose: &str,
        digests: &[String],
        vectors: &[Vec<f32>],
        generation: &str,
    ) -> Result<(), AppError> {
        if digests.len() != vectors.len()
            || vectors.len() > MAX_BATCH
            || vectors.iter().any(|v| {
                v.is_empty()
                    || v.len() > 8192
                    || v.iter().any(|n| !n.is_finite())
                    || v.iter().all(|n| *n == 0.0)
            })
        {
            return Err(error());
        }
        let mut tx = self.pool.begin().await.map_err(|_| error())?;
        let current:Option<String>=sqlx::query_scalar("SELECT CASE WHEN typeof(generation)='text' AND length(CAST(generation AS BLOB))=36 THEN generation ELSE NULL END FROM derived_embedding_cache_state WHERE id=1").fetch_one(&mut *tx).await.map_err(|_|error())?;
        if current.as_deref() != Some(generation) {
            return Err(error());
        }
        for (digest, vector) in digests.iter().zip(vectors) {
            let bytes: Vec<u8> = vector.iter().flat_map(|v| v.to_le_bytes()).collect();
            let checksum = format!("{:x}", Sha256::digest(&bytes));
            sqlx::query("INSERT OR REPLACE INTO derived_embedding_cache(namespace,profile,purpose,content_digest,dimensions,vector,checksum) VALUES(?,?,?,?,?,?,?)").bind(namespace).bind(profile).bind(purpose).bind(digest).bind(vector.len() as i64).bind(bytes).bind(checksum).execute(&mut *tx).await.map_err(|_|error())?;
        }
        // Bounded total storage, FIFO by insertion rowid; eviction affects ranking only.
        sqlx::query("DELETE FROM derived_embedding_cache WHERE rowid IN (SELECT rowid FROM derived_embedding_cache ORDER BY rowid DESC LIMIT -1 OFFSET 4096)").execute(&mut *tx).await.map_err(|_|error())?;
        tx.commit().await.map_err(|_| error())
    }
}
