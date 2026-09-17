//! Persistent full-text indexes for installed navigation editions.

use super::{NavigationDataset, NavigationPoint, NavigationSource, SearchResult};
use crate::{PlanningError, error::invalid};
use rusqlite::{Connection, OpenFlags, params};
use std::path::{Path, PathBuf};

/// A read-only index of one immutable source edition.
pub struct NavigationIndex {
    connection: Connection,
    path: PathBuf,
    source: NavigationSource,
}

impl NavigationIndex {
    /// Writes a complete index, then installs it without replacing an existing file.
    pub fn build_blocking(path: &Path, dataset: &NavigationDataset) -> Result<Self, PlanningError> {
        dataset.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|source| PlanningError::File {
                path: parent.into(),
                source,
            })?;
        let mut connection =
            Connection::open(temporary.path()).map_err(|s| database_error(path, s))?;
        write_database(&mut connection, dataset).map_err(|s| database_error(path, s))?;
        drop(connection);
        temporary
            .persist_noclobber(path)
            .map_err(|error| PlanningError::File {
                path: path.into(),
                source: error.error,
            })?;
        Self::open_blocking(path)
    }

    /// Opens a source index and rejects an unsupported schema.
    pub fn open_blocking(path: &Path) -> Result<Self, PlanningError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|s| database_error(path, s))?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|s| database_error(path, s))?;
        if version != 1 {
            return Err(invalid(
                path.display().to_string(),
                "unsupported navigation index schema",
            ));
        }
        let json: String = connection
            .query_row(
                "SELECT source_json FROM navigation_metadata WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|s| database_error(path, s))?;
        let source: NavigationSource = serde_json::from_str(&json)?;
        source.validate()?;
        connection.prepare("SELECT key,identifier,kind,name,region,latitude_deg,longitude_deg FROM points LIMIT 0")
            .map_err(|s| database_error(path, s))?;
        connection
            .prepare("SELECT rowid,identifier,name,region FROM points_search LIMIT 0")
            .map_err(|s| database_error(path, s))?;
        Ok(Self {
            connection,
            path: path.into(),
            source,
        })
    }

    /// Gets the exact edition identity held by this index.
    pub fn source(&self) -> &NavigationSource {
        &self.source
    }

    /// Searches identifiers, names, and regions. Each word is a literal prefix.
    pub fn search_blocking(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<SearchResult>, PlanningError> {
        if query.len() > 256 {
            return Err(invalid("query", "more than 256 bytes"));
        }
        let terms: Vec<_> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let expression = terms
            .iter()
            .map(|s| format!("\"{s}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = "SELECT p.key,p.identifier,p.kind,p.name,p.region,p.latitude_deg,p.longitude_deg
            FROM points_search JOIN points p ON p.rowid = points_search.rowid
            WHERE points_search MATCH ?1
            ORDER BY CASE WHEN p.identifier = ?2 COLLATE NOCASE THEN 0 ELSE 1 END,
                bm25(points_search, 10.0, 2.0, 1.0),p.identifier,p.region,p.key LIMIT ?3";
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(|s| database_error(&self.path, s))?;
        let points = statement
            .query_map(params![expression, query.trim(), limit.min(100)], |row| {
                Ok(NavigationPoint {
                    key: row.get(0)?,
                    identifier: row.get(1)?,
                    kind: row.get(2)?,
                    name: row.get(3)?,
                    region: row.get(4)?,
                    latitude_deg: row.get(5)?,
                    longitude_deg: row.get(6)?,
                })
            })
            .map_err(|s| database_error(&self.path, s))?;
        let mut results = Vec::new();
        for point in points {
            let point = point.map_err(|s| database_error(&self.path, s))?;
            point.validate()?;
            results.push(SearchResult {
                point,
                source: self.source.clone(),
            });
        }
        Ok(results)
    }
}

fn write_database(
    connection: &mut Connection,
    dataset: &NavigationDataset,
) -> Result<(), rusqlite::Error> {
    let transaction = connection.transaction()?;
    transaction.execute_batch("PRAGMA user_version = 1;
        CREATE TABLE navigation_metadata (id INTEGER PRIMARY KEY CHECK(id = 1), source_json TEXT NOT NULL);
        CREATE TABLE points (key TEXT NOT NULL UNIQUE, identifier TEXT NOT NULL, kind TEXT NOT NULL,
            name TEXT NOT NULL, region TEXT NOT NULL, latitude_deg REAL NOT NULL, longitude_deg REAL NOT NULL);
        CREATE VIRTUAL TABLE points_search USING fts5(identifier,name,region,tokenize='unicode61 remove_diacritics 2');")?;
    let json = serde_json::to_string(&dataset.source)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    transaction.execute("INSERT INTO navigation_metadata VALUES(1,?1)", [json])?;
    {
        let mut points = transaction.prepare("INSERT INTO points VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        let mut search = transaction.prepare(
            "INSERT INTO points_search(rowid,identifier,name,region) VALUES(?1,?2,?3,?4)",
        )?;
        for point in &dataset.points {
            points.execute(params![
                point.key,
                point.identifier,
                point.kind,
                point.name,
                point.region,
                point.latitude_deg,
                point.longitude_deg
            ])?;
            search.execute(params![
                transaction.last_insert_rowid(),
                point.identifier,
                point.name,
                point.region
            ])?;
        }
    }
    transaction.commit()
}

fn database_error(path: &Path, source: rusqlite::Error) -> PlanningError {
    PlanningError::Database {
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests;
