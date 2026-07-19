//! MongoDB-backed store — sole source of truth for Maxcos.
//! Password hashes use Argon2id. Files live in Mongo; disk under data/cache is Terminal-only.

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::Local;
use mongodb::{
    bson::{doc, Bson, DateTime as BsonDateTime, Document},
    options::{ClientOptions, IndexOptions},
    Client, Collection, Database, IndexModel,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;
use crate::apps;

// ─── Models ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub color: String,
    /// Argon2id PHC string — never expose to clients
    #[serde(default)]
    pub password_hash: String,
    #[serde(default)]
    pub password_hint: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    pub updated: String,
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub body: String,
    pub app: String,
    pub app_id: String,
    pub time: String,
    pub read: bool,
    pub created_at: i64,
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub modified: String,
    pub ext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileDoc {
    #[serde(default)]
    pub user_id: String,
    pub path: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub ext: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub encoding: String,
    #[serde(default)]
    pub modified: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub path: Option<String>,
    pub app_id: Option<String>,
    pub score: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafariTab {
    pub id: String,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafariBookmark {
    pub id: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafariHistoryEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub visited_at: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SafariState {
    pub tabs: Vec<SafariTab>,
    pub bookmarks: Vec<SafariBookmark>,
    pub history: Vec<SafariHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceInfo {
    pub id: String,
    pub name: String,
    pub wallpaper: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub wallpaper: String,
    pub spaces: Vec<SpaceInfo>,
    pub active_space: usize,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            wallpaper: "sonoma".into(),
            spaces: vec![
                SpaceInfo {
                    id: "space-1".into(),
                    name: "Desktop 1".into(),
                    wallpaper: "sonoma".into(),
                },
                SpaceInfo {
                    id: "space-2".into(),
                    name: "Desktop 2".into(),
                    wallpaper: "sequoia".into(),
                },
            ],
            active_space: 0,
        }
    }
}

/// Security / admin audit trail (Mongo `audit_log` collection).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    /// login_success | login_fail | user_create | user_delete | settings_change
    pub action: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub detail: String,
    pub ts: i64,
    pub time: String,
}

// ─── Store ──────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Store {
    db: Database,
    /// Local cache root for Terminal sandbox (materialized from Mongo)
    pub disk_root: PathBuf,
}

impl Store {
    pub const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 14;

    pub async fn connect(disk_base: PathBuf) -> mongodb::error::Result<Self> {
        let uri = std::env::var("MONGODB_URI")
            .unwrap_or_else(|_| "mongodb://127.0.0.1:27017".into());
        let db_name = std::env::var("MONGODB_DB").unwrap_or_else(|_| "maxcos".into());
        let mut opts = ClientOptions::parse(&uri).await?;
        opts.app_name = Some("maxcos".into());
        let client = Client::with_options(opts)?;
        let db = client.database(&db_name);
        // ping
        db.run_command(doc! { "ping": 1 }).await?;
        let disk_root = disk_base.join("data");
        fs::create_dir_all(disk_root.join("cache")).ok();
        let s = Self { db, disk_root };
        // ensure indexes best-effort
        let _ = s.ensure_indexes().await;
        Ok(s)
    }

    async fn ensure_indexes(&self) -> mongodb::error::Result<()> {
        // users.name — unique (source of truth for account names)
        let _ = self
            .col::<Document>("users")
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "name": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .name(Some("users_name_unique".to_string()))
                            .build(),
                    )
                    .build(),
            )
            .await;

        // sessions.user_id — lookup when destroying user sessions
        let _ = self
            .col::<Document>("sessions")
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1 })
                    .options(
                        IndexOptions::builder()
                            .name(Some("sessions_user_id".to_string()))
                            .build(),
                    )
                    .build(),
            )
            .await;

        // sessions.expires_at — TTL so expired sessions auto-delete (field must be BSON Date)
        let _ = self
            .col::<Document>("sessions")
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .name(Some("sessions_expires_at_ttl".to_string()))
                            .expire_after(Duration::from_secs(0))
                            .build(),
                    )
                    .build(),
            )
            .await;

        // files (user_id, path) — unique compound; drop legacy non-unique name if present
        let files = self.col::<Document>("files");
        let _ = files.drop_index("user_id_1_path_1").await;
        let _ = files
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "path": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .name(Some("files_user_path_unique".to_string()))
                            .build(),
                    )
                    .build(),
            )
            .await;

        // audit_log by time (newest first)
        let _ = self
            .col::<Document>("audit_log")
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "ts": -1 })
                    .options(
                        IndexOptions::builder()
                            .name(Some("audit_ts_desc".to_string()))
                            .build(),
                    )
                    .build(),
            )
            .await;

        Ok(())
    }

    fn col<T: Send + Sync>(&self, name: &str) -> Collection<T> {
        self.db.collection::<T>(name)
    }

    fn block_on<F, T>(&self, f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
    }

    // ── Password hashing ────────────────────────────────────────────────────

    pub fn hash_password(password: &str) -> Result<String, String> {
        let mut salt_bytes = [0u8; 16];
        // Simple entropy from uuid
        let u = Uuid::new_v4();
        salt_bytes.copy_from_slice(&u.as_bytes()[..16]);
        let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| e.to_string())?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| e.to_string())
    }

    pub fn verify_password(password: &str, password_hash: &str) -> bool {
        if password_hash.is_empty() || !password_hash.starts_with("$argon2") {
            return false;
        }
        match PasswordHash::new(password_hash) {
            Ok(parsed) => Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok(),
            Err(_) => false,
        }
    }

    // ── Audit log ───────────────────────────────────────────────────────────

    pub fn audit(
        &self,
        action: &str,
        user_id: &str,
        username: &str,
        detail: &str,
    ) {
        let entry = AuditEntry {
            id: format!("audit-{}", Uuid::new_v4()),
            action: action.into(),
            user_id: user_id.into(),
            username: username.into(),
            detail: detail.into(),
            ts: Local::now().timestamp(),
            time: now_str(),
        };
        self.block_on(async {
            if let Ok(mut doc) = mongodb::bson::to_document(&entry) {
                doc.insert("_id", &entry.id);
                let _ = self.col::<Document>("audit_log").insert_one(doc).await;
            }
        });
    }

    pub fn audit_list(&self, limit: i64) -> Vec<AuditEntry> {
        let limit = limit.clamp(1, 500);
        self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<AuditEntry>("audit_log")
                .find(doc! {})
                .sort(doc! { "ts": -1 })
                .limit(limit)
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(e)) = c.next().await {
                out.push(e);
            }
            out
        })
    }

    // ── Users ───────────────────────────────────────────────────────────────

    pub fn list_users(&self) -> Vec<UserAccount> {
        self.block_on(async {
            let mut cursor = match self.col::<UserAccount>("users").find(doc! {}).await {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            use futures_util::StreamExt;
            while let Some(doc) = cursor.next().await {
                if let Ok(u) = doc {
                    out.push(u);
                }
            }
            out
        })
    }

    pub fn user_count(&self) -> usize {
        self.block_on(async {
            self.col::<Document>("users")
                .count_documents(doc! {})
                .await
                .unwrap_or(0) as usize
        })
    }

    pub fn has_users(&self) -> bool {
        self.user_count() > 0
    }

    pub fn find_user(&self, id: &str) -> Option<UserAccount> {
        self.block_on(async {
            self.col::<UserAccount>("users")
                .find_one(doc! { "id": id })
                .await
                .ok()
                .flatten()
        })
    }

    pub fn find_user_by_name(&self, name: &str) -> Option<UserAccount> {
        let n = name.trim();
        self.list_users()
            .into_iter()
            .find(|u| u.name.eq_ignore_ascii_case(n))
    }

    pub fn create_user(
        &self,
        name: String,
        avatar: String,
        color: String,
        password: String,
        password_hint: String,
    ) -> io::Result<UserAccount> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "name is required"));
        }
        if self.find_user_by_name(&name).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "that name is already taken",
            ));
        }
        if let Err(msg) = crate::security::validate_password(&password) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
        }
        let password_hash = Self::hash_password(&password)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let avatar = if avatar.trim().is_empty() {
            name.chars().next().unwrap_or('U').to_uppercase().to_string()
        } else {
            avatar.trim().chars().take(2).collect()
        };
        let u = UserAccount {
            id: format!("user-{}", Uuid::new_v4()),
            name: name.clone(),
            avatar,
            color: if color.trim().is_empty() {
                "#0A84FF".into()
            } else {
                color
            },
            password_hash,
            password_hint,
            created_at: Local::now().timestamp(),
        };
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(&u)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("_id", &u.id);
            self.col::<Document>("users")
                .insert_one(doc)
                .await
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("E11000") || msg.contains("duplicate key") {
                        io::Error::new(io::ErrorKind::AlreadyExists, "that name is already taken")
                    } else {
                        io::Error::new(io::ErrorKind::Other, e)
                    }
                })?;
            Ok::<(), io::Error>(())
        })?;
        self.init_user_home(&u.id, &u.name)?;
        self.audit(
            "user_create",
            &u.id,
            &u.name,
            &format!("Created user {}", u.name),
        );
        Ok(u)
    }

    pub fn delete_user(&self, id: &str) -> io::Result<bool> {
        if self.user_count() <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "cannot delete the last user",
            ));
        }
        let victim = self.find_user(id);
        let deleted = self.block_on(async {
            let r = self
                .col::<Document>("users")
                .delete_one(doc! { "id": id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<u64, io::Error>(r.deleted_count)
        })?;
        if deleted > 0 {
            let _ = self.destroy_user_sessions(id);
            for coll in [
                "notes",
                "reminders",
                "notifications",
                "files",
                "safari",
                "settings",
            ] {
                let _ = self.block_on(async {
                    self.col::<Document>(coll)
                        .delete_many(doc! { "user_id": id })
                        .await
                });
            }
            let _ = fs::remove_dir_all(self.user_disk(id));
            let name = victim
                .as_ref()
                .map(|u| u.name.as_str())
                .unwrap_or("");
            self.audit(
                "user_delete",
                id,
                name,
                &format!("Deleted user {name} ({id})"),
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn init_user_home(&self, user_id: &str, display_name: &str) -> io::Result<()> {
        // Default settings
        if self.settings_get(user_id).spaces.is_empty()
            || self
                .block_on(async {
                    self.col::<Document>("settings")
                        .find_one(doc! { "user_id": user_id })
                        .await
                        .ok()
                        .flatten()
                })
                .is_none()
        {
            let mut s = UserSettings::default();
            let walls = ["sonoma", "sequoia", "midnight", "dawn"];
            let idx = user_id.bytes().map(|b| b as usize).sum::<usize>() % walls.len();
            s.wallpaper = walls[idx].into();
            if let Some(sp) = s.spaces.get_mut(0) {
                sp.wallpaper = walls[idx].into();
            }
            let _ = self.settings_save(user_id, &s);
        }
        // safari defaults
        if self
            .block_on(async {
                self.col::<Document>("safari")
                    .find_one(doc! { "user_id": user_id })
                    .await
                    .ok()
                    .flatten()
            })
            .is_none()
        {
            let st = SafariState {
                tabs: vec![SafariTab {
                    id: Uuid::new_v4().to_string(),
                    title: "Start Page".into(),
                    url: "about:start".into(),
                    active: true,
                }],
                bookmarks: vec![SafariBookmark {
                    id: Uuid::new_v4().to_string(),
                    title: "Apple".into(),
                    url: "https://www.apple.com".into(),
                }],
                history: vec![],
            };
            let _ = self.safari_save(user_id, &st);
        }
        // seed notes if empty
        if self.notes_list(user_id).is_empty() {
            let _ = self.note_create(
                user_id,
                format!("Hello, {display_name}"),
                "This note is private to your account.".into(),
            );
        }
        // seed dirs + welcome files in Mongo + disk
        for d in [
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Movies",
            "Applications",
            "Public",
        ] {
            let _ = self.fs_mkdir(user_id, &format!("~/{d}"));
        }
        let welcome = format!("Welcome, {display_name}!\n\nYour files are stored in MongoDB.\n");
        if self.fs_read(user_id, "~/Documents/Welcome.txt").is_err() {
            let _ = self.fs_write(user_id, "~/Documents/Welcome.txt", &welcome);
        }
        if self.fs_read(user_id, "~/Desktop/README.txt").is_err() {
            let _ = self.fs_write(
                user_id,
                "~/Desktop/README.txt",
                &format!("{display_name}'s Desktop\n"),
            );
        }
        // welcome notification
        if self.notifications_list(user_id).is_empty() {
            let _ = self.notification_push(
                user_id,
                "Welcome".into(),
                format!("Signed in as {display_name}"),
                "System".into(),
                "systemsettings".into(),
            );
        }
        // materialize disk cache for terminal
        self.materialize_user_disk(user_id);
        Ok(())
    }

    // ── Sessions ────────────────────────────────────────────────────────────

    fn session_doc(rec: &SessionRecord) -> Document {
        doc! {
            "_id": &rec.id,
            "id": &rec.id,
            "user_id": &rec.user_id,
            "username": &rec.username,
            "created_at": rec.created_at,
            "last_seen": rec.last_seen,
            // BSON Date required for TTL index sessions_expires_at_ttl
            "expires_at": BsonDateTime::from_millis(rec.expires_at.saturating_mul(1000)),
        }
    }

    fn session_from_doc(d: &Document) -> Option<SessionRecord> {
        let id = d.get_str("id").ok()?.to_string();
        let user_id = d.get_str("user_id").ok()?.to_string();
        let username = d.get_str("username").ok()?.to_string();
        let created_at = match d.get("created_at") {
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => *i as i64,
            _ => 0,
        };
        let last_seen = match d.get("last_seen") {
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => *i as i64,
            _ => 0,
        };
        let expires_at = match d.get("expires_at") {
            Some(Bson::DateTime(dt)) => dt.timestamp_millis() / 1000,
            Some(Bson::Int64(i)) => *i,
            Some(Bson::Int32(i)) => *i as i64,
            _ => return None,
        };
        Some(SessionRecord {
            id,
            user_id,
            username,
            created_at,
            last_seen,
            expires_at,
        })
    }

    pub fn create_session(&self, user_id: &str, username: &str) -> io::Result<SessionRecord> {
        let now = Local::now().timestamp();
        let rec = SessionRecord {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.into(),
            username: username.into(),
            created_at: now,
            last_seen: now,
            expires_at: now + Self::SESSION_TTL_SECS,
        };
        let doc = Self::session_doc(&rec);
        self.block_on(async {
            self.col::<Document>("sessions")
                .insert_one(doc)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(rec)
        })
    }

    pub fn touch_session(&self, session_id: &str) -> io::Result<Option<SessionRecord>> {
        let now = Local::now().timestamp();
        self.block_on(async {
            let col = self.col::<Document>("sessions");
            let Some(raw) = col
                .find_one(doc! { "id": session_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            else {
                return Ok(None);
            };
            let Some(mut s) = Self::session_from_doc(&raw) else {
                return Ok(None);
            };
            if s.expires_at <= now {
                let _ = col.delete_one(doc! { "id": session_id }).await;
                return Ok(None);
            }
            s.last_seen = now;
            s.expires_at = now + Self::SESSION_TTL_SECS;
            col.update_one(
                doc! { "id": session_id },
                doc! {
                    "$set": {
                        "last_seen": s.last_seen,
                        "expires_at": BsonDateTime::from_millis(s.expires_at.saturating_mul(1000)),
                    }
                },
            )
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(Some(s))
        })
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionRecord> {
        let now = Local::now().timestamp();
        self.block_on(async {
            let raw = self
                .col::<Document>("sessions")
                .find_one(doc! { "id": session_id })
                .await
                .ok()
                .flatten()?;
            let s = Self::session_from_doc(&raw)?;
            if s.expires_at <= now {
                None
            } else {
                Some(s)
            }
        })
    }

    pub fn destroy_session(&self, session_id: &str) -> io::Result<bool> {
        self.block_on(async {
            let r = self
                .col::<Document>("sessions")
                .delete_one(doc! { "id": session_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.deleted_count > 0)
        })
    }

    pub fn destroy_user_sessions(&self, user_id: &str) -> io::Result<usize> {
        self.block_on(async {
            let r = self
                .col::<Document>("sessions")
                .delete_many(doc! { "user_id": user_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.deleted_count as usize)
        })
    }

    pub fn session_info(s: &SessionRecord) -> SessionInfo {
        SessionInfo {
            id: s.id.clone(),
            user_id: s.user_id.clone(),
            username: s.username.clone(),
            created_at: s.created_at,
            last_seen: s.last_seen,
            expires_at: s.expires_at,
        }
    }

    // ── Settings ────────────────────────────────────────────────────────────

    pub fn settings_get(&self, user_id: &str) -> UserSettings {
        self.block_on(async {
            self.col::<UserSettings>("settings")
                .find_one(doc! { "user_id": user_id })
                .await
                .ok()
                .flatten()
                .unwrap_or_default()
        })
    }

    pub fn settings_save(&self, user_id: &str, s: &UserSettings) -> io::Result<()> {
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(s)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("user_id", user_id);
            doc.insert("_id", user_id);
            self.col::<Document>("settings")
                .replace_one(doc! { "user_id": user_id }, doc)
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })?;
        let username = self
            .find_user(user_id)
            .map(|u| u.name)
            .unwrap_or_default();
        self.audit(
            "settings_change",
            user_id,
            &username,
            &format!(
                "Settings saved (wallpaper={}, spaces={}, active={})",
                s.wallpaper,
                s.spaces.len(),
                s.active_space
            ),
        );
        Ok(())
    }

    // ── Notes ───────────────────────────────────────────────────────────────

    pub fn notes_list(&self, user_id: &str) -> Vec<Note> {
        self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<Note>("notes")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(n)) = c.next().await {
                out.push(n);
            }
            out
        })
    }

    pub fn note_create(&self, user_id: &str, title: String, body: String) -> io::Result<Note> {
        let note = Note {
            id: Uuid::new_v4().to_string(),
            title: if title.is_empty() {
                "Untitled".into()
            } else {
                title
            },
            body,
            updated: now_str(),
            user_id: user_id.into(),
        };
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(&note)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("_id", &note.id);
            self.col::<Document>("notes")
                .insert_one(doc)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })?;
        let path = format!("~/Documents/Notes/{}.md", sanitize(&note.title));
        let _ = self.fs_mkdir(user_id, "~/Documents/Notes");
        let _ = self.fs_write(
            user_id,
            &path,
            &format!("# {}\n\n{}\n", note.title, note.body),
        );
        Ok(note)
    }

    pub fn note_update(
        &self,
        user_id: &str,
        id: &str,
        title: String,
        body: String,
    ) -> io::Result<Option<Note>> {
        let Some(mut n) = self.notes_list(user_id).into_iter().find(|n| n.id == id) else {
            return Ok(None);
        };
        n.title = if title.is_empty() {
            "Untitled".into()
        } else {
            title
        };
        n.body = body;
        n.updated = now_str();
        self.block_on(async {
            self.col::<Document>("notes")
                .update_one(
                    doc! { "id": id, "user_id": user_id },
                    doc! { "$set": {
                        "title": &n.title,
                        "body": &n.body,
                        "updated": &n.updated,
                    }},
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })?;
        Ok(Some(n))
    }

    pub fn note_delete(&self, user_id: &str, id: &str) -> io::Result<bool> {
        self.block_on(async {
            let r = self
                .col::<Document>("notes")
                .delete_one(doc! { "id": id, "user_id": user_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.deleted_count > 0)
        })
    }

    // ── Reminders ───────────────────────────────────────────────────────────

    pub fn reminders_list(&self, user_id: &str) -> Vec<Reminder> {
        self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<Reminder>("reminders")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(n)) = c.next().await {
                out.push(n);
            }
            out
        })
    }

    pub fn reminder_create(&self, user_id: &str, text: String) -> io::Result<Reminder> {
        let r = Reminder {
            id: Uuid::new_v4().to_string(),
            text,
            done: false,
            user_id: user_id.into(),
        };
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(&r)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("_id", &r.id);
            self.col::<Document>("reminders")
                .insert_one(doc)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r)
        })
    }

    pub fn reminder_toggle(&self, user_id: &str, id: &str) -> io::Result<Option<Reminder>> {
        let Some(mut r) = self.reminders_list(user_id).into_iter().find(|r| r.id == id) else {
            return Ok(None);
        };
        r.done = !r.done;
        self.block_on(async {
            self.col::<Document>("reminders")
                .update_one(
                    doc! { "id": id, "user_id": user_id },
                    doc! { "$set": { "done": r.done } },
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(Some(r))
        })
    }

    pub fn reminder_delete(&self, user_id: &str, id: &str) -> io::Result<bool> {
        self.block_on(async {
            let r = self
                .col::<Document>("reminders")
                .delete_one(doc! { "id": id, "user_id": user_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.deleted_count > 0)
        })
    }

    // ── Notifications ───────────────────────────────────────────────────────

    pub fn notifications_list(&self, user_id: &str) -> Vec<Notification> {
        let mut list = self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<Notification>("notifications")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(n)) = c.next().await {
                out.push(n);
            }
            out
        });
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }

    pub fn notification_push(
        &self,
        user_id: &str,
        title: String,
        body: String,
        app: String,
        app_id: String,
    ) -> io::Result<Notification> {
        let n = Notification {
            id: Uuid::new_v4().to_string(),
            title,
            body,
            app,
            app_id,
            time: now_str(),
            read: false,
            created_at: Local::now().timestamp(),
            user_id: user_id.into(),
        };
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(&n)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("_id", &n.id);
            self.col::<Document>("notifications")
                .insert_one(doc)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(n)
        })
    }

    pub fn notification_mark_read(&self, user_id: &str, id: &str) -> io::Result<bool> {
        self.block_on(async {
            let r = self
                .col::<Document>("notifications")
                .update_one(
                    doc! { "id": id, "user_id": user_id },
                    doc! { "$set": { "read": true } },
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.matched_count > 0)
        })
    }

    pub fn notifications_mark_all_read(&self, user_id: &str) -> io::Result<()> {
        self.block_on(async {
            self.col::<Document>("notifications")
                .update_many(doc! { "user_id": user_id }, doc! { "$set": { "read": true } })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })
    }

    pub fn notifications_clear(&self, user_id: &str) -> io::Result<()> {
        self.block_on(async {
            self.col::<Document>("notifications")
                .delete_many(doc! { "user_id": user_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })
    }

    pub fn notification_delete(&self, user_id: &str, id: &str) -> io::Result<bool> {
        self.block_on(async {
            let r = self
                .col::<Document>("notifications")
                .delete_one(doc! { "id": id, "user_id": user_id })
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r.deleted_count > 0)
        })
    }

    // ── Safari ──────────────────────────────────────────────────────────────

    pub fn safari_get(&self, user_id: &str) -> SafariState {
        self.block_on(async {
            #[derive(Deserialize)]
            struct Doc {
                #[serde(flatten)]
                state: SafariState,
            }
            self.col::<Doc>("safari")
                .find_one(doc! { "user_id": user_id })
                .await
                .ok()
                .flatten()
                .map(|d| d.state)
                .unwrap_or_default()
        })
    }

    pub fn safari_save(&self, user_id: &str, s: &SafariState) -> io::Result<()> {
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(s)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            doc.insert("user_id", user_id);
            doc.insert("_id", user_id);
            self.col::<Document>("safari")
                .replace_one(doc! { "user_id": user_id }, doc)
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok::<(), io::Error>(())
        })
    }

    pub fn safari_add_history(&self, user_id: &str, title: &str, url: &str) -> io::Result<()> {
        let mut s = self.safari_get(user_id);
        s.history.retain(|h| h.url != url);
        s.history.insert(
            0,
            SafariHistoryEntry {
                id: Uuid::new_v4().to_string(),
                title: title.into(),
                url: url.into(),
                visited_at: now_str(),
                ts: Local::now().timestamp(),
            },
        );
        if s.history.len() > 200 {
            s.history.truncate(200);
        }
        self.safari_save(user_id, &s)
    }

    // ── Files (Mongo source of truth + disk mirror) ──────────────────────────

    fn user_disk(&self, user_id: &str) -> PathBuf {
        self.disk_root.join("cache").join(user_id).join("fs")
    }

    fn materialize_user_disk(&self, user_id: &str) {
        let files: Vec<FileDoc> = self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<FileDoc>("files")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(f)) = c.next().await {
                out.push(f);
            }
            out
        });
        let root = self.user_disk(user_id);
        let _ = fs::create_dir_all(&root);
        for f in files {
            let rel = f.path.trim_start_matches("~/").trim_start_matches('/');
            let path = root.join(rel);
            if f.kind == "dir" {
                let _ = fs::create_dir_all(&path);
            } else {
                if let Some(p) = path.parent() {
                    let _ = fs::create_dir_all(p);
                }
                if f.encoding == "base64" {
                    if let Ok(bytes) = base64_decode(&f.content) {
                        let _ = fs::write(&path, bytes);
                    }
                } else {
                    let _ = fs::write(&path, &f.content);
                }
            }
        }
    }

    pub fn resolve_virtual(&self, user_id: &str, virt: &str) -> Result<PathBuf, String> {
        // Materialize then resolve on disk for terminal
        self.materialize_user_disk(user_id);
        let fs_root = self.user_disk(user_id);
        let cleaned = virt
            .trim()
            .trim_start_matches("~/")
            .trim_start_matches('~')
            .trim_start_matches("/Users/maxcos/")
            .trim_start_matches('/');
        let mut out = fs_root.clone();
        if !cleaned.is_empty() {
            for comp in Path::new(cleaned).components() {
                match comp {
                    Component::Normal(s) => out.push(s),
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if out == fs_root {
                            return Err("path escapes sandbox".into());
                        }
                        out.pop();
                    }
                    _ => return Err("invalid path".into()),
                }
            }
        }
        let root_canon = fs_root.canonicalize().unwrap_or(fs_root);
        if let Ok(c) = out.canonicalize() {
            if !c.starts_with(&root_canon) {
                return Err("path escapes sandbox".into());
            }
            return Ok(c);
        }
        Ok(out)
    }

    pub fn to_virtual(&self, user_id: &str, real: &Path) -> String {
        let fs_root = self.user_disk(user_id);
        let root = fs_root.canonicalize().unwrap_or_else(|_| fs_root.clone());
        let real_c = real.canonicalize().unwrap_or_else(|_| real.to_path_buf());
        if let Ok(rel) = real_c.strip_prefix(&root) {
            if rel.as_os_str().is_empty() {
                return "~".into();
            }
            format!("~/{}", rel.to_string_lossy().replace('\\', "/"))
        } else {
            "~".into()
        }
    }

    fn normalize_virt(virt: &str) -> String {
        let cleaned = virt
            .trim()
            .trim_start_matches("~/")
            .trim_start_matches('~')
            .trim_start_matches("/Users/maxcos/")
            .trim_start_matches('/');
        if cleaned.is_empty() {
            "~".into()
        } else {
            format!("~/{cleaned}")
        }
    }

    pub fn fs_list(&self, user_id: &str, virt: &str) -> Result<Vec<FsEntry>, String> {
        let parent = Self::normalize_virt(virt);
        let prefix = if parent == "~" {
            "~/".to_string()
        } else {
            format!("{parent}/")
        };
        let files: Vec<FileDoc> = self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<FileDoc>("files")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(e) => return Err(e.to_string()),
            };
            let mut out = Vec::new();
            while let Some(Ok(f)) = c.next().await {
                out.push(f);
            }
            Ok(out)
        })?;
        let mut entries = Vec::new();
        for f in files {
            if parent == "~" {
                // top-level under ~
                let rest = f.path.trim_start_matches("~/");
                if rest.is_empty() || rest.contains('/') {
                    // only direct children
                    if !rest.contains('/') && !rest.is_empty() {
                        entries.push(FsEntry {
                            name: f.name,
                            path: f.path,
                            kind: f.kind,
                            size: f.size,
                            modified: f.modified,
                            ext: f.ext,
                        });
                    }
                }
            } else if f.path.starts_with(&prefix) {
                let rest = &f.path[prefix.len()..];
                if !rest.is_empty() && !rest.contains('/') {
                    entries.push(FsEntry {
                        name: f.name,
                        path: f.path,
                        kind: f.kind,
                        size: f.size,
                        modified: f.modified,
                        ext: f.ext,
                    });
                }
            }
        }
        entries.sort_by(|a, b| match (a.kind.as_str(), b.kind.as_str()) {
            ("dir", "file") => std::cmp::Ordering::Less,
            ("file", "dir") => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(entries)
    }

    pub fn fs_read(&self, user_id: &str, virt: &str) -> Result<(String, String), String> {
        let path = Self::normalize_virt(virt);
        let f: Option<FileDoc> = self.block_on(async {
            self.col::<FileDoc>("files")
                .find_one(doc! { "user_id": user_id, "path": &path })
                .await
                .map_err(|e| e.to_string())
        })?;
        let f = f.ok_or_else(|| "not a file".to_string())?;
        if f.kind != "file" {
            return Err("not a file".into());
        }
        Ok((f.path, f.content))
    }

    pub fn fs_write(&self, user_id: &str, virt: &str, content: &str) -> Result<FsEntry, String> {
        let path = Self::normalize_virt(virt);
        let name = path.rsplit('/').next().unwrap_or("file").to_string();
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        let f = FileDoc {
            user_id: user_id.into(),
            path: path.clone(),
            name: name.clone(),
            kind: "file".into(),
            ext: ext.clone(),
            size: content.len() as u64,
            content: content.into(),
            encoding: "utf-8".into(),
            modified: now_str(),
        };
        self.block_on(async {
            let mut doc = mongodb::bson::to_document(&f).map_err(|e| e.to_string())?;
            // use path+user as unique key
            self.col::<Document>("files")
                .replace_one(doc! { "user_id": user_id, "path": &path }, doc.clone())
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })?;
        // mirror disk
        if let Ok(real) = self.resolve_virtual(user_id, &path) {
            if let Some(p) = real.parent() {
                let _ = fs::create_dir_all(p);
            }
            let _ = fs::write(&real, content);
        }
        Ok(FsEntry {
            name,
            path,
            kind: "file".into(),
            size: content.len() as u64,
            modified: now_str(),
            ext,
        })
    }

    pub fn fs_mkdir(&self, user_id: &str, virt: &str) -> Result<FsEntry, String> {
        let path = Self::normalize_virt(virt);
        let name = path.rsplit('/').next().unwrap_or("folder").to_string();
        let f = FileDoc {
            user_id: user_id.into(),
            path: path.clone(),
            name: name.clone(),
            kind: "dir".into(),
            ext: String::new(),
            size: 0,
            content: String::new(),
            encoding: "utf-8".into(),
            modified: now_str(),
        };
        self.block_on(async {
            self.col::<Document>("files")
                .replace_one(
                    doc! { "user_id": user_id, "path": &path },
                    mongodb::bson::to_document(&f).map_err(|e| e.to_string())?,
                )
                .with_options(
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })?;
        if let Ok(real) = self.resolve_virtual(user_id, &path) {
            let _ = fs::create_dir_all(real);
        }
        Ok(FsEntry {
            name,
            path,
            kind: "dir".into(),
            size: 0,
            modified: now_str(),
            ext: String::new(),
        })
    }

    pub fn fs_create_file(
        &self,
        user_id: &str,
        virt: &str,
        content: &str,
    ) -> Result<FsEntry, String> {
        let path = Self::normalize_virt(virt);
        let exists = self.block_on(async {
            self.col::<Document>("files")
                .find_one(doc! { "user_id": user_id, "path": &path })
                .await
                .ok()
                .flatten()
                .is_some()
        });
        if exists {
            return Err("already exists".into());
        }
        self.fs_write(user_id, virt, content)
    }

    pub fn fs_delete(&self, user_id: &str, virt: &str) -> Result<(), String> {
        let path = Self::normalize_virt(virt);
        if path == "~" {
            return Err("cannot delete home".into());
        }
        self.block_on(async {
            // delete path and children
            let prefix = format!("{path}/");
            self.col::<Document>("files")
                .delete_many(doc! {
                    "user_id": user_id,
                    "$or": [
                        { "path": &path },
                        { "path": { "$regex": format!("^{}", regex_escape(&prefix)) } }
                    ]
                })
                .await
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        })?;
        if let Ok(real) = self.resolve_virtual(user_id, &path) {
            if real.is_dir() {
                let _ = fs::remove_dir_all(real);
            } else {
                let _ = fs::remove_file(real);
            }
        }
        Ok(())
    }

    pub fn fs_rename(&self, user_id: &str, from: &str, to: &str) -> Result<FsEntry, String> {
        let content = match self.fs_read(user_id, from) {
            Ok((_, c)) => c,
            Err(_) => {
                // dir rename: list not full recursive for simplicity
                return Err("rename only supports files currently".into());
            }
        };
        let entry = self.fs_write(user_id, to, &content)?;
        let _ = self.fs_delete(user_id, from);
        Ok(entry)
    }

    pub fn search(&self, user_id: &str, query: &str, limit: usize) -> Vec<SearchHit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let mut hits = Vec::new();
        for app in apps::all_apps() {
            if app.id == "trash" {
                continue;
            }
            if app.name.to_lowercase().contains(&q) || app.id.contains(&q) {
                hits.push(SearchHit {
                    kind: "app".into(),
                    id: app.id.into(),
                    title: app.name.into(),
                    subtitle: "Application".into(),
                    path: None,
                    app_id: Some(app.id.into()),
                    score: if app.name.to_lowercase().starts_with(&q) {
                        100
                    } else {
                        80
                    },
                });
            }
        }
        for note in self.notes_list(user_id) {
            if note.title.to_lowercase().contains(&q) || note.body.to_lowercase().contains(&q) {
                hits.push(SearchHit {
                    kind: "note".into(),
                    id: note.id,
                    title: note.title,
                    subtitle: format!(
                        "Notes — {}",
                        note.body.chars().take(60).collect::<String>()
                    ),
                    path: None,
                    app_id: Some("notes".into()),
                    score: 90,
                });
            }
        }
        for rem in self.reminders_list(user_id) {
            if rem.text.to_lowercase().contains(&q) {
                hits.push(SearchHit {
                    kind: "reminder".into(),
                    id: rem.id,
                    title: rem.text,
                    subtitle: "Reminders".into(),
                    path: None,
                    app_id: Some("reminders".into()),
                    score: 70,
                });
            }
        }
        // files from mongo
        let files: Vec<FileDoc> = self.block_on(async {
            use futures_util::StreamExt;
            let mut c = match self
                .col::<FileDoc>("files")
                .find(doc! { "user_id": user_id })
                .await
            {
                Ok(c) => c,
                Err(_) => return vec![],
            };
            let mut out = Vec::new();
            while let Some(Ok(f)) = c.next().await {
                out.push(f);
            }
            out
        });
        for f in files {
            let name_l = f.name.to_lowercase();
            if name_l.contains(&q)
                || (f.kind == "file" && f.content.to_lowercase().contains(&q))
            {
                hits.push(SearchHit {
                    kind: "file".into(),
                    id: f.path.clone(),
                    title: f.name,
                    subtitle: format!("{} — {}", if f.kind == "dir" { "Folder" } else { "Document" }, f.path),
                    path: Some(f.path),
                    app_id: Some(if f.kind == "dir" {
                        "finder".into()
                    } else {
                        "textedit".into()
                    }),
                    score: if name_l.starts_with(&q) { 95 } else { 55 },
                });
            }
        }
        hits.sort_by(|a, b| b.score.cmp(&a.score));
        hits.truncate(limit);
        hits
    }
}

fn now_str() -> String {
    Local::now().format("%b %-d, %Y %-I:%M %p").to_string()
}
fn sanitize(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "Untitled".into()
    } else {
        s
    }
}
fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ".+*?^$()[]{}|\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
fn base64_decode(s: &str) -> Result<Vec<u8>, ()> {
    // minimal base64
    use std::collections::HashMap;
    // use data_encoding if available - simple via std not available
    // fallback empty
    let _ = s;
    Err(())
}
