ALTER TABLE bindings ADD COLUMN workspace_id TEXT;
ALTER TABLE bindings ADD COLUMN display_name TEXT;
ALTER TABLE bindings ADD COLUMN status_message_id INTEGER;
ALTER TABLE bindings ADD COLUMN status_text TEXT;
ALTER TABLE bindings ADD COLUMN observed_items TEXT NOT NULL DEFAULT '{}';
ALTER TABLE bindings ADD COLUMN history_ready INTEGER NOT NULL DEFAULT 0;
ALTER TABLE operations ADD COLUMN payload TEXT;
ALTER TABLE operations ADD COLUMN receipt TEXT;
ALTER TABLE deliveries ADD COLUMN not_before INTEGER NOT NULL DEFAULT 0;
ALTER TABLE polling_cursors ADD COLUMN last_poll_at INTEGER NOT NULL DEFAULT 0;

CREATE TABLE callbacks (
    token TEXT PRIMARY KEY,
    bot_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    config_hash TEXT NOT NULL,
    payload TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE topic_requests (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('dispatching','accepted','rejected','outcome_unknown')),
    topic_id INTEGER,
    PRIMARY KEY(bot_id, chat_id, instance_id, thread_id)
);

CREATE INDEX deliveries_pending ON deliveries(bot_id, state, not_before);
CREATE INDEX inbox_pending ON inbound_updates(bot_id, processed, received_at);
