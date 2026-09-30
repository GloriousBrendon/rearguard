// SPDX-License-Identifier: MIT OR Apache-2.0

//! Evidence store (SQLite).
//!
//! No seed, secret or resume token is ever written here: a session's seed can always
//! be re-derived from the master secret and its `(match_id, player_id)`.
//!
//! A session's ground-truth `label` (test environment only, see
//! [`rearguard_core::protocol::ClientMessage::Hello`]) is stored for the evaluation
//! harness. The detector never reads it: it lives only in the `sessions` table.
//!
//! Identifiers are `u64` in Rust and SQLite `INTEGER` (i64); the server only issues
//! values below 2^63, so they are stored unchanged.

use std::path::Path;
use std::sync::Mutex;

use rearguard_core::detect::{Evidence, Report};
use rearguard_core::protocol::{EvidenceSummary, SessionStatus, VerdictReport};
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id     INTEGER PRIMARY KEY,
    match_id       TEXT    NOT NULL,
    player_id      INTEGER NOT NULL,
    amplitude_ppm  INTEGER NOT NULL,
    client         TEXT    NOT NULL,
    label          TEXT,
    status         TEXT    NOT NULL,
    created_ms     INTEGER NOT NULL,
    ended_ms       INTEGER,
    records        INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS evidence (
    session_id   INTEGER NOT NULL REFERENCES sessions(session_id),
    scope        TEXT    NOT NULL,   -- 'window' or 'session'
    window_index INTEGER NOT NULL,   -- -1 for the session scope
    statistic    TEXT    NOT NULL,   -- 'error' or 'steps'
    start_ms INTEGER, end_ms INTEGER, pairs INTEGER,
    r REAL, slope REAL, slope_se REAL, residual_sd REAL, kappa REAL,
    z REAL, score REAL, confidence REAL, flagged INTEGER,
    PRIMARY KEY (session_id, scope, window_index, statistic)
);
CREATE TABLE IF NOT EXISTS verdicts (
    session_id       INTEGER PRIMARY KEY REFERENCES sessions(session_id),
    status           TEXT    NOT NULL,
    score            REAL    NOT NULL,
    confidence       REAL    NOT NULL,
    flagged          INTEGER NOT NULL,
    records          INTEGER NOT NULL,
    shots            INTEGER NOT NULL,
    engagements      INTEGER NOT NULL,
    angle_mismatches INTEGER NOT NULL,
    windows          INTEGER NOT NULL,
    flagged_windows  INTEGER NOT NULL
);
";

/// A newly opened session.
#[derive(Clone, Debug, PartialEq)]
pub struct NewSession {
    /// Session identifier.
    pub session_id: u64,
    /// Match identifier (probe key hierarchy).
    pub match_id: String,
    /// Player identifier (probe key hierarchy).
    pub player_id: u64,
    /// Drift amplitude, ppm.
    pub amplitude_ppm: u32,
    /// Client software name, as it reported itself.
    pub client: String,
    /// Ground-truth label for the evaluation harness, as the client reported it.
    pub label: Option<String>,
    /// Creation time, Unix milliseconds.
    pub created_ms: u64,
}

