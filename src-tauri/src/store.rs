use std::{fs, path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{recorder, speech_api, Result};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Speaker {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub speaker_id: String,
    #[serde(default)]
    pub overlap: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub duration: f64,
    pub source: String,
    #[serde(skip_serializing)]
    pub audio_path: String,
    pub recording_status: String,
    pub recording_error: Option<String>,
    pub transcription_status: String,
    pub transcription_stage: String,
    pub transcription_progress: Option<u8>,
    pub transcription_error: Option<String>,
    pub language: String,
    pub speakers: Vec<Speaker>,
    pub segments: Vec<Segment>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub audio_directory: String,
    pub language: String,
    pub method: String,
    pub api_url: String,
    pub api_model: String,
    pub has_api_key: bool,
    pub replacement_rules: Vec<ReplacementRule>,
}

#[derive(Deserialize, Serialize)]
pub struct ReplacementRule {
    pub find: String,
    pub replacement: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProfile {
    pub id: String,
    pub name: String,
    pub url: String,
    pub model: String,
    pub has_key: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiProfiles {
    profiles: Vec<ApiProfile>,
    active_id: String,
    method: String,
}

pub struct Store(Mutex<Connection>);

impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error)
    }
}

struct DatabaseError(rusqlite::Error);

impl From<DatabaseError> for String {
    fn from(error: DatabaseError) -> Self {
        format!("本地记录保存或读取失败：{}", error.0)
    }
}

const PROJECT_COLUMNS: &str = "id, name, created_at, duration, source, audio_path, recording_status,
    recording_error, transcription_status, transcription_stage, transcription_error, language, speakers, segments, transcription_progress";

fn read_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
        duration: row.get(3)?,
        source: row.get(4)?,
        audio_path: row.get(5)?,
        recording_status: row.get(6)?,
        recording_error: row.get(7)?,
        transcription_status: row.get(8)?,
        transcription_stage: row.get(9)?,
        transcription_error: row.get(10)?,
        language: row.get(11)?,
        speakers: json_column(row, 12)?,
        segments: json_column(row, 13)?,
        transcription_progress: row.get(14)?,
    })
}

