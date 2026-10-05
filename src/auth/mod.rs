//! Login with MediaWiki OAuth 1.0a and editing as the logged-in user.
//! Tokens stay server-side: in the session file and in a running worker's memory.

pub mod edit;
pub mod oauth;
pub mod session;
pub mod store;

pub use edit::{EditError, Editor};
pub use oauth::{OAuth, Token};
pub use session::{User, current_user, require_user};
pub use store::FileSessionStore;
