use warpui::AppContext;

use super::{GenericStringObjectFormat, JsonObjectType, ObjectType, StoredObject};
use crate::cloud_object::update_manager::{InitiatedBy, ObjectOperation, OperationSuccessType};

pub struct StoredObjectToastMessage;

impl StoredObjectToastMessage {
    pub fn toast_message(
        object: &dyn StoredObject,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
        app: &AppContext,
    ) -> Option<String> {
        let object_name = localized_object_name(object.model_type_name());
        let object_name_lowercase = object_name.to_lowercase();

        match (object.object_type(), operation, success_type) {
            // We should only show toasts for creates initiated by the user, not by the system
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => {
                let containing_object_name = object.containing_object_name(app);
                Some(crate::t!(
                    "cloud-object-toast-saved-to",
                    object = object_name,
                    container = containing_object_name
                ))
            }
            // notebooks intentionally do not have an update message, as they are updated
            // as the user types and so toasts would be VERY noisy
            (ObjectType::Notebook, ObjectOperation::Update, OperationSuccessType::Success) => None,
            (_, ObjectOperation::Update, OperationSuccessType::Success) => Some(crate::t!(
                "cloud-object-toast-updated",
                object = object_name
            )),
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Success)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Success) => {
                let containing_object_name = object.containing_object_name(app);
                Some(crate::t!(
                    "cloud-object-toast-moved-to",
                    object = object_name,
                    container = containing_object_name
                ))
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Success) => Some(crate::t!(
                "cloud-object-toast-trashed",
                object = object_name
            )),
            (_, ObjectOperation::Untrash, OperationSuccessType::Success) => Some(crate::t!(
                "cloud-object-toast-restored",
                object = object_name
            )),
            (_, ObjectOperation::Leave, OperationSuccessType::Success) => {
                Some(crate::t!("cloud-object-toast-left", object = object_name))
            }
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(crate::t!(
                "cloud-object-toast-create-failed",
                object = object_name_lowercase
            )),
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Denied(message),
            ) => Some(message.to_string()),
            (_, ObjectOperation::Update, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-update-failed",
                object = object_name_lowercase
            )),
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Failure)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-move-failed",
                object = object_name_lowercase
            )),
            (_, ObjectOperation::Trash, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-trash-failed",
                object = object_name_lowercase
            )),
            (_, ObjectOperation::Untrash, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-restore-failed",
                object = object_name_lowercase
            )),
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                _,
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(crate::t!(
                "cloud-object-toast-delete-failed",
                object = object_name_lowercase
            )),
            (_, ObjectOperation::Leave, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-leave-failed",
                object = object_name_lowercase
            )),
            (ObjectType::Workflow, ObjectOperation::Update, OperationSuccessType::Rejection) => {
                Some(crate::t!("cloud-object-toast-workflow-conflict"))
            }
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::EnvVarCollection,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(crate::t!("cloud-object-toast-env-vars-conflict")),
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::AIFact,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(crate::t!("cloud-object-toast-rule-conflict")),
            (_, ObjectOperation::TakeEditAccess, OperationSuccessType::Failure) => Some(crate::t!(
                "cloud-object-toast-start-editing-failed",
                object = object_name_lowercase
            )),
            _ => None,
        }
    }

    pub fn toast_deletion_confirm_message(
        num_objects: i32,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
    ) -> Option<String> {
        match (operation, success_type) {
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => Some(crate::t!(
                "cloud-object-toast-deleted-forever",
                count = num_objects
            )),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Success) => Some(crate::t!(
                "cloud-object-toast-trash-emptied",
                count = num_objects
            )),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Failure) => {
                Some(crate::t!("cloud-object-toast-empty-trash-failed"))
            }
            (ObjectOperation::EmptyTrash, OperationSuccessType::Rejection) => {
                Some(crate::t!("cloud-object-toast-trash-already-empty"))
            }
            _ => None,
        }
    }
}

fn localized_object_name(model_type_name: &str) -> String {
    match model_type_name {
        "Notebook" => crate::t!("drive-notebook"),
        "Plan" => crate::t!("cloud-object-type-plan"),
        "Workflow" => crate::t!("drive-workflow"),
        "Prompt" => crate::t!("drive-prompt"),
        "Folder" => crate::t!("drive-folder"),
        "Environment variables" => crate::t!("drive-environment-variables"),
        "Rule" => crate::t!("cloud-object-type-rule"),
        "MCP server" => crate::t!("drive-object-type-mcp-server"),
        "AIExecutionProfile" => crate::t!("cloud-object-type-ai-execution-profile"),
        "Preference" => crate::t!("cloud-object-type-preference"),
        "WorkflowEnum" => crate::t!("cloud-object-type-workflow-enum"),
        model_type_name => model_type_name.to_string(),
    }
}
