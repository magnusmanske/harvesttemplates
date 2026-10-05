//! Login with MediaWiki OAuth 2 and editing as the logged-in user.
//! Tokens stay server-side: in the session file and in process memory.

pub mod edit;
pub mod oauth;
pub mod session;
pub mod store;
pub mod tokens;

pub use edit::{EditError, Editor};
pub use oauth::{OAuth, Token};
pub use session::{User, current_user, require_user};
pub use store::FileSessionStore;
pub use tokens::TokenCache;
