ALTER TABLE bindings RENAME TO old_bindings;
CREATE TABLE bindings (
    bot_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    topic_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1,
    active INTEGER NOT NULL DEFAULT 1,
    workspace_id TEXT,
    display_name TEXT,
    status_message_id INTEGER,
    status_text TEXT,
    observed_items TEXT NOT NULL DEFAULT '{}',
    history_ready INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(bot_id,chat_id,topic_id)
);
INSERT INTO bindings SELECT * FROM old_bindings;
DROP TABLE old_bindings;
CREATE UNIQUE INDEX one_active_binding_per_session
ON bindings(bot_id,chat_id,instance_id,thread_id) WHERE active=1;