fn json_column<T: serde::de::DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let content: String = row.get(index)?;
    serde_json::from_str(&content).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn preference(connection: &Connection, key: &str) -> Result<String> {
    Ok(connection
        .query_row(
            "SELECT value FROM preferences WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .map_err(DatabaseError)?)
}

impl Store {
    pub fn open(directory: &Path, audio_directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory).map_err(|e| format!("无法创建应用数据目录：{e}"))?;
        let connection =
            Connection::open(directory.join("voicetag.sqlite3")).map_err(DatabaseError)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS api_profiles (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, url TEXT NOT NULL, model TEXT NOT NULL,
                api_key BLOB NOT NULL DEFAULT X''
             );
             CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, audio_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                duration REAL NOT NULL, source TEXT NOT NULL, recording_status TEXT NOT NULL,
                recording_error TEXT, transcription_status TEXT NOT NULL DEFAULT 'pending',
                transcription_stage TEXT NOT NULL DEFAULT '', transcription_error TEXT, transcription_progress INTEGER,
                language TEXT NOT NULL DEFAULT 'auto', speakers TEXT NOT NULL DEFAULT '[]',
                segments TEXT NOT NULL DEFAULT '[]', original_segments TEXT NOT NULL DEFAULT '[]'
             );
             CREATE TABLE IF NOT EXISTS exports (
                id INTEGER PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                format TEXT NOT NULL, path TEXT NOT NULL, status TEXT NOT NULL, error TEXT,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             );"
        ).map_err(DatabaseError)?;
        let has_progress = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('projects') WHERE name = 'transcription_progress')",
            [], |row| row.get::<_, bool>(0),
        ).map_err(DatabaseError)?;
        if !has_progress {
            connection
                .execute(
                    "ALTER TABLE projects ADD COLUMN transcription_progress INTEGER",
                    [],
                )
                .map_err(DatabaseError)?;
        }
        connection
            .execute(
                "UPDATE projects SET transcription_status = 'failed', transcription_stage = '',
             transcription_error = '上次转写被中断，可以重新开始。', transcription_progress = NULL
             WHERE transcription_status = 'processing'",
                [],
            )
            .map_err(DatabaseError)?;
        connection
            .execute(
                "INSERT OR IGNORE INTO preferences (key, value) VALUES
                ('audio_directory', ?1), ('language', 'auto'), ('method', 'local'),
                ('replacement_rules', '[]')",
                [audio_directory.to_string_lossy().as_ref()],
            )
            .map_err(DatabaseError)?;

        let store = Self(Mutex::new(connection));
        let mut statement_guard = store.0.lock().unwrap();
        let transaction = statement_guard.transaction().map_err(DatabaseError)?;
        if transaction
            .execute(
                "INSERT OR IGNORE INTO preferences (key, value) VALUES ('active_api_profile', '')",
                [],
            )
            .map_err(DatabaseError)?
            > 0
        {
            transaction.execute_batch(
                "INSERT INTO api_profiles (id, name, url, model, api_key)
                    SELECT 'legacy', model.value, url.value, model.value, key.value
                    FROM preferences url, preferences model, preferences key
                    WHERE url.key = 'api_url' AND url.value <> '' AND model.key = 'api_model' AND key.key = 'api_key';
                 UPDATE preferences SET value = 'legacy' WHERE key = 'active_api_profile'
                    AND EXISTS (SELECT 1 FROM api_profiles WHERE id = 'legacy');
                 DELETE FROM preferences WHERE key IN ('api_url', 'api_model', 'api_key');"
            ).map_err(DatabaseError)?;
        }
        let interrupted = {
            let mut statement = transaction
                .prepare("SELECT id, audio_path FROM projects WHERE recording_status = 'recording'")
                .map_err(DatabaseError)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(DatabaseError)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DatabaseError)?
        };
        for (id, path) in interrupted {
            let (duration, warning, status) = match recorder::recover(Path::new(&path)) {
                Ok(duration) => (
                    duration,
                    "录音异常中断，已恢复写入磁盘的音频。".to_owned(),
                    "saved",
                ),
                Err(error) => (0.0, format!("上次录音异常中断：{error}"), "failed"),
            };
            transaction.execute(
                "UPDATE projects SET duration = ?2, recording_status = ?3, recording_error = ?4 WHERE id = ?1",
                params![id, duration, status, warning],
            ).map_err(DatabaseError)?;
        }
        transaction.commit().map_err(DatabaseError)?;
        drop(statement_guard);
        Ok(store)
    }

    pub fn preferences(&self) -> Result<Preferences> {
        Ok(self.transcription_preferences()?.0)
    }

    pub fn transcription_preferences(&self) -> Result<(Preferences, Vec<u8>)> {
        let connection = self.0.lock().unwrap();
        let (api_url, api_model, key): (String, String, Vec<u8>) = connection.query_row(
            "SELECT url, model, api_key FROM api_profiles WHERE id = (SELECT value FROM preferences WHERE key = 'active_api_profile')",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(DatabaseError)?.unwrap_or_default();
        Ok((
            Preferences {
                audio_directory: preference(&connection, "audio_directory")?,
                language: preference(&connection, "language")?,
                method: preference(&connection, "method")?,
                api_url,
                api_model,
                has_api_key: !key.is_empty(),
                replacement_rules: serde_json::from_str(&preference(
                    &connection,
                    "replacement_rules",
                )?)
                .map_err(|error| format!("无法读取替换规则：{error}"))?,
            },
            key,
        ))
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        let mut connection = self.0.lock().unwrap();
        let tx = connection.transaction().map_err(DatabaseError)?;
        if preferences.method == "api" {
            let ready: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM api_profiles WHERE id = (SELECT value FROM preferences WHERE key = 'active_api_profile') AND length(api_key) > 0)",
                [], |row| row.get(0),
            ).map_err(DatabaseError)?;
            if !ready {
                return Err("请在 AI 配置中选用一套已填写 API Key 的模型。".into());
            }
        }
        let rules =
            serde_json::to_string(&preferences.replacement_rules).map_err(|err| err.to_string())?;
        for (key, value) in [
            ("audio_directory", preferences.audio_directory.trim()),
            ("language", &preferences.language),
            ("method", &preferences.method),
            ("replacement_rules", &rules),
        ] {
            tx.execute(
                "UPDATE preferences SET value = ?2 WHERE key = ?1",
                params![key, value],
            )
            .map_err(DatabaseError)?;
        }
        tx.commit().map_err(DatabaseError)?;
        Ok(())
    }

    pub fn api_profiles(&self) -> Result<ApiProfiles> {
        let connection = self.0.lock().unwrap();
        let mut statement = connection
            .prepare(
                "SELECT id, name, url, model, length(api_key) > 0 FROM api_profiles ORDER BY rowid",
            )
            .map_err(DatabaseError)?;
        let rows = statement
            .query_map([], |row| {
                Ok(ApiProfile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    model: row.get(3)?,
                    has_key: row.get(4)?,
                })
            })
            .map_err(DatabaseError)?;
        Ok(ApiProfiles {
            profiles: rows
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(DatabaseError)?,
            active_id: preference(&connection, "active_api_profile")?,
            method: preference(&connection, "method")?,
        })
    }

    pub fn save_api_profile(&self, profile: &ApiProfile, key: Option<&[u8]>) -> Result<()> {
        let name = profile.name.trim();
        let url = profile.url.trim();
        let model = profile.model.trim();
        if name.is_empty() || model.is_empty() {
            return Err("请填写配置名称和模型 ID。".into());
        }
        let endpoint = speech_api::endpoint(url, model)?;
        let mut connection = self.0.lock().unwrap();
        let tx = connection.transaction().map_err(DatabaseError)?;
        if profile.id.is_empty() {
            tx.execute("INSERT INTO api_profiles (id, name, url, model, api_key) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![Uuid::new_v4().to_string(), name, url, model, key.unwrap_or_default()]).map_err(DatabaseError)?;
        } else {
            let (previous_url, previous_model, has_key): (String, String, bool) = tx
                .query_row(
                    "SELECT url, model, length(api_key) > 0 FROM api_profiles WHERE id = ?1",
                    [&profile.id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(DatabaseError)?
                .ok_or("此模型配置已不存在。")?;
            if key.is_none()
                && has_key
                && speech_api::endpoint(&previous_url, &previous_model)?.origin()
                    != endpoint.origin()
            {
                return Err("服务地址已更换，请填写新的 API Key，或清除旧 Key。".into());
            }
            tx.execute("UPDATE api_profiles SET name = ?2, url = ?3, model = ?4, api_key = COALESCE(?5, api_key) WHERE id = ?1",
                params![profile.id, name, url, model, key]).map_err(DatabaseError)?;
            if key.is_some_and(|key| key.is_empty()) {
                tx.execute(
                    "UPDATE preferences SET value = 'local' WHERE key = 'method' AND value = 'api'
                    AND (SELECT value FROM preferences WHERE key = 'active_api_profile') = ?1",
                    [&profile.id],
                )
                .map_err(DatabaseError)?;
            }
        }
        tx.commit().map_err(DatabaseError)?;
        Ok(())
    }

    pub fn use_api_profile(&self, id: &str) -> Result<()> {
        let mut connection = self.0.lock().unwrap();
        let tx = connection.transaction().map_err(DatabaseError)?;
        let ready: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM api_profiles WHERE id = ?1 AND length(api_key) > 0)",
                [id],
                |row| row.get(0),
            )
            .map_err(DatabaseError)?;
        if !ready {
            return Err("请先为此模型配置 API Key。".into());
        }
        tx.execute(
            "UPDATE preferences SET value = ?1 WHERE key = 'active_api_profile'",
            [id],
        )
        .map_err(DatabaseError)?;
        tx.execute(
            "UPDATE preferences SET value = 'api' WHERE key = 'method'",
            [],
        )
        .map_err(DatabaseError)?;
        tx.commit().map_err(DatabaseError)?;
        Ok(())
    }

    pub fn delete_api_profile(&self, id: &str) -> Result<()> {
        let mut connection = self.0.lock().unwrap();
        let tx = connection.transaction().map_err(DatabaseError)?;
        tx.execute("DELETE FROM api_profiles WHERE id = ?1", [id])
            .map_err(DatabaseError)?;
        let active = tx
            .execute(
                "UPDATE preferences SET value = '' WHERE key = 'active_api_profile' AND value = ?1",
                [id],
            )
            .map_err(DatabaseError)?;
        if active > 0 {
            tx.execute(
                "UPDATE preferences SET value = 'local' WHERE key = 'method' AND value = 'api'",
                [],
            )
            .map_err(DatabaseError)?;
        }
        tx.commit().map_err(DatabaseError)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Project>> {
        let connection = self.0.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, name, created_at, duration, source, audio_path, recording_status, recording_error,
                    transcription_status, transcription_stage, transcription_error, language, '[]', '[]', transcription_progress
             FROM projects WHERE recording_status != 'recording' ORDER BY created_at DESC, id"
        ).map_err(DatabaseError)?;
        let rows = statement
            .query_map([], read_project)
            .map_err(DatabaseError)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(DatabaseError)?)
    }

    pub fn get(&self, id: &str) -> Result<Project> {
        self.0
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
                [id],
                read_project,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => "找不到这条录音记录。".into(),
                other => DatabaseError(other).into(),
            })
    }

    pub fn create(
        &self,
        name: &str,
        path: &Path,
        duration: f64,
        source: &str,
        status: &str,
    ) -> Result<Project> {
        let id = Uuid::new_v4().to_string();
        self.0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, audio_path, duration, source, recording_status, language)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT value FROM preferences WHERE key = 'language'))",
                params![id, name, path.to_string_lossy(), duration, source, status],
            )
            .map_err(DatabaseError)?;
        self.get(&id)
    }

    pub fn end_recording(&self, id: &str, duration: f64, error: Option<&str>) -> Result<()> {
        let status = if duration > 0.0 { "saved" } else { "failed" };
        let error = error.or(if duration <= 0.0 {
            Some("未录到音频，请检查麦克风后重试。")
        } else {
            None
        });
        self.0.lock().unwrap().execute(
            "UPDATE projects SET duration = ?2, recording_status = ?3, recording_error = ?4 WHERE id = ?1",
            params![id, duration, status, error],
        ).map_err(DatabaseError)?;
        Ok(())
    }

    pub fn update_duration(&self, id: &str, duration: f64) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute(
                "UPDATE projects SET duration = ?2 WHERE id = ?1",
                params![id, duration],
            )
            .map_err(DatabaseError)?;
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute(
                "UPDATE projects SET name = ?2 WHERE id = ?1",
                params![id, name],
            )
            .map_err(DatabaseError)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM projects WHERE id = ?1", [id])
            .map_err(DatabaseError)?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str, delete_audio: bool) -> Result<()> {
        let mut connection = self.0.lock().unwrap();
        let tx = connection.transaction().map_err(DatabaseError)?;
        let project = tx
            .query_row(
                &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
                [id],
                read_project,
            )
            .map_err(DatabaseError)?;
        if project.recording_status == "recording" || project.transcription_status == "processing" {
            return Err("请等待当前录音或转写结束。".into());
        }
        tx.execute("DELETE FROM projects WHERE id = ?1", [id])
            .map_err(DatabaseError)?;
        if delete_audio {
            match fs::remove_file(&project.audio_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("无法删除原始音频：{error}")),
            }
        }
        tx.commit().map_err(DatabaseError)?;
        Ok(())
    }

    pub fn has_transcription_in_progress(&self) -> Result<bool> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE transcription_status = 'processing')",
                [],
                |row| row.get(0),
            )
            .map_err(DatabaseError)?)
    }

    pub fn begin_transcription(&self, id: &str, language: &str) -> Result<Project> {
        let changed = self.0.lock().unwrap().execute(
            "UPDATE projects SET transcription_status = 'processing', transcription_stage = '正在准备转写',
                transcription_error = NULL, transcription_progress = NULL, language = ?2
             WHERE id = ?1 AND recording_status = 'saved' AND transcription_status IN ('pending', 'failed')",
            params![id, language],
        ).map_err(DatabaseError)?;
        if changed == 0 {
            return Err("当前录音无法开始转写。".into());
        }
        self.get(id)
    }

    pub fn begin_manual_transcript(&self, id: &str) -> Result<Project> {
        let changed = self.0.lock().unwrap().execute(
            "UPDATE projects SET transcription_status = 'complete', transcription_stage = '', transcription_error = NULL
             WHERE id = ?1 AND recording_status = 'saved' AND transcription_status IN ('pending', 'failed')",
            [id],
        ).map_err(DatabaseError)?;
        if changed == 0 {
            return Err("当前录音无法开始手动听写。".into());
        }
        self.get(id)
    }

    pub fn set_transcription_stage(&self, id: &str, stage: &str) -> Result<()> {
        self.set_transcription_progress(id, stage, None)
    }

    pub fn set_transcription_progress(
        &self,
        id: &str,
        stage: &str,
        progress: Option<u8>,
    ) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute(
                "UPDATE projects SET transcription_stage = ?2, transcription_progress = ?3
                 WHERE id = ?1 AND transcription_status = 'processing'",
                params![id, stage, progress],
            )
            .map_err(DatabaseError)?;
        Ok(())
    }

    pub fn finish_transcription(
        &self,
        id: &str,
        speakers: &[Speaker],
        segments: &[Segment],
        rules: &[ReplacementRule],
        polish: Option<Result<Vec<String>>>,
    ) -> Result<()> {
        let speakers = serde_json::to_string(speakers).map_err(|e| e.to_string())?;
        let original = serde_json::to_string(segments).map_err(|e| e.to_string())?;
        let mut corrected = segments.to_vec();
        let warning = match polish {
            Some(Ok(texts)) => {
                for (segment, text) in corrected.iter_mut().zip(texts) {
                    segment.text = text;
                }
                None
            }
            Some(Err(error)) => Some(format!("在线校对未完成，已保存本机转写。{error}")),
            None => None,
        };
        for segment in &mut corrected {
            for rule in rules {
                segment.text = segment.text.replace(&rule.find, &rule.replacement);
            }
        }
        let corrected = serde_json::to_string(&corrected).map_err(|e| e.to_string())?;
        self.0.lock().unwrap().execute(
            "UPDATE projects SET transcription_status = 'complete', transcription_stage = '', speakers = ?2,
                segments = ?3, original_segments = ?4, transcription_error = ?5, transcription_progress = 100
                WHERE id = ?1 AND transcription_status = 'processing'",
            params![id, speakers, corrected, original, warning],
        ).map_err(DatabaseError)?;
        Ok(())
    }

    pub fn fail_transcription(&self, id: &str, error: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .execute(
                "UPDATE projects SET transcription_status = 'failed', transcription_stage = '',
                transcription_error = ?2, transcription_progress = NULL WHERE id = ?1 AND transcription_status = 'processing'",
                params![id, error],
            )
            .map_err(DatabaseError)?;
        Ok(())
    }

    pub fn save_transcript(
        &self,
        id: &str,
        speakers: &[Speaker],
        segments: &[Segment],
    ) -> Result<()> {
        let speakers = serde_json::to_string(speakers).map_err(|e| e.to_string())?;
        let segments = serde_json::to_string(segments).map_err(|e| e.to_string())?;
        self.0.lock().unwrap().execute(
            "UPDATE projects SET speakers = ?2, segments = ?3 WHERE id = ?1 AND transcription_status = 'complete'",
            params![id, speakers, segments],
        ).map_err(DatabaseError)?;
        Ok(())
    }

    pub fn log_export(
        &self,
        id: &str,
        format: &str,
        path: &Path,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO exports (project_id, format, path, status, error) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, format, path.to_string_lossy(), status, error],
        ).map_err(DatabaseError)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_switches_profiles_without_crossing_keys() {
        struct TestDirectory(std::path::PathBuf);
        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = fs::remove_file(self.0.join("voicetag.sqlite3"));
                let _ = fs::remove_dir(&self.0);
            }
        }
        let directory = TestDirectory(
            std::env::temp_dir().join(format!("voicetag-profiles-{}", Uuid::new_v4())),
        );
        fs::create_dir(&directory.0).unwrap();
        let legacy = Connection::open(directory.0.join("voicetag.sqlite3")).unwrap();
        let old_key = speech_api::protect_key("legacy-test-key").unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO preferences VALUES ('api_url', 'https://old.example/v1'),
                 ('api_model', 'whisper-1'), ('method', 'api');",
            )
            .unwrap();
        legacy
            .execute("INSERT INTO preferences VALUES ('api_key', ?1)", [&old_key])
            .unwrap();
        drop(legacy);

        let store = Store::open(&directory.0, &directory.0).unwrap();
        let (old, migrated_key) = store.transcription_preferences().unwrap();
        assert_eq!(old.method, "api");
        assert_eq!(old.api_url, "https://old.example/v1");
        assert_eq!(migrated_key, old_key);
        let mut second = ApiProfile {
            id: String::new(),
            name: "第二个模型".into(),
            url: "https://new.example/api/v1".into(),
            model: "paraformer-v2".into(),
            has_key: false,
        };
        let second_key = speech_api::protect_key("new-test-key").unwrap();
        store.save_api_profile(&second, Some(&second_key)).unwrap();
        let list = store.api_profiles().unwrap();
        assert_eq!(list.profiles.len(), 2);
        assert_eq!(list.active_id, "legacy");
        second.id = list.profiles[1].id.clone();
        store.use_api_profile(&second.id).unwrap();
        let (active, key) = store.transcription_preferences().unwrap();
        assert_eq!(active.api_model, "paraformer-v2");
        assert_eq!(key, second_key);
        second.url = "https://different.example/v1".into();
        assert!(store.save_api_profile(&second, None).is_err());
        drop(store);

        let reopened = Store::open(&directory.0, &directory.0).unwrap();
        assert_eq!(reopened.api_profiles().unwrap().active_id, second.id);
        reopened.delete_api_profile(&second.id).unwrap();
        assert_eq!(reopened.preferences().unwrap().method, "local");
        assert!(reopened.use_api_profile(&second.id).is_err());
        reopened.use_api_profile("legacy").unwrap();
        assert_eq!(reopened.transcription_preferences().unwrap().1, old_key);
    }
}