/// The SQLite database behind a mutex. Calls block; async callers use
/// `spawn_blocking`.
#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl Store {
    /// Opens (creating if needed) the database at `path`.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// An in-memory database, for tests.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        conn.execute_batch(SCHEMA)?;
        // Databases created before task 1.8 have no `label` column.
        let has_label = conn
            .prepare("SELECT 1 FROM pragma_table_info('sessions') WHERE name = 'label'")?
            .exists([])?;
        if !has_label {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN label TEXT;")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        f(&mut conn)
    }

    /// Records a new, open session.
    ///
    /// # Errors
    /// SQLite errors (including a duplicate session identifier).
    pub fn create_session(&self, s: &NewSession) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO sessions (session_id, match_id, player_id, amplitude_ppm, client, label, status,
                     created_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    to_i64(s.session_id),
                    s.match_id,
                    to_i64(s.player_id),
                    s.amplitude_ppm,
                    s.client,
                    s.label,
                    SessionStatus::Open.name(),
                    to_i64(s.created_ms)
                ],
            )?;
            Ok(())
        })
    }

    /// Stores completed evidence windows (both statistics each).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn add_windows(
        &self,
        session_id: u64,
        first_index: usize,
        windows: &[Report],
    ) -> rusqlite::Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            for (i, w) in windows.iter().enumerate() {
                let index = to_i64((first_index + i) as u64);
                insert_evidence(&tx, session_id, "window", index, "error", &w.error)?;
                insert_evidence(&tx, session_id, "window", index, "steps", &w.steps)?;
            }
            tx.commit()
        })
    }

    /// Closes a session: its final status, session-scope evidence and verdict, in one
    /// transaction.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn finish_session(
        &self,
        verdict: &VerdictReport,
        session: &Report,
        ended_ms: u64,
    ) -> rusqlite::Result<()> {
        self.with(|c| {
            let tx = c.transaction()?;
            let id = to_i64(verdict.session_id);
            tx.execute(
                "UPDATE sessions SET status = ?2, ended_ms = ?3, records = ?4 WHERE session_id = ?1",
                params![id, verdict.status.name(), to_i64(ended_ms), to_i64(verdict.records)],
            )?;
            insert_evidence(&tx, verdict.session_id, "session", -1, "error", &session.error)?;
            insert_evidence(&tx, verdict.session_id, "session", -1, "steps", &session.steps)?;
            tx.execute(
                "INSERT OR REPLACE INTO verdicts (session_id, status, score, confidence, flagged, records, shots,
                     engagements, angle_mismatches, windows, flagged_windows)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    verdict.status.name(),
                    verdict.score,
                    verdict.confidence,
                    verdict.flagged,
                    to_i64(verdict.records),
                    to_i64(verdict.shots),
                    to_i64(verdict.engagements),
                    to_i64(verdict.angle_mismatches),
                    to_i64(verdict.windows),
                    to_i64(verdict.flagged_windows)
                ],
            )?;
            tx.commit()
        })
    }

    /// The stored verdict of a closed session.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn verdict(&self, session_id: u64) -> rusqlite::Result<Option<VerdictReport>> {
        self.with(|c| {
            let id = to_i64(session_id);
            let head = c
                .query_row(
                    "SELECT status, score, confidence, flagged, records, shots, engagements, angle_mismatches,
                            windows, flagged_windows
                     FROM verdicts WHERE session_id = ?1",
                    params![id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, f64>(1)?,
                            r.get::<_, f64>(2)?,
                            r.get::<_, bool>(3)?,
                            [r.get::<_, i64>(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?],
                        ))
                    },
                )
                .optional()?;
            let Some((status, score, confidence, flagged, counts)) = head else {
                return Ok(None);
            };
            let evidence = |statistic: &str| -> rusqlite::Result<EvidenceSummary> {
                c.query_row(
                    "SELECT pairs, r, slope, slope_se, residual_sd, kappa, z, score, confidence, flagged
                     FROM evidence WHERE session_id = ?1 AND scope = 'session' AND statistic = ?2",
                    params![id, statistic],
                    |r| {
                        let real = |i: usize| r.get::<_, Option<f64>>(i).map(|v| v.unwrap_or(f64::NAN));
                        Ok(EvidenceSummary {
                            pairs: to_u64(r.get(0)?),
                            r: real(1)?,
                            slope: real(2)?,
                            slope_se: real(3)?,
                            residual_sd: real(4)?,
                            kappa: real(5)?,
                            z: real(6)?,
                            score: real(7)?,
                            confidence: real(8)?,
                            flagged: r.get(9)?,
                        })
                    },
                )
            };
            Ok(Some(VerdictReport {
                session_id,
                status: SessionStatus::from_name(&status).unwrap_or(SessionStatus::Abandoned),
                score,
                confidence,
                flagged,
                records: to_u64(counts[0]),
                shots: to_u64(counts[1]),
                engagements: to_u64(counts[2]),
                angle_mismatches: to_u64(counts[3]),
                error: evidence("error")?,
                steps: evidence("steps")?,
                windows: to_u64(counts[4]),
                flagged_windows: to_u64(counts[5]),
            }))
        })
    }

    /// Status of every stored session, oldest first: `(session_id, status)`.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn sessions(&self) -> rusqlite::Result<Vec<(u64, SessionStatus)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT session_id, status FROM sessions ORDER BY created_ms, session_id",
            )?;
            let rows = stmt.query_map([], |r| Ok((to_u64(r.get(0)?), r.get::<_, String>(1)?)))?;
            rows.map(|row| {
                row.map(|(id, s)| {
                    (
                        id,
                        SessionStatus::from_name(&s).unwrap_or(SessionStatus::Abandoned),
                    )
                })
            })
            .collect()
        })
    }

    /// Every stored session that has a ground-truth label, oldest first:
    /// `(session_id, label, status)`. For the evaluation harness only.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn labelled_sessions(&self) -> rusqlite::Result<Vec<(u64, String, SessionStatus)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT session_id, label, status FROM sessions WHERE label IS NOT NULL
                 ORDER BY created_ms, session_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    to_u64(r.get(0)?),
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.map(|row| {
                row.map(|(id, label, s)| {
                    (
                        id,
                        label,
                        SessionStatus::from_name(&s).unwrap_or(SessionStatus::Abandoned),
                    )
                })
            })
            .collect()
        })
    }

    /// Number of stored window evidence rows for a session (two per window).
    ///
    /// # Errors
    /// SQLite errors.
    pub fn window_rows(&self, session_id: u64) -> rusqlite::Result<u64> {
        self.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM evidence WHERE session_id = ?1 AND scope = 'window'",
                params![to_i64(session_id)],
                |r| r.get::<_, i64>(0),
            )
            .map(to_u64)
        })
    }
}

