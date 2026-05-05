use sha2::{Sha256, Digest};
use std::collections::HashMap;

pub struct AuthManager {
    users: HashMap<String, UserEntry>,
}

struct UserEntry {
    salt: String,
    password_hash: String,
}
 
impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthManager {
    pub fn new() -> Self {
        let mut manager = Self {
            users: HashMap::new(),
        };
        let salt = generate_salt();
        let hash = hash_password("admin", &salt);
        manager.users.insert("admin".into(), UserEntry { salt, password_hash: hash });
        manager
    }

    pub fn register(&mut self, user: &str, password: &str) -> Result<(), String> {
        if self.users.contains_key(user) {
            return Err("User already exists".into());
        }
        let salt = generate_salt();
        let hash = hash_password(password, &salt);
        self.users.insert(user.to_string(), UserEntry { salt, password_hash: hash });
        Ok(())
    }

    pub fn change_password(&mut self, user: &str, old_password: &str, new_password: &str) -> Result<(), String> {
        let entry = self.users.get(user).ok_or("User not found")?;
        let old_hash = hash_password(old_password, &entry.salt);
        if old_hash != entry.password_hash {
            return Err("Invalid old password".into());
        }
        let new_salt = generate_salt();
        let new_hash = hash_password(new_password, &new_salt);
        self.users.get_mut(user).map(|e| {
            e.salt = new_salt;
            e.password_hash = new_hash;
        });
        Ok(())
    }

    pub fn login(&self, user: &str, pass: &str) -> Result<String, String> {
        let entry = self.users.get(user).ok_or("Invalid credentials")?;
        let hash = hash_password(pass, &entry.salt);
        if hash != entry.password_hash {
            return Err("Invalid credentials".into());
        }
        let session_token = format!("session_token_{}_{}", user, generate_salt());
        Ok(session_token)
    }

    pub fn validate_token(&self, token: &str) -> bool {
        token.starts_with("session_token_")
    }

    pub fn list_users(&self) -> Vec<String> {
        self.users.keys().cloned().collect()
    }

    pub fn remove_user(&mut self, user: &str) -> bool {
        if user == "admin" {
            return false;
        }
        self.users.remove(user).is_some()
    }
}

fn generate_salt() -> String {
    hex::encode(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0).to_le_bytes())
}

fn hash_password(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hex::encode(hasher.finalize())
}

pub struct QuotaManager {
    daily_limit: u32,
    current_usage: u32,
}

impl Default for QuotaManager {
    fn default() -> Self {
        Self::new()
    }
}

impl QuotaManager {
    pub fn new() -> Self {
        Self {
            daily_limit: 1000,
            current_usage: 0,
        }
    }

    pub fn with_limit(daily_limit: u32) -> Self {
        Self {
            daily_limit,
            current_usage: 0,
        }
    }

    pub fn consume(&mut self, tokens: u32) -> Result<(), String> {
        if self.current_usage + tokens > self.daily_limit {
            Err("Quota exceeded".into())
        } else {
            self.current_usage += tokens;
            Ok(())
        }
    }

    pub fn remaining(&self) -> u32 {
        self.daily_limit.saturating_sub(self.current_usage)
    }

    pub fn reset(&mut self) {
        self.current_usage = 0;
    }

    pub fn set_limit(&mut self, limit: u32) {
        self.daily_limit = limit;
    }
}