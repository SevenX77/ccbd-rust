use crate::db::queries::insert_event;
use rusqlite::{Connection, OptionalExtension, params};
use std::io::Read;
use std::sync::{Arc, Mutex};

pub fn spawn_pty_reader_task(
    agent_id: String,
    mut reader: Box<dyn Read + Send>,
    db: Arc<Mutex<Connection>>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = [0_u8; 4096];

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let conn = match db.lock() {
                        Ok(conn) => conn,
                        Err(err) => {
                            tracing::warn!(error = %err, "pty reader db mutex poisoned");
                            break;
                        }
                    };

                    if let Err(err) = insert_event(&conn, &agent_id, None, "output_chunk", &chunk) {
                        tracing::warn!(error = %err, "failed to persist pty output chunk");
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "pty reader failed");
                    break;
                }
            }
        }
    });
}

pub fn spawn_child_wait_task(
    agent_id: String,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    db: Arc<Mutex<Connection>>,
) {
    tokio::task::spawn_blocking(move || {
        let exit_status_result = child.wait();
        let exit_code = match exit_status_result {
            Ok(status) => Some(status.exit_code() as i64),
            Err(err) => {
                tracing::warn!(error = %err, "child wait failed");
                None
            }
        };

        let update_result = (|| -> rusqlite::Result<()> {
            let mut conn = match db.lock() {
                Ok(conn) => conn,
                Err(err) => {
                    tracing::warn!(error = %err, "child wait db mutex poisoned");
                    return Ok(());
                }
            };
            let tx = conn.transaction()?;
            let prev_state = tx
                .query_row(
                    "SELECT state FROM agents WHERE id = ?",
                    params![agent_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            let Some(prev_state) = prev_state else {
                tracing::warn!(agent_id = %agent_id, "agent missing during child wait");
                return Ok(());
            };

            let payload = serde_json::json!({
                "from": prev_state,
                "to": "CRASHED",
                "reason": "PROCESS_EXIT",
                "exit_code": exit_code,
            })
            .to_string();

            tx.execute(
                "INSERT INTO events (agent_id, request_id, event_type, payload) VALUES (?, NULL, 'state_change', ?)",
                params![agent_id.as_str(), payload],
            )?;
            tx.execute(
                "UPDATE agents SET state = 'CRASHED', state_version = state_version + 1, exit_code = ?, updated_at = unixepoch() WHERE id = ? AND state != 'CRASHED'",
                params![exit_code, agent_id.as_str()],
            )?;
            tx.commit()?;

            Ok(())
        })();

        if let Err(err) = update_result {
            tracing::warn!(error = %err, "failed to persist child exit state");
        }

        match crate::pty::PTY_MAP.lock() {
            Ok(mut pty_map) => {
                let _ = pty_map.remove(&agent_id);
            }
            Err(err) => tracing::warn!(error = %err, "PTY_MAP mutex poisoned during child wait"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{spawn_child_wait_task, spawn_pty_reader_task};
    use crate::db::init;
    use crate::db::queries::{insert_agent, insert_session};
    use crate::pty::{PTY_MAP, spawn_agent};
    use rusqlite::OptionalExtension;
    use std::io::Write;
    use std::time::{Duration, Instant};

    fn remove_writer(agent_id: &str) {
        PTY_MAP.lock().unwrap().remove(agent_id);
    }

    fn write_to_agent(agent_id: &str, bytes: &[u8]) {
        let mut pty_map = PTY_MAP.lock().unwrap();
        match pty_map.get_mut(agent_id) {
            Some(writer) => {
                writer.write_all(bytes).unwrap();
                writer.flush().unwrap();
            }
            None => panic!("missing PTY writer for {agent_id}"),
        }
    }

    async fn sleep_ms(ms: u64) {
        tokio::task::spawn_blocking(move || std::thread::sleep(Duration::from_millis(ms)))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reader_writes_output_chunks_to_db() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let db_agent_id = format!("ag_r_{}", uuid::Uuid::new_v4());
        let pty_agent_id = format!("ag_r_test_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(&conn, &db_agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let spawn_result = spawn_agent(&pty_agent_id, "bash").unwrap();
        spawn_pty_reader_task(
            db_agent_id.clone(),
            spawn_result.master_reader,
            db.0.clone(),
        );
        write_to_agent(&pty_agent_id, b"echo kiro_test\n");

        sleep_ms(100).await;

        write_to_agent(&pty_agent_id, b"exit\n");
        sleep_ms(100).await;

        let found = {
            let conn = db.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT payload FROM events WHERE agent_id = ? AND event_type = 'output_chunk'",
                )
                .unwrap();
            let rows = stmt
                .query_map([db_agent_id.as_str()], |row| row.get::<_, String>(0))
                .unwrap();
            rows.collect::<Result<Vec<_>, _>>().unwrap()
        };

        remove_writer(&pty_agent_id);

        assert!(
            found.iter().any(|payload| payload.contains("kiro_test")),
            "payloads={found:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_child_wait_marks_crashed_and_clears_pty_map() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let db = init(file.path()).unwrap();
        let agent_id = format!("ag_w_{}", uuid::Uuid::new_v4());

        {
            let conn = db.conn();
            insert_session(&conn, "s1", "p1", "/tmp/foo", 999).unwrap();
            insert_agent(&conn, &agent_id, "s1", "bash", "IDLE", None).unwrap();
        }

        let spawn_result = spawn_agent(&agent_id, "bash").unwrap();
        spawn_pty_reader_task(agent_id.clone(), spawn_result.master_reader, db.0.clone());
        spawn_child_wait_task(agent_id.clone(), spawn_result.child, db.0.clone());
        write_to_agent(&agent_id, b"exit\n");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut transitioned = false;
        while Instant::now() < deadline {
            let state: Option<String> = {
                let conn = db.conn();
                conn.query_row(
                    "SELECT state FROM agents WHERE id = ?",
                    [agent_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .unwrap()
            };

            if state.as_deref() == Some("CRASHED") {
                transitioned = true;
                break;
            }

            sleep_ms(50).await;
        }
        assert!(
            transitioned,
            "agent did not transition to CRASHED within 5s"
        );

        let (state, state_version, exit_code, event_type, payload) = {
            let conn = db.conn();
            let (state, state_version, exit_code): (String, i64, Option<i64>) = conn
                .query_row(
                    "SELECT state, state_version, exit_code FROM agents WHERE id = ?",
                    [agent_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let (event_type, payload): (String, String) = conn
                .query_row(
                    "SELECT event_type, payload FROM events WHERE agent_id = ? AND event_type = 'state_change'",
                    [agent_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();

            (state, state_version, exit_code, event_type, payload)
        };
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let writer_present = PTY_MAP.lock().unwrap().contains_key(&agent_id);

        assert_eq!(state, "CRASHED");
        assert_eq!(state_version, 2);
        assert_eq!(exit_code, Some(0));
        assert_eq!(event_type, "state_change");
        assert_eq!(payload["to"], "CRASHED");
        assert_eq!(payload["reason"], "PROCESS_EXIT");
        assert_eq!(payload["exit_code"], 0);
        assert!(!writer_present);
    }
}
