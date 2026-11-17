use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

pub struct History(pub Mutex<Connection>);

#[derive(Serialize, Debug, PartialEq)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Source {
    pub url: String,
    pub title: String,
}

#[derive(Serialize, Debug)]
pub struct HistoryMessage {
    pub question: String,
    pub images: Vec<String>,
    pub answer: String,
    pub model: String,
    pub sources: Vec<Source>,
}

#[derive(Serialize, Debug)]
pub struct ConversationDetail {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub created_at: i64,
    pub messages: Vec<HistoryMessage>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS conversations(
    id INTEGER PRIMARY KEY,
    title TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages(
    id INTEGER PRIMARY KEY,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    question TEXT NOT NULL,
    images TEXT NOT NULL,
    answer TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    model TEXT NOT NULL DEFAULT 'haiku',
    sources TEXT NOT NULL DEFAULT '[]'
);
";

fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(SCHEMA)?;
    let (has_model, has_sources) = {
        let mut stmt = conn.prepare("PRAGMA table_info(messages)")?;
        let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
        let mut model = false;
        let mut sources = false;
        for n in names {
            let n = n?;
            if n == "model" {
                model = true;
            } else if n == "sources" {
                sources = true;
            }
        }
        (model, sources)
    };
    if !has_model {
        conn.execute_batch("ALTER TABLE messages ADD COLUMN model TEXT NOT NULL DEFAULT 'haiku';")?;
    }
    if !has_sources {
        conn.execute_batch("ALTER TABLE messages ADD COLUMN sources TEXT NOT NULL DEFAULT '[]';")?;
    }
    Ok(())
}

pub fn open(app: &AppHandle) -> Result<History, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let conn = Connection::open(dir.join("history.db")).map_err(|e| e.to_string())?;
    init(&conn).map_err(|e| e.to_string())?;
    Ok(History(Mutex::new(conn)))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn make_title(question: &str) -> String {
    let first = question.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        "(이미지)".to_string()
    } else {
        first.chars().take(60).collect()
    }
}

fn escape_like(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    for c in q.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn add(
    conn: &Connection,
    conversation_id: Option<i64>,
    provider: &str,
    model: &str,
    question: &str,
    images: &[String],
    answer: &str,
    sources: &[Source],
) -> rusqlite::Result<i64> {
    let now = now_ms();
    let cid = match conversation_id {
        Some(id) => id,
        None => {
            conn.execute(
                "INSERT INTO conversations(title, provider, model, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![make_title(question), provider, model, now],
            )?;
            conn.last_insert_rowid()
        }
    };
    let seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE conversation_id = ?1",
        params![cid],
        |r| r.get(0),
    )?;
    let images_json = serde_json::to_string(images).unwrap_or_else(|_| "[]".to_string());
    let sources_json = serde_json::to_string(sources).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO messages(conversation_id, seq, question, images, answer, created_at, model, sources) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![cid, seq, question, images_json, answer, now, model, sources_json],
    )?;
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now, cid],
    )?;
    Ok(cid)
}

fn list(conn: &Connection, query: &str, limit: i64) -> rusqlite::Result<Vec<ConversationSummary>> {
    let map = |r: &rusqlite::Row| {
        Ok(ConversationSummary {
            id: r.get(0)?,
            title: r.get(1)?,
            provider: r.get(2)?,
            model: r.get(3)?,
            updated_at: r.get(4)?,
        })
    };
    if query.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT id, title, provider, model, updated_at FROM conversations ORDER BY updated_at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], map)?;
        rows.collect()
    } else {
        let pat = format!("%{}%", escape_like(query));
        let mut stmt = conn.prepare(
            "SELECT id, title, provider, model, updated_at FROM conversations c
             WHERE c.title LIKE ?1 ESCAPE '\\'
                OR EXISTS (SELECT 1 FROM messages m WHERE m.conversation_id = c.id
                           AND (m.question LIKE ?1 ESCAPE '\\' OR m.answer LIKE ?1 ESCAPE '\\'))
             ORDER BY updated_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pat, limit], map)?;
        rows.collect()
    }
}

