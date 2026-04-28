use crate::db::{self, Db};
use crate::marker::{MarkerMatcher, MatchResult, registry};
use std::io::Read;
use std::sync::{Arc, Mutex};

pub fn spawn_pty_reader_task(
    agent_id: String,
    mut reader: Box<dyn Read + Send>,
    db: Arc<Db>,
    parser_handle: Arc<Mutex<vt100::Parser>>,
    matcher: Arc<MarkerMatcher>,
) {
    tokio::task::spawn_blocking(move || {
        let mut buf = [0_u8; 8192];

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let payload =
                        serde_json::json!({"text": String::from_utf8_lossy(chunk)}).to_string();
                    let insert_result = {
                        let conn = db.conn();
                        db::queries::insert_event(&conn, &agent_id, None, "output_chunk", &payload)
                    };
                    if let Err(err) = insert_result {
                        tracing::warn!(error = %err, "failed to persist pty output chunk");
                    }

                    let match_result = match parser_handle.lock() {
                        Ok(mut parser) => {
                            parser.process(chunk);
                            matcher.scan(&parser)
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "parser mutex poisoned in pty reader");
                            MatchResult::NoMatch
                        }
                    };
                    match match_result {
                        MatchResult::Matched => {
                            if let Err(err) = db::queries::mark_agent_idle_matched(&db, &agent_id) {
                                tracing::warn!(error = %err, "failed to mark agent IDLE after marker match");
                            }
                            if let Some(handle) = registry::take(&agent_id) {
                                let _ = handle.cancel_tx.send(());
                            }
                        }
                        MatchResult::NoMatch => registry::reset(&agent_id),
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

#[cfg(test)]
mod tests {
    use super::spawn_pty_reader_task;
    use crate::db::init;
    use crate::db::queries::{insert_agent, insert_session};
    use crate::marker::MarkerMatcher;
    use crate::pty::{PTY_MAP, spawn_agent};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
            Arc::new(db.clone()),
            Arc::new(Mutex::new(vt100::Parser::new(200, 200, 0))),
            Arc::new(MarkerMatcher::default()),
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
}
