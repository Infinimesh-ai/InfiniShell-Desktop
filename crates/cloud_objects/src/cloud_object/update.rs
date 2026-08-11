use super::RevisionAndLastEditor;

/// Result of attempting to update a cloud object.
#[derive(Debug)]
pub enum UpdateCloudObjectResult<T> {
    /// The update was successful and the object now has the specified revision.
    Success {
        revision_and_editor: RevisionAndLastEditor,
    },
    /// The update was rejected because the update was not sent from the current revision in
    /// storage. The object and revision in storage are returned.
    Rejected { object: T },
}