fn get(conn: &Connection, id: i64) -> rusqlite::Result<ConversationDetail> {
    let (title, provider, model, created_at) = conn.query_row(
        "SELECT title, provider, model, created_at FROM conversations WHERE id = ?1",
        params![id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    let mut stmt = conn.prepare(
        "SELECT question, images, answer, model, sources FROM messages WHERE conversation_id = ?1 ORDER BY seq",
    )?;
    let messages = stmt
        .query_map(params![id], |r| {
            let images: String = r.get(1)?;
            let sources: String = r.get(4)?;
            Ok(HistoryMessage {
                question: r.get(0)?,
                images: serde_json::from_str(&images).unwrap_or_default(),
                answer: r.get(2)?,
                model: r.get(3)?,
                sources: serde_json::from_str(&sources).unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ConversationDetail {
        id,
        title,
        provider,
        model,
        created_at,
        messages,
    })
}

fn delete(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
    Ok(())
}

#[tauri::command]
pub fn history_add(
    state: tauri::State<History>,
    conversation_id: Option<i64>,
    provider: String,
    model: String,
    question: String,
    images: Vec<String>,
    answer: String,
    sources: Option<Vec<Source>>,
) -> Result<i64, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    add(
        &conn,
        conversation_id,
        &provider,
        &model,
        &question,
        &images,
        &answer,
        &sources.unwrap_or_default(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_list(
    state: tauri::State<History>,
    query: String,
    limit: i64,
) -> Result<Vec<ConversationSummary>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    list(&conn, &query, limit).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_get(state: tauri::State<History>, id: i64) -> Result<ConversationDetail, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    get(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_delete(state: tauri::State<History>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    delete(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_text_file(path: String, content: String) -> Result<(), String> {
    fs::write(path, content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init(&conn).unwrap();
        conn
    }

    #[test]
    fn add_new_and_append() {
        let conn = mem();
        let id = add(&conn, None, "cli", "haiku", "첫 질문\n둘째 줄", &["data:x".to_string()], "답1", &[]).unwrap();
        let id2 = add(&conn, Some(id), "cli", "haiku", "후속", &[], "답2", &[]).unwrap();
        assert_eq!(id, id2);
        let c = get(&conn, id).unwrap();
        assert_eq!(c.title, "첫 질문");
        assert_eq!(c.messages.len(), 2);
        assert_eq!(c.messages[0].images, vec!["data:x".to_string()]);
        assert_eq!(c.messages[1].question, "후속");
        let empty = add(&conn, None, "api", "haiku", "  ", &[], "a", &[]).unwrap();
        assert_eq!(get(&conn, empty).unwrap().title, "(이미지)");
    }

    #[test]
    fn model_round_trip() {
        let conn = mem();
        let id = add(&conn, None, "cli", "haiku", "q1", &[], "a1", &[]).unwrap();
        add(&conn, Some(id), "cli", "opus", "q2", &[], "a2", &[]).unwrap();
        let c = get(&conn, id).unwrap();
        assert_eq!(c.model, "haiku");
        assert_eq!(c.messages[0].model, "haiku");
        assert_eq!(c.messages[1].model, "opus");
    }

    #[test]
    fn migrates_old_schema() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE conversations(
                id INTEGER PRIMARY KEY, title TEXT NOT NULL, provider TEXT NOT NULL,
                model TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE messages(
                id INTEGER PRIMARY KEY,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                seq INTEGER NOT NULL, question TEXT NOT NULL, images TEXT NOT NULL,
                answer TEXT NOT NULL, created_at INTEGER NOT NULL);
             INSERT INTO conversations VALUES (1, 't', 'cli', 'haiku', 1, 1);
             INSERT INTO messages VALUES (1, 1, 1, 'old q', '[]', 'old a', 1);",
        )
        .unwrap();
        init(&conn).unwrap();
        init(&conn).unwrap();
        let c = get(&conn, 1).unwrap();
        assert_eq!(c.messages[0].model, "haiku");
        add(&conn, Some(1), "cli", "sonnet", "new q", &[], "new a", &[]).unwrap();
        assert_eq!(get(&conn, 1).unwrap().messages[1].model, "sonnet");
    }

    #[test]
    fn list_with_and_without_query() {
        let conn = mem();
        add(&conn, None, "cli", "haiku", "검색창 만들기", &[], "네", &[]).unwrap();
        add(&conn, None, "cli", "haiku", "날씨", &[], "맑음", &[]).unwrap();
        assert_eq!(list(&conn, "", 100).unwrap().len(), 2);
        let r = list(&conn, "검색", 100).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "검색창 만들기");
        let r = list(&conn, "맑", 100).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "날씨");
    }

    #[test]
    fn like_escaping() {
        let conn = mem();
        add(&conn, None, "cli", "haiku", "100% 확실", &[], "a", &[]).unwrap();
        add(&conn, None, "cli", "haiku", "abc", &[], "b", &[]).unwrap();
        assert_eq!(list(&conn, "%", 100).unwrap().len(), 1);
        assert_eq!(list(&conn, "_", 100).unwrap().len(), 0);
    }

    #[test]
    fn delete_cascades() {
        let conn = mem();
        let id = add(&conn, None, "cli", "haiku", "q", &[], "a", &[]).unwrap();
        delete(&conn, id).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert!(list(&conn, "", 100).unwrap().is_empty());
    }

    #[test]
    fn sources_round_trip() {
        let conn = mem();
        let src = vec![Source { url: "https://a.com/x".into(), title: "A".into() }];
        let id = add(&conn, None, "api", "haiku", "q1", &[], "a1", &src).unwrap();
        add(&conn, Some(id), "api", "haiku", "q2", &[], "a2", &[]).unwrap();
        let c = get(&conn, id).unwrap();
        assert_eq!(c.messages[0].sources, src);
        assert!(c.messages[1].sources.is_empty());
    }

    #[test]
    fn migrates_sources_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE conversations(
                id INTEGER PRIMARY KEY, title TEXT NOT NULL, provider TEXT NOT NULL,
                model TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE messages(
                id INTEGER PRIMARY KEY,
                conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                seq INTEGER NOT NULL, question TEXT NOT NULL, images TEXT NOT NULL,
                answer TEXT NOT NULL, created_at INTEGER NOT NULL,
                model TEXT NOT NULL DEFAULT 'haiku');
             INSERT INTO conversations VALUES (1, 't', 'api', 'haiku', 1, 1);
             INSERT INTO messages VALUES (1, 1, 1, 'q', '[]', 'a', 1, 'haiku');",
        )
        .unwrap();
        init(&conn).unwrap();
        init(&conn).unwrap();
        assert!(get(&conn, 1).unwrap().messages[0].sources.is_empty());
        let src = vec![Source { url: "https://b.com".into(), title: "B".into() }];
        add(&conn, Some(1), "api", "haiku", "q2", &[], "a2", &src).unwrap();
        assert_eq!(get(&conn, 1).unwrap().messages[1].sources, src);
    }
}
