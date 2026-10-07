-- Migration to support PostgreSQL LISTEN/NOTIFY for task queue changes and recording session dispatch

CREATE OR REPLACE FUNCTION notify_task_queue_changed()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('task_queue_changed', '');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_notify_task_runs_queued ON task_runs;
CREATE TRIGGER trg_notify_task_runs_queued
AFTER INSERT OR UPDATE OF status ON task_runs
FOR EACH ROW
WHEN (NEW.status = 'queued')
EXECUTE FUNCTION notify_task_queue_changed();

DROP TRIGGER IF EXISTS trg_notify_recording_sessions ON recording_sessions;
CREATE TRIGGER trg_notify_recording_sessions
AFTER INSERT OR UPDATE OF status ON recording_sessions
FOR EACH ROW
WHEN (NEW.status = 'recording')
EXECUTE FUNCTION notify_task_queue_changed();