fn insert_evidence(
    c: &Connection,
    session_id: u64,
    scope: &str,
    window_index: i64,
    statistic: &str,
    e: &Evidence,
) -> rusqlite::Result<()> {
    // Non-finite values (an infinite kappa) are stored as NULL.
    let finite = |v: f64| v.is_finite().then_some(v);
    c.execute(
        "INSERT OR REPLACE INTO evidence (session_id, scope, window_index, statistic, start_ms, end_ms, pairs,
             r, slope, slope_se, residual_sd, kappa, z, score, confidence, flagged)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            to_i64(session_id),
            scope,
            window_index,
            statistic,
            to_i64(e.start_ms),
            to_i64(e.end_ms),
            to_i64(e.pairs),
            finite(e.r),
            finite(e.slope),
            finite(e.slope_se),
            finite(e.residual_sd),
            finite(e.kappa),
            finite(e.z),
            finite(e.score),
            finite(e.confidence),
            e.flagged
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(score: f64, flagged: bool) -> Evidence {
        Evidence {
            start_ms: 0,
            end_ms: 30_000,
            pairs: 40,
            r: 0.5,
            slope: 1.0,
            slope_se: 0.1,
            residual_sd: 0.2,
            kappa: f64::INFINITY,
            z: 5.0,
            score,
            confidence: 0.99,
            flagged,
        }
    }

    fn report(score: f64) -> Report {
        Report {
            error: evidence(score, true),
            steps: evidence(-1.0, false),
            shots: 10,
            engagements: 5,
            angle_mismatches: 0,
        }
    }

    #[test]
    fn a_session_round_trips_through_the_store() {
        let store = Store::open_in_memory().unwrap();
        store
            .create_session(&NewSession {
                session_id: (1 << 62) + 5,
                match_id: "m-1".into(),
                player_id: 1,
                amplitude_ppm: 5000,
                client: "test".into(),
                label: None,
                created_ms: 1,
            })
            .unwrap();
        assert_eq!(
            store.sessions().unwrap(),
            vec![((1 << 62) + 5, SessionStatus::Open)]
        );
        store
            .add_windows((1 << 62) + 5, 0, &[report(3.0), report(4.0)])
            .unwrap();
        assert_eq!(store.window_rows((1 << 62) + 5).unwrap(), 4);
        assert_eq!(
            store.verdict((1 << 62) + 5).unwrap(),
            None,
            "no verdict while open"
        );

        let r = report(20.0);
        let verdict = VerdictReport {
            session_id: (1 << 62) + 5,
            status: SessionStatus::Finished,
            score: 20.0,
            confidence: 0.99,
            flagged: true,
            records: 123,
            shots: 10,
            engagements: 5,
            angle_mismatches: 0,
            error: EvidenceSummary::from(&r.error),
            steps: EvidenceSummary::from(&r.steps),
            windows: 2,
            flagged_windows: 2,
        };
        store.finish_session(&verdict, &r, 99).unwrap();
        let back = store.verdict((1 << 62) + 5).unwrap().unwrap();
        // Infinite kappa is stored as NULL and read back as NaN; everything else exact.
        assert!(back.error.kappa.is_nan());
        let mut expected = verdict.clone();
        expected.error.kappa = back.error.kappa;
        expected.steps.kappa = back.steps.kappa;
        assert_eq!(format!("{back:?}"), format!("{expected:?}"));
        assert_eq!(
            store.sessions().unwrap(),
            vec![((1 << 62) + 5, SessionStatus::Finished)]
        );
    }

    #[test]
    fn labels_are_stored_and_listed() {
        let store = Store::open_in_memory().unwrap();
        for (id, label) in [
            (1, None),
            (2, Some("cheat:flick-aimbot")),
            (3, Some("bot:scripted")),
        ] {
            store
                .create_session(&NewSession {
                    session_id: id,
                    match_id: format!("m-{id}"),
                    player_id: 1,
                    amplitude_ppm: 5000,
                    client: "test".into(),
                    label: label.map(Into::into),
                    created_ms: id,
                })
                .unwrap();
        }
        assert_eq!(
            store.labelled_sessions().unwrap(),
            vec![
                (2, "cheat:flick-aimbot".into(), SessionStatus::Open),
                (3, "bot:scripted".into(), SessionStatus::Open),
            ]
        );
    }

    #[test]
    fn a_database_from_before_labels_gains_the_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&SCHEMA.replace("    label          TEXT,\n", ""))
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, match_id, player_id, amplitude_ppm, client, status, created_ms)
             VALUES (9, 'm-9', 1, 5000, 'old', 'finished', 1)",
            [],
        )
        .unwrap();
        let store = Store::init(conn).unwrap();
        assert!(store.labelled_sessions().unwrap().is_empty());
        assert_eq!(
            store.sessions().unwrap(),
            vec![(9, SessionStatus::Finished)]
        );
    }
}
