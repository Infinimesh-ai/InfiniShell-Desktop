use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::auth::UserUid;
use crate::cloud_object::Owner;
use crate::ids::ServerId;

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SharingAccessLevel {
    View,
    Edit,
    Full,
}

impl SharingAccessLevel {
    pub fn label(&self) -> &'static str {
        match self {
            SharingAccessLevel::View => "Can view",
            SharingAccessLevel::Edit => "Can edit",
            SharingAccessLevel::Full => "Full access",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            SharingAccessLevel::View => "view",
            SharingAccessLevel::Edit => "edit",
            SharingAccessLevel::Full => "access",
        }
    }

    /// Whether or not this access level implies the `Trash` action.
    pub fn can_trash(self) -> bool {
        self >= SharingAccessLevel::Edit
    }

    /// Whether or not this access level implies the `DeletePermanently` action.
    pub fn can_delete(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Whether or not this access level implies the `ChangeOwner` action.
    pub fn can_move_drive(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Whether or not this access level implies the `EditAccess` action.
    pub fn can_edit_access(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Convert this access level to a serializable value, which can be parsed by [`FromStr`].
    pub fn to_serializable_value(self) -> &'static str {
        match self {
            SharingAccessLevel::View => "VIEW",
            SharingAccessLevel::Edit => "EDIT",
            SharingAccessLevel::Full => "FULL",
        }
    }
}

impl FromStr for SharingAccessLevel {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "VIEW" => Ok(Self::View),
            "EDIT" => Ok(Self::Edit),
            "FULL" => Ok(Self::Full),
            _ => Err(anyhow::anyhow!("unknown access level {value}")),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkSharingSubjectType {
    None,
    Anyone,
}

/// A `Subject` is someone with access to a shared object, like its owner or a directly-added
/// guest.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    User(UserKind),
    #[allow(dead_code)]
    PendingUser {
        email: Option<String>,
    },
    Team(TeamKind),
    AnyoneWithLink(LinkSharingSubjectType),
}

/// A kind of user. Zap 剥离了 session sharing 协议,因此只保留本地账户一种形态。
#[derive(Debug, Clone)]
pub enum UserKind {
    /// A user account, tracked in the app-side `UserProfiles` model.
    Account(UserUid),
}

/// A kind of team.
#[derive(Debug, Clone, PartialEq)]
pub enum TeamKind {
    Team { team_uid: ServerId },
}

impl TeamKind {
    /// Gets the team UID.
    pub fn team_uid(&self) -> ServerId {
        match self {
            TeamKind::Team { team_uid } => *team_uid,
        }
    }
}

impl Subject {
    /// Convert an [`Owner`] into the closest [`Subject`] type.
    pub fn from_owner(owner: Owner) -> Self {
        match owner {
            Owner::User { user_uid } => Subject::User(UserKind::Account(user_uid)),
            Owner::Team { team_uid } => Subject::Team(TeamKind::Team { team_uid }),
        }
    }

    /// Gets the user UID for this subject, if it has one.
    pub fn user_uid(&self) -> Option<UserUid> {
        match self {
            Subject::User(user_kind) => match user_kind {
                UserKind::Account(user_uid) => Some(*user_uid),
            },
            Subject::PendingUser { .. } | Subject::Team(_) | Subject::AnyoneWithLink(_) => None,
        }
    }

    /// Checks if this subject refers to a given user directly.
    pub fn is_user(&self, other_uid: UserUid) -> bool {
        match self {
            Subject::User(UserKind::Account(user_uid)) => *user_uid == other_uid,
            Subject::PendingUser { .. } | Subject::Team(_) | Subject::AnyoneWithLink(_) => false,
        }
    }

    /// Gets the team UID for this subject, if it has one.
    pub fn team_uid(&self) -> Option<ServerId> {
        match self {
            Subject::Team(team_kind) => Some(team_kind.team_uid()),
            Subject::User(_) | Subject::PendingUser { .. } | Subject::AnyoneWithLink(_) => None,
        }
    }
}

impl PartialEq for UserKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Account(self_uid), Self::Account(other_uid)) => self_uid == other_uid,
        }
    }
}
