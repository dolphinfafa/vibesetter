use crate::model::*;
use rusqlite::{params, Connection};
use std::{path::Path, sync::Mutex};

pub struct Store(Mutex<Connection>);

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path).map_err(|_| "无法打开本地数据库")?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS assistants(id TEXT PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, assistant_id TEXT NOT NULL REFERENCES assistants(id) ON DELETE CASCADE, started_at INTEGER NOT NULL, ended_at INTEGER);
            CREATE INDEX IF NOT EXISTS sessions_assistant ON sessions(assistant_id, started_at DESC);
            CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, role TEXT NOT NULL, source TEXT NOT NULL, text TEXT NOT NULL, created_at INTEGER NOT NULL, interrupted INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS messages_session ON messages(session_id, created_at);
            CREATE TABLE IF NOT EXISTS routes(assistant_id TEXT PRIMARY KEY REFERENCES assistants(id) ON DELETE CASCADE, data TEXT NOT NULL);")
            .map_err(|_| "初始化本地数据库失败")?;
        db.execute(
            "UPDATE sessions SET ended_at=?1 WHERE ended_at IS NULL",
            [now()],
        )
        .map_err(|_| "恢复会话状态失败")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "无法限制数据库访问权限")?;
        }
        Ok(Self(Mutex::new(db)))
    }
    fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.0
            .lock()
            .map_err(|_| "数据库正在恢复，请重启应用".into())
    }
    pub fn assistants(&self) -> Result<Vec<Assistant>> {
        let db = self.db()?;
        let mut stmt = db
            .prepare("SELECT data FROM assistants ORDER BY json_extract(data,'$.updated_at') DESC")
            .map_err(|_| "读取助手失败")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|_| "读取助手失败")?;
        rows.map(|r| {
            serde_json::from_str(&r.map_err(|_| "读取助手失败")?).map_err(|_| "助手数据损坏".into())
        })
        .collect()
    }
    pub fn assistant(&self, id: &str) -> Result<Assistant> {
        self.assistants()?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or("助手不存在".into())
    }
    pub fn save_assistant(&self, mut assistant: Assistant) -> Result<Assistant> {
        assistant.validate()?;
        assistant.name = assistant.name.trim().to_owned();
        assistant.updated_at = now();
        if assistant.id.is_empty() {
            assistant.id = uuid::Uuid::new_v4().to_string();
            assistant.created_at = assistant.updated_at;
        } else {
            assistant.created_at = self.assistant(&assistant.id)?.created_at;
        }
        self.db()?.execute("INSERT INTO assistants(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data", params![assistant.id, serde_json::to_string(&assistant).map_err(|_| "助手保存失败")?]).map_err(|_| "助手保存失败")?;
        Ok(assistant)
    }
    pub fn delete_assistant(&self, id: &str) -> Result<()> {
        self.db()?
            .execute("DELETE FROM assistants WHERE id=?1", [id])
            .map_err(|_| "删除助手失败")?;
        Ok(())
    }
    pub fn save_route(&self, id: &str, route: &SessionConfig) -> Result<()> {
        self.db()?.execute("INSERT INTO routes VALUES(?1,?2) ON CONFLICT(assistant_id) DO UPDATE SET data=excluded.data", params![id, serde_json::to_string(route).map_err(|_| "路由保存失败")?]).map_err(|_| "路由保存失败")?;
        Ok(())
    }
    pub fn route(&self, id: &str) -> Result<SessionConfig> {
        use rusqlite::OptionalExtension;
        let json: Option<String> = self
            .db()?
            .query_row("SELECT data FROM routes WHERE assistant_id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|_| "路由读取失败")?;
        match json {
            Some(s) => serde_json::from_str(&s).map_err(|_| "路由配置损坏".into()),
            None => Ok(SessionConfig {
                text_input: true,
                text_output: true,
                ..Default::default()
            }),
        }
    }
    pub fn create_session(&self, assistant_id: &str) -> Result<Session> {
        let s = Session {
            id: uuid::Uuid::new_v4().to_string(),
            assistant_id: assistant_id.into(),
            started_at: now(),
            ended_at: None,
        };
        self.db()?
            .execute(
                "INSERT INTO sessions VALUES(?1,?2,?3,NULL)",
                params![s.id, s.assistant_id, s.started_at],
            )
            .map_err(|_| "创建会话失败")?;
        Ok(s)
    }
    pub fn end_session(&self, id: &str) -> Result<()> {
        self.db()?
            .execute(
                "UPDATE sessions SET ended_at=?1 WHERE id=?2",
                params![now(), id],
            )
            .map_err(|_| "结束会话失败")?;
        Ok(())
    }
    pub fn sessions(&self, assistant_id: &str) -> Result<Vec<Session>> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT id,assistant_id,started_at,ended_at FROM sessions WHERE assistant_id=?1 ORDER BY started_at DESC LIMIT 100").map_err(|_| "读取历史失败")?;
        let rows = q
            .query_map([assistant_id], |r| {
                Ok(Session {
                    id: r.get(0)?,
                    assistant_id: r.get(1)?,
                    started_at: r.get(2)?,
                    ended_at: r.get(3)?,
                })
            })
            .map_err(|_| "读取历史失败")?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| "读取历史失败".into())
    }
    pub fn add_message(&self, m: &Message) -> Result<()> {
        self.db()?
            .execute(
                "INSERT INTO messages VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    m.id,
                    m.session_id,
                    m.role,
                    m.source,
                    m.text,
                    m.created_at,
                    m.interrupted
                ],
            )
            .map_err(|_| "保存消息失败")?;
        Ok(())
    }
    pub fn interrupt_message(&self, id: &str) -> Result<()> {
        self.db()?
            .execute("UPDATE messages SET interrupted=1 WHERE id=?1", [id])
            .map_err(|_| "更新消息失败")?;
        Ok(())
    }
    pub fn messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let db = self.db()?;
        let mut q = db.prepare("SELECT id,session_id,role,source,text,created_at,interrupted FROM messages WHERE session_id=?1 ORDER BY created_at,rowid").map_err(|_| "读取消息失败")?;
        let rows = q
            .query_map([session_id], |r| {
                Ok(Message {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    role: r.get(2)?,
                    source: r.get(3)?,
                    text: r.get(4)?,
                    created_at: r.get(5)?,
                    interrupted: r.get(6)?,
                })
            })
            .map_err(|_| "读取消息失败")?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| "读取消息失败".into())
    }
    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.db()?
            .execute("DELETE FROM sessions WHERE id=?1", [id])
            .map_err(|_| "删除会话失败")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_roundtrip_and_cascade_delete() {
        let d = tempfile::tempdir().unwrap();
        let db = Store::open(&d.path().join("app.db")).unwrap();
        let a = db
            .save_assistant(Assistant {
                id: "".into(),
                name: "搭档🎙️".into(),
                prompt: "请用中文回应".into(),
                avatar: None,
                color: "violet".into(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        assert_eq!(db.assistant(&a.id).unwrap().name, "搭档🎙️");
        let s = db.create_session(&a.id).unwrap();
        db.add_message(&Message {
            id: "m".into(),
            session_id: s.id.clone(),
            role: "user".into(),
            source: "麦克风".into(),
            text: "你好世界".into(),
            created_at: now(),
            interrupted: false,
        })
        .unwrap();
        assert_eq!(db.messages(&s.id).unwrap()[0].text, "你好世界");
        db.delete_assistant(&a.id).unwrap();
        assert!(db.messages(&s.id).unwrap().is_empty());
        assert!(db.sessions(&a.id).unwrap().is_empty());
    }
}
