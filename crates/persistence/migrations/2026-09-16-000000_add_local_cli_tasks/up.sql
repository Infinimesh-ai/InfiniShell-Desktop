CREATE TABLE local_cli_tasks (
    task_id TEXT PRIMARY KEY NOT NULL,
    parent_task_id TEXT,
    harness TEXT NOT NULL,
    native_session_id TEXT,
    generation BIGINT NOT NULL CHECK (generation > 0),
    revision BIGINT NOT NULL CHECK (revision >= 0),
    state TEXT NOT NULL,
    data TEXT NOT NULL
);

CREATE INDEX local_cli_tasks_parent ON local_cli_tasks(parent_task_id);
CREATE UNIQUE INDEX local_cli_tasks_active_session
    ON local_cli_tasks(harness, native_session_id)
    WHERE native_session_id IS NOT NULL
      AND state IN ('queued', 'running', 'waiting_for_user');

CREATE TABLE local_cli_task_generations (
    task_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    data TEXT NOT NULL,
    PRIMARY KEY (task_id, generation),
    FOREIGN KEY (task_id) REFERENCES local_cli_tasks(task_id)
);

CREATE TABLE local_cli_messages (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    message_id TEXT NOT NULL UNIQUE,
    sender_task_id TEXT NOT NULL,
    recipient_task_id TEXT NOT NULL,
    sender_generation BIGINT NOT NULL CHECK (sender_generation > 0),
    recipient_generation BIGINT NOT NULL CHECK (recipient_generation > 0),
    state TEXT NOT NULL,
    data TEXT NOT NULL,
    FOREIGN KEY (sender_task_id) REFERENCES local_cli_tasks(task_id),
    FOREIGN KEY (recipient_task_id) REFERENCES local_cli_tasks(task_id)
);

CREATE INDEX local_cli_messages_recipient
    ON local_cli_messages(recipient_task_id, recipient_generation, sequence);
