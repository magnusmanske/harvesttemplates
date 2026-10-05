use super::oauth::Token;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

/// The freshest token per user (by central id). Refresh tokens rotate, so once
/// a worker refreshes, the copy in the user's session is stale; this keeps the
/// current one for the lifetime of the process.
#[derive(Debug, Default)]
pub struct TokenCache(Mutex<HashMap<u64, Token>>);

impl TokenCache {
    /// Remember `token` unless a fresher one is known; return the freshest.
    pub fn freshest(&self, user: u64, token: Token) -> Token {
        let mut map = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = map.entry(user).or_insert_with(|| token.clone());
        if token.expires_at > entry.expires_at {
            *entry = token;
        }
        entry.clone()
    }

    pub fn get(&self, user: u64) -> Option<Token> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&user)
            .cloned()
    }

    pub fn replace(&self, user: u64, token: Token) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user, token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Secret;

    fn token(access: &str, expires_at: i64) -> Token {
        Token {
            access: Secret::from(access),
            refresh: None,
            expires_at,
        }
    }

    #[test]
    fn keeps_the_freshest() {
        let cache = TokenCache::default();
        assert_eq!(cache.freshest(1, token("a", 100)).access.expose(), "a");
        assert_eq!(cache.freshest(1, token("old", 50)).access.expose(), "a");
        assert_eq!(cache.freshest(1, token("b", 200)).access.expose(), "b");
        assert_eq!(cache.get(2), None);
    }
}
