//! Wikidata entity id newtypes, so a property can never be passed as an item.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

macro_rules! entity_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }

        /// Accepts `Q42`, `q42` and plain `42`.
        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let s = s.trim();
                let digits = s
                    .strip_prefix($prefix)
                    .or_else(|| s.strip_prefix(&$prefix.to_lowercase()))
                    .unwrap_or(s);
                match digits.parse::<u64>() {
                    Ok(n) if n > 0 => Ok(Self(n)),
                    _ => Err(format!(concat!("not a valid ", $prefix, "-id: {}"), s)),
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

entity_id!(ItemId, "Q");
entity_id!(PropertyId, "P");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        assert_eq!("Q42".parse::<ItemId>().unwrap().to_string(), "Q42");
        assert_eq!("p31".parse::<PropertyId>().unwrap(), PropertyId(31));
        assert_eq!("345".parse::<PropertyId>().unwrap(), PropertyId(345));
        assert!("P31".parse::<ItemId>().is_err());
        assert!("Q0".parse::<ItemId>().is_err());
        assert!("Q-1".parse::<ItemId>().is_err());
    }

    #[test]
    fn serde_as_strings() {
        let json = serde_json::to_string(&ItemId(5)).unwrap();
        assert_eq!(json, "\"Q5\"");
        assert_eq!(serde_json::from_str::<ItemId>(&json).unwrap(), ItemId(5));
    }
}
