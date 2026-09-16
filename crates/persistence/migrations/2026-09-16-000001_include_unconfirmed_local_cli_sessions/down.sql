-- 存在新状态时拒绝降级，避免旧索引解除尚未确认退出的原生会话占用。
CREATE TEMP TABLE local_cli_unconfirmed_downgrade_guard (
    active_records INTEGER NOT NULL CHECK (active_records = 0)
);
INSERT INTO local_cli_unconfirmed_downgrade_guard
    SELECT COUNT(*) FROM local_cli_tasks WHERE state = 'unconfirmed';
DROP TABLE local_cli_unconfirmed_downgrade_guard;
DROP INDEX local_cli_tasks_active_session;
CREATE UNIQUE INDEX local_cli_tasks_active_session
    ON local_cli_tasks(harness, native_session_id)
    WHERE native_session_id IS NOT NULL
        AND state IN ('queued', 'running', 'waiting_for_user');
