-- 未确认结果仍占用原生会话，不能由另一个应用任务重复关联。
DROP INDEX local_cli_tasks_active_session;
CREATE UNIQUE INDEX local_cli_tasks_active_session
    ON local_cli_tasks(harness, native_session_id)
    WHERE native_session_id IS NOT NULL
        AND state IN ('queued', 'running', 'waiting_for_user', 'unconfirmed');
