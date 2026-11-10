//! Per-pane agent access, shown like Unix permissions.
//!
//! Each pane carries three bits that say what socket API callers (agents in other
//! panes, scripts, integrations) may do with it:
//!
//! - `r`: read the screen and see the pane's full metadata.
//! - `w`: deliver raw input (text and keys).
//! - `x`: execute through a structured operation (`pane.run`, `agent.prompt`,
//!   `agent.start`, link activation).
//!
//! `rwx` is the default and means every operation is allowed, `---` means agents only
//! learn that the pane exists. Layout and lifecycle operations never depend on the mode.
//! The Herdr UI may set any mode; API callers may only remove bits, and never from a
//! pane the user set explicitly. See `App::pane_access_refusal`.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

const READ: u8 = 0b100;
const WRITE: u8 = 0b010;
const EXEC: u8 = 0b001;

/// The eight modes in bit order, so `NAMES[bits]` is the display form.
const NAMES: [&str; 8] = ["---", "--x", "-w-", "-wx", "r--", "r-x", "rw-", "rwx"];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PaneAccess(u8);

impl PaneAccess {
    pub const FULL: Self = Self(READ | WRITE | EXEC);
    pub const NONE: Self = Self(0);
    pub const READ_ONLY: Self = Self(READ);
    pub const PROMPT_ONLY: Self = Self(READ | EXEC);

    pub fn can(self, bit: PaneAccessBit) -> bool {
        self.0 & bit.mask() != 0
    }

    pub fn can_read(self) -> bool {
        self.can(PaneAccessBit::Read)
    }

    pub fn can_write(self) -> bool {
        self.can(PaneAccessBit::Write)
    }

    pub fn can_exec(self) -> bool {
        self.can(PaneAccessBit::Exec)
    }

    /// For `skip_serializing_if`: the default mode needs no record.
    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// True when `self` grants nothing that `other` does not: the tightening check.
    pub fn is_subset_of(self, other: Self) -> bool {
        self.0 & !other.0 == 0
    }

    /// The three-character form, from a static table: no allocation.
    pub fn as_str(self) -> &'static str {
        NAMES[usize::from(self.0 & 0b111)]
    }

    /// The keybinding cycle: full -> watch only -> private -> full. Any other mode
    /// returns to full first.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::FULL => Self::READ_ONLY,
            Self::READ_ONLY => Self::NONE,
            _ => Self::FULL,
        }
    }
}

impl Default for PaneAccess {
    fn default() -> Self {
        Self::FULL
    }
}

impl fmt::Display for PaneAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PaneAccess {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        let mut bits = 0;
        if bytes.len() == 3 {
            for (byte, (letter, mask)) in
                bytes
                    .iter()
                    .zip([(b'r', READ), (b'w', WRITE), (b'x', EXEC)])
            {
                match *byte {
                    b'-' => {}
                    b if b == letter => bits |= mask,
                    _ => {
                        return Err(format!(
                            "{s:?} is not an access mode; expected a mode like rwx, r--, or ---"
                        ))
                    }
                }
            }
            return Ok(Self(bits));
        }
        Err(format!(
            "{s:?} is not an access mode; expected a mode like rwx, r--, or ---"
        ))
    }
}

impl TryFrom<String> for PaneAccess {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<PaneAccess> for String {
    fn from(value: PaneAccess) -> Self {
        value.as_str().to_string()
    }
}

impl Serialize for PaneAccess {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PaneAccess {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for PaneAccess {
    fn schema_name() -> Cow<'static, str> {
        "PaneAccess".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "enum": NAMES,
            "description": "Agent access to a pane like Unix permissions: r read the screen and metadata, w send raw input, x run commands or prompt the agent. rwx allows everything, --- hides the pane."
        })
    }
}

/// One of the three permissions, for refusal messages and gates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneAccessBit {
    Read,
    Write,
    Exec,
}

impl PaneAccessBit {
    fn mask(self) -> u8 {
        match self {
            Self::Read => READ,
            Self::Write => WRITE,
            Self::Exec => EXEC,
        }
    }

    pub fn letter(self) -> char {
        match self {
            Self::Read => 'r',
            Self::Write => 'w',
            Self::Exec => 'x',
        }
    }

    /// What the bit permits, for messages such as "reading needs r".
    pub fn verb(self) -> &'static str {
        match self {
            Self::Read => "reading",
            Self::Write => "input",
            Self::Exec => "execution",
        }
    }
}

/// Who chose a pane's access mode. A mode the user chose from the Herdr UI is not
/// changed by programs or agents.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneAccessSource {
    #[default]
    Default,
    Program,
    User,
}

impl PaneAccessSource {
    pub fn is_default(&self) -> bool {
        *self == Self::Default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_round_trips_through_its_string_form() {
        for (bits, name) in NAMES.iter().enumerate() {
            let mode: PaneAccess = name.parse().unwrap();
            assert_eq!(mode, PaneAccess(bits as u8));
            assert_eq!(mode.as_str(), *name);
            assert_eq!(mode.to_string(), *name);
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<PaneAccess>(&json).unwrap(), mode);
        }
    }

    #[test]
    fn malformed_modes_are_rejected() {
        for bad in ["rw", "rwxx", "rwX", "xwr", "rwx ", "", "r w", "RWX"] {
            assert!(
                bad.parse::<PaneAccess>().is_err(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn bits_and_subsets() {
        let full = PaneAccess::FULL;
        let watch = PaneAccess::READ_ONLY;
        let prompt = PaneAccess::PROMPT_ONLY;
        let none = PaneAccess::NONE;
        assert!(full.can_read() && full.can_write() && full.can_exec());
        assert!(watch.can_read() && !watch.can_write() && !watch.can_exec());
        assert!(prompt.can_read() && !prompt.can_write() && prompt.can_exec());
        assert!(!none.can_read() && !none.can_write() && !none.can_exec());
        assert!(watch.is_subset_of(full));
        assert!(none.is_subset_of(watch));
        assert!(!full.is_subset_of(watch));
        assert!(!prompt.is_subset_of(watch), "r-x adds x over r--");
        assert!(
            full.is_subset_of(full),
            "equal modes are subsets of each other"
        );
        assert!(PaneAccess::default().is_full());
    }

    #[test]
    fn cycle_visits_full_watch_and_private() {
        assert_eq!(PaneAccess::FULL.cycle_next(), PaneAccess::READ_ONLY);
        assert_eq!(PaneAccess::READ_ONLY.cycle_next(), PaneAccess::NONE);
        assert_eq!(PaneAccess::NONE.cycle_next(), PaneAccess::FULL);
        assert_eq!(PaneAccess::PROMPT_ONLY.cycle_next(), PaneAccess::FULL);
    }

    #[test]
    fn schema_is_a_constrained_string() {
        let schema = schemars::schema_for!(PaneAccess);
        let value = serde_json::to_value(schema).unwrap();
        assert_eq!(value["type"], "string");
        assert_eq!(value["enum"].as_array().map(Vec::len), Some(8));
        assert_eq!(value["enum"][7], "rwx");
    }

    #[test]
    fn source_defaults_to_default_and_serializes_as_snake_case() {
        assert!(PaneAccessSource::default().is_default());
        assert_eq!(
            serde_json::to_string(&PaneAccessSource::User).unwrap(),
            "\"user\""
        );
    }
}
