use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    Done,
    Blocked,
}

#[allow(dead_code)] // Used in later phases
impl Status {
    /// Returns true if transitioning from `self` to `to` is valid.
    ///
    /// Valid transitions:
    /// - todo → doing, done, blocked
    /// - doing → done, blocked, todo
    /// - blocked → (none directly; unblock restores previous status)
    /// - done → (none; terminal state)
    pub fn can_transition_to(self, to: Status) -> bool {
        matches!(
            (self, to),
            (Status::Todo, Status::Doing)
                | (Status::Todo, Status::Done)
                | (Status::Todo, Status::Blocked)
                | (Status::Doing, Status::Done)
                | (Status::Doing, Status::Blocked)
                | (Status::Doing, Status::Todo)
        )
    }
}

impl std::str::FromStr for Status {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "todo" => Ok(Status::Todo),
            "doing" => Ok(Status::Doing),
            "done" => Ok(Status::Done),
            "blocked" => Ok(Status::Blocked),
            _ => Err(format!(
                "Invalid status '{}'. Valid values: todo, doing, done, blocked",
                s
            )),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Todo => write!(f, "todo"),
            Status::Doing => write!(f, "doing"),
            Status::Done => write!(f, "done"),
            Status::Blocked => write!(f, "blocked"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        assert!(Status::Todo.can_transition_to(Status::Doing));
        assert!(Status::Todo.can_transition_to(Status::Done));
        assert!(Status::Todo.can_transition_to(Status::Blocked));
        assert!(Status::Doing.can_transition_to(Status::Done));
        assert!(Status::Doing.can_transition_to(Status::Blocked));
        assert!(Status::Doing.can_transition_to(Status::Todo));
    }

    #[test]
    fn invalid_transitions() {
        // Done is terminal
        assert!(!Status::Done.can_transition_to(Status::Todo));
        assert!(!Status::Done.can_transition_to(Status::Doing));
        assert!(!Status::Done.can_transition_to(Status::Done));
        assert!(!Status::Done.can_transition_to(Status::Blocked));

        // Blocked cannot transition directly (must unblock first)
        assert!(!Status::Blocked.can_transition_to(Status::Todo));
        assert!(!Status::Blocked.can_transition_to(Status::Doing));
        assert!(!Status::Blocked.can_transition_to(Status::Done));
        assert!(!Status::Blocked.can_transition_to(Status::Blocked));

        // Self-transitions
        assert!(!Status::Todo.can_transition_to(Status::Todo));
        assert!(!Status::Doing.can_transition_to(Status::Doing));
    }

    #[test]
    fn serde_lowercase() {
        assert_eq!(serde_json::to_string(&Status::Todo).unwrap(), "\"todo\"");
        assert_eq!(serde_json::to_string(&Status::Doing).unwrap(), "\"doing\"");
        assert_eq!(serde_json::to_string(&Status::Done).unwrap(), "\"done\"");
        assert_eq!(
            serde_json::to_string(&Status::Blocked).unwrap(),
            "\"blocked\""
        );

        assert_eq!(
            serde_json::from_str::<Status>("\"todo\"").unwrap(),
            Status::Todo
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"doing\"").unwrap(),
            Status::Doing
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"done\"").unwrap(),
            Status::Done
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"blocked\"").unwrap(),
            Status::Blocked
        );
    }

    #[test]
    fn display_matches_serde() {
        assert_eq!(Status::Todo.to_string(), "todo");
        assert_eq!(Status::Doing.to_string(), "doing");
        assert_eq!(Status::Done.to_string(), "done");
        assert_eq!(Status::Blocked.to_string(), "blocked");
    }
}
