use crate::db::Db;
use crate::db::common::{is_unique_constraint_error, map_db_error, spawn_db};
use crate::db::schema::Event;
use crate::error::CcbdError;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn query_event_by_request_id_sync(
    conn: &Connection,
    agent_id: &str,
    request_id: &str,
) -> Result<Option<Event>, CcbdError> {
    conn.query_row(
        "SELECT seq_id, event_type, payload, created_at FROM events WHERE agent_id = ? AND request_id = ? LIMIT 1",
        params![agent_id, request_id],
        |row| {
            Ok(Event {
                seq_id: row.get(0)?,
                agent_id: agent_id.to_string(),
                request_id: Some(request_id.to_string()),
                event_type: row.get(1)?,
                payload: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|err| map_db_error("query event by request_id", err))
}

pub(crate) fn insert_event_sync(
    conn: &Connection,
    agent_id: &str,
    request_id: Option<&str>,
    event_type: &str,
    payload: &str,
) -> Result<i64, CcbdError> {
    let result = conn.execute(
        "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, ?, ?, ?)",
        params![agent_id, request_id, event_type, payload],
    );

    match result {
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(err) if is_unique_constraint_error(&err) && request_id.is_some() => {
            let existing_seq_id = conn
                .query_row(
                    "SELECT seq_id FROM events WHERE agent_id = ? AND request_id = ? LIMIT 1",
                    params![agent_id, request_id],
                    |row| row.get(0),
                )
                .map_err(|select_err| {
                    map_db_error("query duplicate event by request_id", select_err)
                })?;

            Err(CcbdError::DuplicateRequest { existing_seq_id })
        }
        Err(err) => Err(map_db_error("insert event", err)),
    }
}

pub(crate) fn query_events_since_sync(
    conn: &Connection,
    agent_id: &str,
    since_seq_id: i64,
) -> Result<Vec<Event>, CcbdError> {
    let mut stmt = conn
        .prepare(
            "SELECT seq_id, request_id, event_type, payload, created_at FROM events WHERE agent_id = ? AND seq_id > ? ORDER BY seq_id ASC",
        )
        .map_err(|err| map_db_error("prepare query events since", err))?;
    let rows = stmt
        .query_map(params![agent_id, since_seq_id], |row| {
            Ok(Event {
                seq_id: row.get(0)?,
                agent_id: agent_id.to_string(),
                request_id: row.get(1)?,
                event_type: row.get(2)?,
                payload: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|err| map_db_error("query events since", err))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| map_db_error("collect events since", err))
}

pub async fn query_event_by_request_id(
    db: Db,
    agent_id: String,
    request_id: String,
) -> Result<Option<Event>, CcbdError> {
    spawn_db("events::query_event_by_request_id", move || {
        let conn = db.conn();
        query_event_by_request_id_sync(&conn, &agent_id, &request_id)
    })
    .await
}

pub async fn insert_event(
    db: Db,
    agent_id: String,
    request_id: Option<String>,
    event_type: String,
    payload: String,
) -> Result<i64, CcbdError> {
    spawn_db("events::insert_event", move || {
        let conn = db.conn();
        insert_event_sync(
            &conn,
            &agent_id,
            request_id.as_deref(),
            &event_type,
            &payload,
        )
    })
    .await
}

pub async fn query_events_since(
    db: Db,
    agent_id: String,
    since_seq_id: i64,
) -> Result<Vec<Event>, CcbdError> {
    spawn_db("events::query_events_since", move || {
        let conn = db.conn();
        query_events_since_sync(&conn, &agent_id, since_seq_id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{insert_event_sync, query_event_by_request_id_sync, query_events_since_sync};
    use crate::db::agents::insert_agent_sync;
    use crate::db::init;
    use crate::db::sessions::insert_session_sync;
    use crate::error::CcbdError;

    fn with_test_db<T>(test: impl FnOnce(&mut rusqlite::Connection) -> T) -> T {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let mut conn = db.conn();
        test(&mut conn)
    }

    fn seed_agent(conn: &rusqlite::Connection) {
        insert_session_sync(conn, "s1", "p1", "/tmp/foo").unwrap();
        insert_agent_sync(conn, "a1", "s1", "bash", "IDLE", Some(123)).unwrap();
    }

    #[test]
    fn test_insert_event_idempotent() {
        with_test_db(|conn| {
            seed_agent(conn);
            let seq_id = insert_event_sync(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 1\n"}"#,
            )
            .unwrap();
            let err = insert_event_sync(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 2\n"}"#,
            )
            .unwrap_err();
            assert!(
                matches!(err, CcbdError::DuplicateRequest { existing_seq_id } if existing_seq_id == seq_id)
            );
        });
    }

    #[test]
    fn test_insert_event_no_request_id_no_unique() {
        with_test_db(|conn| {
            seed_agent(conn);
            let seq_1 =
                insert_event_sync(conn, "a1", None, "output_chunk", r#"{"text":"one"}"#).unwrap();
            let seq_2 =
                insert_event_sync(conn, "a1", None, "output_chunk", r#"{"text":"two"}"#).unwrap();
            assert_ne!(seq_1, seq_2);
        });
    }

    #[test]
    fn test_query_event_by_request_id_found_and_missing() {
        with_test_db(|conn| {
            seed_agent(conn);
            let seq_id = insert_event_sync(
                conn,
                "a1",
                Some("req-1"),
                "command_received",
                r#"{"cmd":"echo 1\n"}"#,
            )
            .unwrap();
            let found = query_event_by_request_id_sync(conn, "a1", "req-1")
                .unwrap()
                .unwrap();
            let missing = query_event_by_request_id_sync(conn, "a1", "req-2").unwrap();
            assert_eq!(found.seq_id, seq_id);
            assert_eq!(found.agent_id, "a1");
            assert_eq!(found.request_id.as_deref(), Some("req-1"));
            assert!(missing.is_none());
        });
    }

    #[test]
    fn test_query_events_since() {
        with_test_db(|conn| {
            seed_agent(conn);
            insert_event_sync(conn, "a1", None, "output_chunk", r#"{"text":"one"}"#).unwrap();
            let seq_2 =
                insert_event_sync(conn, "a1", None, "output_chunk", r#"{"text":"two"}"#).unwrap();
            let events = query_events_since_sync(conn, "a1", seq_2 - 1).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].seq_id, seq_2);
        });
    }
}
