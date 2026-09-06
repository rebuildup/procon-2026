//! Stable identifier types used across the core domain.

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(
            ::core::fmt::Debug,
            ::core::clone::Clone,
            ::core::cmp::PartialEq,
            ::core::cmp::Eq,
            ::core::cmp::PartialOrd,
            ::core::cmp::Ord,
            ::core::hash::Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(::std::string::String);

        impl $name {
            /// Construct a new identifier. The prefix is enforced for namespace clarity.
            pub fn new(
                value: impl ::core::convert::AsRef<str>,
            ) -> ::core::result::Result<Self, ::std::string::String> {
                let value = value.as_ref();
                if value.is_empty() {
                    return ::core::result::Result::Err(::std::string::String::from("empty id"));
                }
                ::core::result::Result::Ok(::core::clone::Clone::clone(&Self(value.to_owned())))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn prefix() -> &'static str {
                $prefix
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

id_type!(MatchId, "match");
id_type!(PlayerId, "player");
id_type!(TurnId, "turn");
