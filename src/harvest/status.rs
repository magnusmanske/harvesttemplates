use serde::{Deserialize, Serialize};

/// An enum stored and served as a fixed lower-case string.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $s:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $s)] $variant),*
        }

        impl $name {
            pub const ALL: &[Self] = &[$(Self::$variant),*];

            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $s),* }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$variant),)* _ => None }
            }
        }
    };
}

string_enum!(
    /// Lifecycle of a run.
    RunStatus {
        /// Collecting candidate pages.
        Loading => "loading",
        /// Candidates are in; nothing running.
        Ready => "ready",
        /// Checking rows without editing.
        Previewing => "previewing",
        Editing => "editing",
        /// Stopped by the user or a restart; can be resumed.
        Paused => "paused",
        Done => "done",
        Failed => "failed",
    }
);

string_enum!(
    /// State of one candidate page.
    RowStatus {
        Pending => "pending",
        /// Previewed: would be added.
        Ready => "ready",
        /// Added to Wikidata.
        Done => "done",
        /// Nothing to do: no value, already set, …
        Skipped => "skipped",
        /// The value is unusable, violates a constraint, or the edit failed.
        Error => "error",
    }
);

impl RunStatus {
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Loading | Self::Previewing | Self::Editing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_round_trip() {
        for s in RunStatus::ALL {
            assert_eq!(RunStatus::parse(s.as_str()), Some(*s));
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        for s in RowStatus::ALL {
            assert_eq!(RowStatus::parse(s.as_str()), Some(*s));
        }
        assert_eq!(RowStatus::parse("nope"), None);
    }
}
